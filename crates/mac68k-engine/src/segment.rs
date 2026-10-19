//! The Segment Loader: CODE 0 (the A5 world), CODE n segments, the jump table and `_LoadSeg`.
//!
//! Facts (Inside Macintosh II "Segment Loader", cross-read against metamage_1 `68k/modules/ams-seg/Segments.cc`):
//!
//! * `CODE 0` data: four longs `aboveA5` (bytes above A5 incl. the 32-byte application parameters and the jump table),
//!   `belowA5` (bytes of application globals below A5), `jtSize` and `jtOffset` (the jump table's offset from A5,
//!   normally 32), then the `jtSize` bytes of the jump table, which are copied to `A5 + jtOffset`.
//! * The A5 world is one block: `belowA5` bytes of globals, then A5, then `aboveA5` bytes. A5 itself addresses the
//!   first of the application parameters; its first long is where `InitGraf` stores the QuickDraw `thePort` pointer.
//! * A jump-table entry is 8 bytes. Unloaded: `u16 offset, move.w #seg,-(sp) ($3F3C seg), _LoadSeg ($A9F0)`; loaded:
//!   `u16 seg, jmp abs.l ($4EF9 addr)`. Callers `jsr` to the entry's address + 2, which is the `$3F3C` word before the
//!   segment is loaded and the `$4EF9` after.
//! * `CODE n` data: `u16 firstEntryOffset` (of this segment's first entry in the jump table, from its start),
//!   `u16 entryCount`, then the code. An entry's `offset` is from the first byte after those four header bytes.
//! * The program starts by running the first entry (entry 0), which is the main routine in `CODE 1`.
//!
//! `_LoadSeg` here resumes the guest at the patched `jmp` (Cpu::set_pc), where a real one rewinds its return address.
//! `_UnloadSeg(routine)` takes the address a caller `jsr`s to (the entry's `+2` word): the entry names its segment, every
//! entry of that segment goes back to `offset, move.w #seg,-(sp), _LoadSeg` (the offset is recovered from the `jmp` target)
//! and the segment's handle is unlocked and made purgeable (Inside Macintosh: Processes, "Segment Loader"). The code is
//! not freed here (this Memory Manager never purges): the program may `ReleaseResource` it, and the next `_LoadSeg`
//! reloads it. An entry that is already unloaded is a no-op, as on a real Mac.
//! A `CODE` segment whose first word is `$FFFF` is the 32-bit "far model" header; it is refused by name.
use crate::lowmem;
use crate::machine::{Call, Done, Mac, EXIT_STUB, STACK_TOP};
use crate::pascal::Arg;
use crate::resmgr::APP_REF_NUM;
use crate::Refusal;
use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::{String, ToString};
use guestui_abi::GuestUi;
use retro_m68k::{Cpu, REG_A0};
use retro_resfork::ResFork;

const APP_PARAMS: u32 = 32;
const MAX_WORLD: u32 = 8 << 20;

#[derive(Default)]
pub struct SegLoader {
    pub a5: u32,
    pub jt_offset: u32,
    pub jt_size: u32,
    pub loaded: BTreeSet<u16>,
}

/// The `CODE 0` header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Code0 { pub above: u32, pub below: u32, pub jt_size: u32, pub jt_offset: u32 }

pub fn parse_code0(d: &[u8]) -> Result<(Code0, &[u8]), String> {
    let long = |i: usize| d.get(i..i + 4).map(|b| u32::from_be_bytes(b.try_into().unwrap())).ok_or_else(|| "CODE 0 is shorter than its 16-byte header".to_string());
    let c = Code0 { above: long(0)?, below: long(4)?, jt_size: long(8)?, jt_offset: long(12)? };
    if c.jt_size % 8 != 0 { return Err(format!("CODE 0: jump table size {} is not a multiple of the 8-byte entry", c.jt_size)); }
    if c.jt_offset < APP_PARAMS || c.jt_offset.checked_add(c.jt_size).is_none_or(|e| e > c.above) {
        return Err(format!("CODE 0: jump table (offset {}, size {}) does not fit above A5 ({} bytes)", c.jt_offset, c.jt_size, c.above));
    }
    let jt = d.get(16..16 + c.jt_size as usize).ok_or_else(|| format!("CODE 0 holds fewer than the {} jump-table bytes it declares", c.jt_size))?;
    if c.below.checked_add(c.above).is_none_or(|t| t > MAX_WORLD) { return Err(format!("CODE 0: A5 world of {}+{} bytes is unreasonable", c.below, c.above)); }
    Ok((c, jt))
}

