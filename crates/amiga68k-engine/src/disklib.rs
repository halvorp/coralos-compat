//! OpenLibrary of a library that lives on disk: find it in LIBS:, LoadSeg it, find its RomTag, initialise it the way
//! exec's InitResident does, then call its Open vector. The 68k parts (init routine, Open, Close) run in the GUEST
//! through guest_call.rs, so a disk library is real 68k code with a real JMP table.
//!
//! `struct Resident` (the RomTag), computed with m68k-linux-gnu-gcc -mno-align-int over AROS exec/resident.h:
//! rt_MatchWord 0 ($4AFC), rt_MatchTag 2 (points at the matchword), rt_EndSkip 6, rt_Flags 10, rt_Version 11,
//! rt_Type 12, rt_Pri 13, rt_Name 14, rt_IdString 18, rt_Init 22. RTF_AUTOINIT is bit 7 of the flags; its rt_Init
//! points at four longs: dataSize, funcTable, dataInit (an InitStruct table), initRoutine.
//!
//! Failure modes, all named: not found in the search path (a plain NULL, as on a real machine), unloadable file,
//! no RomTag, a resident that is not a library, an init routine returning NULL (the memory and segments are given
//! back). A library newer than the request but too old for it returns NULL without calling Open.
use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::format;
use crate::dos_path::file_part;
use crate::guest_call::{self, Next};
use crate::loadseg::{self, SegList};
use crate::machine::{Amiga, EXEC_BASE};
use crate::makelib::{self, LIB_VERSION};
use crate::Refusal;
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

const RTC_MATCHWORD: u16 = 0x4AFC;
const RTF_AUTOINIT: u8 = 0x80;
const NT_LIBRARY: u8 = 9;
const RT_FLAGS: u32 = 10;
const RT_TYPE: u32 = 12;
const RT_NAME: u32 = 14;
const RT_INIT: u32 = 22;
/// The vectors every library has, as negative offsets from its base.
const LVO_OPEN: u32 = 6;
const LVO_CLOSE: u32 = 12;

/// One library loaded from disk.
pub struct DiskLib {
    /// Lower-cased file part of the name it was requested by (what a later OpenLibrary matches).
    pub key: String,
    /// rt_Name.
    pub name: String,
    pub path: String,
    pub base: u32,
    pub seglist: u32,
    pub open_count: u32,
    /// Bases its Open vector returned that are not `base`: a library that gives every opener its own base (AROS rellibs such as
    /// stdc.library build one with MakeLibrary) is closed through THAT base, so CloseLibrary must recognise it.
    pub openers: Vec<u32>,
}

/// Where a library name is searched: a name with a volume is taken as given, otherwise LIBS:, then PROGDIR:Libs.
pub fn search_paths(name: &str) -> Vec<String> {
    if name.contains(':') { return alloc::vec![name.to_string()]; }
    alloc::vec![format!("LIBS:{name}"), format!("PROGDIR:Libs/{name}"), format!("PROGDIR:{name}")]
}

/// Scan the first segment's contents for a RomTag (the matchword followed by a pointer back to itself).
fn find_rom_tag<U: GuestUi>(a: &Amiga<U>, base: u32, size: u32) -> Option<u32> {
    (0..size.saturating_sub(26)).step_by(2).map(|o| base + o)
        .find(|&p| a.mem.read_u16(p) == Ok(RTC_MATCHWORD) && a.mem.read_u32(p + 2) == Ok(p))
}

fn rd<U: GuestUi>(a: &Amiga<U>, at: u32, what: &str) -> Result<u32, Refusal> { a.mem.read_u32(at).map_err(Refusal::mem(what)) }

/// The Open vector call for disk library `i`, after the version check.
fn open_next<U: GuestUi>(a: &mut Amiga<U>, i: usize, want: u32) -> Result<Next<U>, Refusal> {
    let base = a.fs.disk_libs[i].base;
    let have = a.mem.read_u16(base + LIB_VERSION).map_err(Refusal::mem("lib_Version"))? as u32;
    if have < want {
        a.log.push(format!("[exec] OpenLibrary \"{}\": version {have} is older than the {want} asked for -> NULL", a.fs.disk_libs[i].name));
        return Ok(Next::Return(0));
    }
    Ok(Next::Call {
        target: base - LVO_OPEN, d0: want, aregs: alloc::vec![(6, base)],
        then: Box::new(move |a, _, d0| {
            if d0 != 0 {
                let l = &mut a.fs.disk_libs[i];
                l.open_count += 1;
                if d0 != l.base && !l.openers.contains(&d0) { l.openers.push(d0); }
            }
            Ok(Next::Return(d0))
        }),
    })
}

/// Hand back everything a half-initialised library holds.
fn discard<U: GuestUi>(a: &mut Amiga<U>, made: &makelib::Made, seg: u32) {
    let _ = a.heap.free(made.block, made.block_size);
    let _ = loadseg::unload(a, seg);
}