impl<U: GuestUi> Mac<U> {
    /// Launch a classic Mac application from its file: MacBinary, an AppleDouble header file or a raw resource fork.
    /// Builds the A5 world and the jump table and leaves the CPU at the first jump-table entry, ready to [`Mac::step`].
    pub fn launch(&mut self, file: &[u8]) -> Result<(), String> {
        let opened = retro_resfork::open(file).map_err(|e| e.to_string())?;
        let fork = ResFork::parse(file[opened.rsrc.clone()].to_vec()).map_err(|e| e.to_string())?;
        self.rm.open_fork(APP_REF_NUM, fork);
        let code0 = {
            let f = &self.rm.files.last().ok_or("no resource file open")?.fork;
            let i = f.find(*b"CODE", 0).ok_or("no 'CODE' 0 resource: not a 68k application")?;
            f.data(i).ok_or("CODE 0 unreadable")?.to_vec()
        };
        let (c, jt) = parse_code0(&code0)?;
        if c.jt_size == 0 { return Err("CODE 0: an empty jump table has no entry point".into()); }
        // The A5 world is the first block ever allocated, so it begins exactly at STACK_TOP == CurStackBase: the stack
        // ends where the application's globals begin, which is what THINK C-style startup code relies on (it fills the
        // memory from CurStackBase up to A5 from its DATA resource).
        let world = self.mm.new_ptr(&mut self.mem, &mut self.heap, c.below + c.above, true).map_err(|e| format!("no memory for the A5 world (memFullErr {e})"))?;
        if world != STACK_TOP { return Err(format!("the A5 world landed at {world:#x}, not at STACK_TOP: launch needs a fresh machine")); }
        let a5 = world + c.below;
        self.mem.write_bytes(a5 + c.jt_offset, jt).map_err(|e| e.to_string())?;
        lowmem::set_app(&mut self.mem, &opened.name.unwrap_or_else(|| "Application".into()), a5, c.jt_offset as u16, APP_REF_NUM).map_err(|e| e.0)?;
        self.seg = SegLoader { a5, jt_offset: c.jt_offset, jt_size: c.jt_size, loaded: BTreeSet::new() };
        self.mem.write_u32(STACK_TOP - 4, EXIT_STUB).map_err(|e| e.to_string())?;
        self.ctx = Some(retro_m68k::start_with(self, a5 + c.jt_offset + 2, STACK_TOP - 4, &[(REG_A0 + 5, a5)]));
        self.log.push(format!("[segment] launch: A5 {a5:#x}, jump table {} entries at A5+{}", c.jt_size / 8, c.jt_offset));
        Ok(())
    }

    /// Load `CODE seg`: patch every jump-table entry the segment owns to `jmp abs.l`. Returns the log line.
    fn load_segment(&mut self, seg: u16) -> Result<String, String> {
        let h = self.get_resource(*b"CODE", seg as i16, true).map_err(|e| format!("no 'CODE' {seg} resource (ResErr {e})"))?;
        if let Some(b) = self.mm.block_mut(h) { b.locked = true; } // the code must not move once entries point into it
        let _ = self.mm.sync_master(&mut self.mem, h);
        let (base, size) = { let b = self.mm.block(h).ok_or("CODE handle vanished")?; (b.ptr, b.size) };
        if size < 4 { return Err(format!("CODE {seg} is shorter than its 4-byte header")); }
        let (first, count) = (self.mem.read_u16(base).map_err(|e| e.to_string())?, self.mem.read_u16(base + 2).map_err(|e| e.to_string())?);
        if first == 0xFFFF { return Err(format!("CODE {seg} has the 32-bit far-model header ($FFFF): not supported")); }
        let table = self.seg.a5 + self.seg.jt_offset;
        if u32::from(first) + u32::from(count) * 8 > self.seg.jt_size { return Err(format!("CODE {seg}: {count} entries at jump-table offset {first} run past the {}-byte table", self.seg.jt_size)); }
        for k in 0..u32::from(count) {
            let e = table + u32::from(first) + 8 * k;
            let offset = self.mem.read_u16(e).map_err(|e| e.to_string())?;
            if u32::from(offset) + 4 >= size { return Err(format!("CODE {seg}: entry {k} points {offset} bytes into a {size}-byte segment")); }
            self.mem.write_u16(e, seg).map_err(|e| e.to_string())?;
            self.mem.write_u16(e + 2, 0x4EF9).map_err(|e| e.to_string())?;
            self.mem.write_u32(e + 4, base + 4 + u32::from(offset)).map_err(|e| e.to_string())?;
        }
        self.seg.loaded.insert(seg);
        Ok(format!("[segment] LoadSeg {seg}: {count} entries patched, code at {:#x}", base + 4))
    }
}