fn load_new<U: GuestUi>(a: &mut Amiga<U>, name: &str, key: String, path: String, bytes: &[u8], want: u32) -> Result<Next<U>, Refusal> {
    let seg: SegList = loadseg::load_bytes(a, bytes).map_err(|(e, why)| Refusal(format!("OpenLibrary \"{name}\": LoadSeg {path} failed ({why}), IoErr {}", e.0)))?;
    let tag = find_rom_tag(a, seg.bases[0], seg.sizes[0]).ok_or_else(|| { let _ = loadseg::unload(a, seg.bptr); Refusal(format!("OpenLibrary \"{name}\": {path} has no RomTag in its first hunk")) })?;
    let flags = a.mem.read_u8(tag + RT_FLAGS).map_err(Refusal::mem("rt_Flags"))?;
    let ty = a.mem.read_u8(tag + RT_TYPE).map_err(Refusal::mem("rt_Type"))?;
    if ty != NT_LIBRARY {
        let _ = loadseg::unload(a, seg.bptr);
        return Err(Refusal(format!("OpenLibrary \"{name}\": {path} is a resident of type {ty}, not a library")));
    }
    let rt_name = a.mem.read_cstr(rd(a, tag + RT_NAME, "rt_Name")?).map_err(Refusal::mem("rt_Name"))?;
    let init = rd(a, tag + RT_INIT, "rt_Init")?;
    let record = move |a: &mut Amiga<U>, base: u32| -> usize {
        a.fs.disk_libs.push(DiskLib { key, name: rt_name, path, base, seglist: seg.bptr, open_count: 0, openers: Vec::new() });
        let _ = crate::execlists::add_tail(&mut a.mem, crate::execlists::LIB_LIST, base); // AddLibrary
        a.fs.disk_libs.len() - 1
    };
    if flags & RTF_AUTOINIT == 0 {
        // A plain init function: D0 = 0, A0 = the seglist, A6 = ExecBase; it returns the library (it builds it itself).
        let n = name.to_string();
        return Ok(Next::Call { target: init, d0: 0, aregs: alloc::vec![(0, seg.bptr), (6, EXEC_BASE)], then: Box::new(move |a, _, base| {
            if base == 0 { let _ = loadseg::unload(a, seg.bptr); return Err(Refusal(format!("OpenLibrary \"{n}\": its init function returned NULL"))); }
            let i = record(a, base);
            open_next(a, i, want)
        }) });
    }
    let (data_size, funcs, data_init, init_routine) = (rd(a, init, "init table")?, rd(a, init + 4, "init table")?, rd(a, init + 8, "init table")?, rd(a, init + 12, "init table")?);
    let made = makelib::make_library(a, funcs, data_init, data_size).inspect_err(|_| { let _ = loadseg::unload(a, seg.bptr); })?;
    if init_routine == 0 {
        let i = record(a, made.base);
        return open_next(a, i, want);
    }
    let n = name.to_string();
    Ok(Next::Call { target: init_routine, d0: made.base, aregs: alloc::vec![(0, seg.bptr), (6, EXEC_BASE)], then: Box::new(move |a, _, d0| {
        if d0 == 0 { discard(a, &made, seg.bptr); return Err(Refusal(format!("OpenLibrary \"{n}\": its init routine returned NULL"))); }
        let i = record(a, d0);
        open_next(a, i, want)
    }) })
}

/// OpenLibrary's disk path. Ok(None): no such library on disk (the caller carries on with its own table). Ok(Some):
/// handled; D0 is set now (Return) or by the guest call chain this starts.
pub fn try_open<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, name: &str, want: u32) -> Result<Option<String>, Refusal> {
    let key = name[file_part(name)..].to_ascii_lowercase();
    if let Some(i) = a.fs.disk_libs.iter().position(|l| l.key == key) {
        let next = open_next(a, i, want)?;
        guest_call::start(a, cpu, next)?;
        return Ok(Some(format!("[exec] OpenLibrary name=\"{name}\" ver={want} -> disk library {} (Open vector in the guest)", a.fs.disk_libs[i].path)));
    }
    let cwd = crate::fs::cur_dir(a);
    let found = search_paths(name).into_iter().find_map(|p| {
        let n = a.fs.vol.resolve(cwd, &p).ok()?;
        a.fs.vol.file_data(n).map(|d| (p, d.to_vec()))
    });
    let Some((path, bytes)) = found else { return Ok(None) };
    let next = load_new(a, name, key, path.clone(), &bytes, want)?;
    guest_call::start(a, cpu, next)?;
    Ok(Some(format!("[exec] OpenLibrary name=\"{name}\" ver={want} -> loading {path} from disk")))
}

/// CloseLibrary of a disk library: its Close vector runs in the guest; a segment list it returns is unloaded.
pub fn close<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, i: usize, opened: u32) -> Result<String, Refusal> {
    let l = &a.fs.disk_libs[i];
    if l.open_count == 0 { return Err(Refusal(format!("CloseLibrary: {} closed more times than opened", l.name))); }
    let (base, name) = (opened, l.name.clone()); // the Close vector of the base the program passed, with A6 = that base (exec's CloseLibrary)
    let next = Next::Call { target: base - LVO_CLOSE, d0: 0, aregs: alloc::vec![(6, base)], then: Box::new(move |a, _, d0| {
        a.fs.disk_libs[i].open_count -= 1;
        if d0 != 0 { loadseg::unload(a, d0)?; }
        Ok(Next::Return(0))
    }) };
    guest_call::start(a, cpu, next)?;
    Ok(format!("[exec] CloseLibrary {name} base={base:#x} (Close vector in the guest)"))
}

/// The disk library whose base is `base`.
pub fn index_of_base<U: GuestUi>(a: &Amiga<U>, base: u32) -> Option<usize> { a.fs.disk_libs.iter().position(|l| l.base == base || l.openers.contains(&base)) }