impl<U: GuestUi> Mac<U> {
    /// Unload the segment owning the jump-table entry that `routine` (entry + 2) points into. Returns the log line.
    fn unload_segment(&mut self, routine: u32) -> Result<String, String> {
        let table = self.seg.a5 + self.seg.jt_offset;
        let entry = self.mem.strip(routine).wrapping_sub(2);
        if entry < table || entry >= table + self.seg.jt_size || (entry - table) % 8 != 0 {
            return Err(format!("{routine:#x} is not a jump-table entry (table {table:#x}..{:#x})", table + self.seg.jt_size));
        }
        let word = self.mem.read_u16(entry + 2).map_err(|e| e.to_string())?;
        if word != 0x4EF9 { return Ok(format!("[segment] UnloadSeg {routine:#x}: entry already unloaded (no-op)")); }
        let seg = self.mem.read_u16(entry).map_err(|e| e.to_string())?;
        let h = self.get_resource(*b"CODE", seg as i16, true).map_err(|e| format!("no 'CODE' {seg} resource (ResErr {e})"))?;
        let (base, size) = { let b = self.mm.block(h).ok_or("CODE handle vanished")?; (b.ptr, b.size) };
        if size < 4 { return Err(format!("CODE {seg} is shorter than its 4-byte header")); }
        let (first, count) = (self.mem.read_u16(base).map_err(|e| e.to_string())?, self.mem.read_u16(base + 2).map_err(|e| e.to_string())?);
        for k in 0..u32::from(count) {
            let e = table + u32::from(first) + 8 * k;
            let target = self.mem.read_u32(e + 4).map_err(|e| e.to_string())?;
            let offset = target.wrapping_sub(base + 4) as u16;
            self.mem.write_u16(e, offset).map_err(|e| e.to_string())?;
            self.mem.write_u16(e + 2, 0x3F3C).map_err(|e| e.to_string())?;
            self.mem.write_u16(e + 4, seg).map_err(|e| e.to_string())?;
            self.mem.write_u16(e + 6, 0xA9F0).map_err(|e| e.to_string())?;
        }
        if let Some(b) = self.mm.block_mut(h) { b.locked = false; b.purgeable = true; }
        let _ = self.mm.sync_master(&mut self.mem, h);
        self.seg.loaded.remove(&seg);
        Ok(format!("[segment] UnloadSeg {seg}: {count} entries back to their LoadSeg stubs, handle unlocked and purgeable"))
    }
}

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    m.register(0xA9F0, &[Arg::W], None, load_seg::<U>)?;
    m.register(0xA9F1, &[Arg::L], None, unload_seg::<U>)
}

/// `_LoadSeg`: stack is `[segNum][return address of the jsr to the jump-table entry]`. The trap word sits at entry+6,
/// so the entry's `jmp` (now patched) is at `trap - 4`; the frame's segNum word is popped by the Pascal epilogue and
/// execution resumes at the `jmp` with the caller's return address on top, exactly as if the entry had always been loaded.
fn load_seg<U: GuestUi>(m: &mut Mac<U>, cpu: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let seg = c.args.get(0) as u16;
    match m.load_segment(seg) {
        Ok(log) => { cpu.set_pc(c.pc - 4); Ok(Done { result: 0, log }) }
        Err(why) => { cpu.halt(); Err(Refusal(format!("LoadSeg {seg}: {why} (a real Mac raises system error 15)"))) }
    }
}

fn unload_seg<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let routine = c.args.get(0);
    m.unload_segment(routine).map(|log| Done { result: 0, log }).map_err(|why| Refusal(format!("UnloadSeg: {why}")))
}
