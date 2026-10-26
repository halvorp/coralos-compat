//! The Resource Manager over [`heirloom_fmt_resfork`]: resources become guest Handles (a master pointer + a block from the
//! Memory Manager, flagged `resource`), so a program's `**h` reads the real bytes and `DisposeHandle`/`HLock`/
//! `GetHandleSize` work on them like on any handle.
//!
//! One resource file is open today: the application's own fork (refNum [`APP_REF_NUM`]); the chain machinery
//! (CurResFile, UseResFile, the `Get1*` variants that search only the current file) is in place for more.
//! `OpenResFile` needs the File Manager and is absent: calling it is an `unimplemented trap` refusal.
use crate::lowmem::RES_ERR;
use crate::machine::{Call, Done, Mac};
use crate::pascal::{read_pstring_bytes, Arg};
use crate::Refusal;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_ui_abi::GuestUi;
use heirloom_cpu_68k::Cpu;
use heirloom_fmt_resfork::{OsType, ResFork};

mod write;

/// The application's resource file reference number (an arbitrary small positive number; 0 is the System file).
pub const APP_REF_NUM: i16 = 2;
/// The System file's reference number.
pub const SYSTEM_REF_NUM: i16 = 0;
pub const RES_NOT_FOUND: i16 = -192;
pub const RES_F_NOT_FOUND: i16 = -193;
pub const ADD_RES_FAILED: i16 = -194;
pub const RMV_RES_FAILED: i16 = -196;
pub const RES_ATTR_ERR: i16 = -198;
/// resPurgeable, resLocked, resProtected and resChanged in a resource's attribute byte.
const ATTR_PURGEABLE: u8 = 0x20;
const ATTR_LOCKED: u8 = 0x10;
const ATTR_PROTECTED: u8 = 0x08;
const ATTR_CHANGED: u8 = 0x02;

pub struct ResFile {
    pub ref_num: i16,
    pub fork: ResFork,
    /// entry index -> the Handle it was loaded into (while that handle lives).
    loaded: BTreeMap<usize, u32>,
    /// The resource map has changes the file does not hold yet (`mapChanged`): `UpdateResFile` / `CloseResFile` write the file.
    dirty: bool,
}

#[derive(Default)]
pub struct ResMgr {
    /// The chain, oldest first; the last file is the most recently opened.
    pub files: Vec<ResFile>,
    /// CurResFile.
    pub cur: i16,
    /// Handle -> (refNum, entry index) for every live resource handle.
    by_handle: BTreeMap<u32, (i16, usize)>,
}

impl ResMgr {
    /// CloseResFile: drop the file (and the handles loaded from it stop being resources); false if it is not open.
    pub fn close(&mut self, ref_num: i16) -> bool {
        let Some(pos) = self.files.iter().position(|f| f.ref_num == ref_num) else { return false };
        let f = self.files.remove(pos);
        for h in f.loaded.values() { self.by_handle.remove(h); }
        if self.cur == ref_num { self.cur = self.files.last().map_or(0, |f| f.ref_num); }
        true
    }
    pub fn open_fork(&mut self, ref_num: i16, fork: ResFork) {
        self.files.push(ResFile { ref_num, fork, loaded: BTreeMap::new(), dirty: false });
        self.cur = ref_num;
    }
    /// The System file's resources (reference number 0): searched AFTER every other open file, so an application's own
    /// resource of the same type and ID wins. This is where the shared `'dcmp'` set comes from.
    pub fn add_system(&mut self, fork: ResFork) {
        self.files.retain(|f| f.ref_num != SYSTEM_REF_NUM);
        self.files.insert(0, ResFile { ref_num: SYSTEM_REF_NUM, fork, loaded: BTreeMap::new(), dirty: false });
    }
    pub(crate) fn file(&self, ref_num: i16) -> Option<&ResFile> { self.files.iter().find(|f| f.ref_num == ref_num) }
    /// The files a plain `Get*` searches: the current one, then the older ones. `only_cur` limits it to the current one.
    pub(crate) fn chain(&self, only_cur: bool) -> Vec<i16> {
        let Some(pos) = self.files.iter().position(|f| f.ref_num == self.cur) else { return Vec::new() };
        if only_cur { return alloc::vec![self.cur]; }
        self.files[..=pos].iter().rev().map(|f| f.ref_num).collect()
    }
    /// A disposed (or detached) handle stops being a resource.
    pub fn forget(&mut self, h: u32) {
        if let Some((rn, i)) = self.by_handle.remove(&h) {
            if let Some(f) = self.files.iter_mut().find(|f| f.ref_num == rn) { f.loaded.remove(&i); }
        }
    }
    pub fn is_resource(&self, h: u32) -> bool { self.by_handle.contains_key(&h) }
}

impl<U: GuestUi> Mac<U> {
    fn set_res_err(&mut self, e: i16) { let _ = self.mem.write_u16(RES_ERR, e as u16); }

    /// Load (or find already loaded) the resource at `entry` of file `rn` into a Handle.
    fn load_entry(&mut self, rn: i16, entry: usize) -> Result<u32, i16> {
        let file = self.rm.file(rn).ok_or(RES_F_NOT_FOUND)?;
        if let Some(&h) = file.loaded.get(&entry) { if self.mm.block(h).is_some() { return Ok(h); } }
        let data = file.fork.data(entry).ok_or(RES_NOT_FOUND)?.to_vec();
        let (attrs, ty, id) = file.fork.entry(entry).map(|e| (e.attrs, e.ty, e.id)).unwrap_or((0, [0; 4], 0));
        // resCompressed: the handle gets the DEcompressed size; the bytes arrive when the guest's 'dcmp' has run.
        let compressed = if attrs & crate::compressed::RES_COMPRESSED == 0 { None } else {
            match crate::compressed::parse(&data) {
                Ok(h) => h,
                Err(why) => { self.note_refusal("Resource Manager", format!("cannot decompress '{}' {id}: {why}", ty_str(ty))); return Err(crate::compressed::CANT_DECOMPRESS); }
            }
        };
        if compressed.is_some() && (ty == *b"dcmp" || !self.in_trap) {
            let why = if ty == *b"dcmp" { "a 'dcmp' resource cannot itself be compressed" } else { "decompression runs guest code and needs the 68k CPU of a trap (not available from this caller)" };
            self.note_refusal("Resource Manager", format!("cannot decompress '{}': {why}", ty_str(ty)));
            return Err(crate::compressed::CANT_DECOMPRESS);
        }
        let size = compressed.map_or(data.len() as u32, |c| c.size);
        let h = self.mm.new_handle(&mut self.mem, &mut self.heap, size, false)?;
        let p = self.mm.block(h).map(|b| b.ptr).ok_or(RES_NOT_FOUND)?;
        if compressed.is_none() { self.mem.write_bytes(p, &data).map_err(|_| RES_NOT_FOUND)?; }
        if let Some(b) = self.mm.block_mut(h) { b.resource = true; b.purgeable = attrs & ATTR_PURGEABLE != 0; b.locked = attrs & ATTR_LOCKED != 0; }
        let _ = self.mm.sync_master(&mut self.mem, h);
        self.rm.by_handle.insert(h, (rn, entry));
        if let Some(f) = self.rm.files.iter_mut().find(|f| f.ref_num == rn) { f.loaded.insert(entry, h); }
        if let Some(c) = compressed {
            if let Err(e) = self.schedule_decompress(&data, &c, h) { self.rm.forget(h); let _ = self.dispose_handle(h); return Err(e); }
            return Err(crate::compressed::PENDING);
        }
        Ok(h)
    }

    /// GetResource / Get1Resource.
    pub fn get_resource(&mut self, ty: OsType, id: i16, only_cur: bool) -> Result<u32, i16> {
        let r = (|| {
            for rn in self.rm.chain(only_cur) {
                if let Some(e) = self.rm.file(rn).and_then(|f| f.fork.find(ty, id)) { return self.load_entry(rn, e); }
            }
            Err(RES_NOT_FOUND)
        })();
        self.track_res_err(r)
    }
    /// Every load the engine makes for the program (a menu bar, a dialog, a segment) is a Resource Manager call: it leaves its
    /// result in `ResErr` exactly as the program's own `GetResource` does, success included (a stale error from an earlier probe
    /// must not survive a call that worked). The deferred-decompression answer is not a result and leaves it alone.
    fn track_res_err(&mut self, r: Result<u32, i16>) -> Result<u32, i16> {
        match r { Err(e) if e == crate::compressed::PENDING => {} Ok(_) => self.set_res_err(0), Err(e) => self.set_res_err(e) }
        r
    }
    /// `ty id` from ONE named resource file, whatever the current file is (the Segment Loader reads the APPLICATION's
    /// fork, not `CurMap`: a program that opened a preferences file and left it current still loads its code).
    pub fn get_resource_from(&mut self, rn: i16, ty: OsType, id: i16) -> Result<u32, i16> {
        let r = self.rm.file(rn).and_then(|f| f.fork.find(ty, id)).ok_or(RES_NOT_FOUND).and_then(|e| self.load_entry(rn, e));
        self.track_res_err(r)
    }
    pub fn get_named_resource(&mut self, ty: OsType, name: &[u8], only_cur: bool) -> Result<u32, i16> {
        for rn in self.rm.chain(only_cur) {
            if let Some(e) = self.rm.file(rn).and_then(|f| f.fork.find_named(ty, name)) { return self.load_entry(rn, e); }
        }
        Err(RES_NOT_FOUND)
    }
    pub fn count_resources(&self, ty: OsType, only_cur: bool) -> usize {
        self.rm.chain(only_cur).iter().filter_map(|&rn| self.rm.file(rn)).map(|f| f.fork.count(ty)).sum()
    }
    pub fn get_ind_resource(&mut self, ty: OsType, n: usize, only_cur: bool) -> Result<u32, i16> {
        let mut left = n;
        for rn in self.rm.chain(only_cur) {
            let Some(f) = self.rm.file(rn) else { continue };
            let c = f.fork.count(ty);
            if left >= 1 && left <= c { let e = f.fork.nth(ty, left).ok_or(RES_NOT_FOUND)?; return self.load_entry(rn, e); }
            left = left.saturating_sub(c);
        }
        Err(RES_NOT_FOUND)
    }
    /// The distinct resource types in the searched files, in the order first met.
    pub fn resource_types(&self, only_cur: bool) -> Vec<OsType> {
        let mut out: Vec<OsType> = Vec::new();
        for rn in self.rm.chain(only_cur) { if let Some(f) = self.rm.file(rn) { for t in f.fork.types() { if !out.contains(&t) { out.push(t); } } } }
        out
    }
    /// The bytes of a loaded resource handle (or any handle).
    pub fn handle_bytes(&self, h: u32) -> Option<Vec<u8>> {
        let b = self.mm.block(h)?;
        self.mem.bytes(self.mem.span(b.ptr, b.size).ok()?).ok().map(<[u8]>::to_vec)
    }
    /// (type, id, name) of a resource handle.
    pub fn res_info(&self, h: u32) -> Option<(OsType, i16, Option<Vec<u8>>)> {
        let &(rn, i) = self.rm.by_handle.get(&h)?;
        let f = self.rm.file(rn)?;
        let e = f.fork.entry(i)?;
        Some((e.ty, e.id, f.fork.name(i).map(<[u8]>::to_vec)))
    }
}

fn four(v: u32) -> OsType { v.to_be_bytes() }
fn ty_str(t: OsType) -> alloc::string::String { t.iter().map(|&b| char::from(b)).collect() }

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    for (t, one) in [(0xA9A0, false), (0xA81F, true)] { m.register(t, &[Arg::L, Arg::W], Some(Arg::L), if one { get1::<U> } else { get::<U> })?; }
    for (t, one) in [(0xA9A1, false), (0xA820, true)] { m.register(t, &[Arg::L, Arg::L], Some(Arg::L), if one { get_named1::<U> } else { get_named::<U> })?; }
    for (t, one) in [(0xA99C, false), (0xA80D, true)] { m.register(t, &[Arg::L], Some(Arg::W), if one { count1::<U> } else { count::<U> })?; }
    for (t, one) in [(0xA99D, false), (0xA80E, true)] { m.register(t, &[Arg::L, Arg::W], Some(Arg::L), if one { get_ind1::<U> } else { get_ind::<U> })?; }
    m.register(0xA9A3, &[Arg::P], None, release::<U>)?;
    m.register(0xA9A2, &[Arg::P], None, load::<U>)?;
    m.register(0xA992, &[Arg::P], None, detach::<U>)?;
    m.register(0xA9A5, &[Arg::P], Some(Arg::L), size_rsrc::<U>)?;
    m.register(0xA821, &[Arg::P], Some(Arg::L), max_size_rsrc::<U>)?;
    m.register(0xA9A6, &[Arg::P], Some(Arg::W), get_attrs::<U>)?;
    m.register(0xA9A8, &[Arg::L, Arg::L, Arg::L, Arg::L], None, get_info::<U>)?;
    m.register(0xA9B9, &[Arg::W], Some(Arg::L), get_cursor::<U>)?;
    m.register(0xA9BA, &[Arg::W], Some(Arg::L), get_string::<U>)?;
    m.register(0xA9BB, &[Arg::W], Some(Arg::L), get_icon::<U>)?;
    m.register(0xA9BC, &[Arg::W], Some(Arg::L), get_picture::<U>)?;
    m.register(0xA9B8, &[Arg::W], Some(Arg::L), get_pattern::<U>)?;
    m.register(0xA99E, &[], Some(Arg::W), count_types::<U>)?;
    m.register(0xA81C, &[], Some(Arg::W), count1_types::<U>)?;
    m.register(0xA99F, &[Arg::P, Arg::W], None, get_ind_type::<U>)?;
    m.register(0xA80F, &[Arg::P, Arg::W], None, get1_ind_type::<U>)?;
    m.register(0xA9AF, &[], Some(Arg::W), res_error::<U>)?;
    m.register(0xA994, &[], Some(Arg::W), cur_res_file::<U>)?;
    m.register(0xA998, &[Arg::W], None, use_res_file::<U>)?;
    write::register(m)
}

fn finish<U: GuestUi>(m: &mut Mac<U>, what: &str, r: Result<u32, i16>) -> Result<Done, Refusal> {
    let (e, v) = match r { Ok(v) => (0, v), Err(e) => (e, 0) };
    m.set_res_err(e);
    Ok(Done { result: v, log: format!("[res] {what} -> {v:#x} (ResErr {e})") })
}

fn get<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (t, id) = (four(c.args.get(0)), c.args.get(1) as u16 as i16);
    let r = m.get_resource(t, id, false);
    finish(m, &format!("GetResource '{}' {id}", ty_str(t)), r)
}
fn get1<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (t, id) = (four(c.args.get(0)), c.args.get(1) as u16 as i16);
    let r = m.get_resource(t, id, true);
    finish(m, &format!("Get1Resource '{}' {id}", ty_str(t)), r)
}
fn named<U: GuestUi>(m: &mut Mac<U>, c: &Call, only_cur: bool) -> Result<Done, Refusal> {
    let t = four(c.args.get(0));
    let name = read_pstring_bytes(&m.mem, c.args.get(1))?;
    let r = m.get_named_resource(t, &name, only_cur);
    finish(m, &format!("{} '{}' \"{}\"", c.name, ty_str(t), name.iter().map(|&b| char::from(b)).collect::<alloc::string::String>()), r)
}
fn get_named<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { named(m, c, false) }
fn get_named1<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { named(m, c, true) }
fn count<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let t = four(c.args.get(0));
    let n = m.count_resources(t, false) as u32;
    m.set_res_err(0);
    Ok(Done { result: n, log: format!("[res] CountResources '{}' -> {n}", ty_str(t)) })
}
fn count1<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let t = four(c.args.get(0));
    let n = m.count_resources(t, true) as u32;
    m.set_res_err(0);
    Ok(Done { result: n, log: format!("[res] Count1Resources '{}' -> {n}", ty_str(t)) })
}
fn ind<U: GuestUi>(m: &mut Mac<U>, c: &Call, only_cur: bool) -> Result<Done, Refusal> {
    let (t, n) = (four(c.args.get(0)), c.args.get(1) as u16 as i16);
    let r = if n < 1 { Err(RES_NOT_FOUND) } else { m.get_ind_resource(t, n as usize, only_cur) };
    finish(m, &format!("{} '{}' #{n}", c.name, ty_str(t)), r)
}
fn get_ind<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { ind(m, c, false) }
fn get_ind1<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { ind(m, c, true) }

fn release<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let h = c.args.get(0);
    if !m.rm.is_resource(h) { m.set_res_err(RES_NOT_FOUND); return Ok(Done { result: 0, log: format!("[res] ReleaseResource {h:#x}: not a resource handle (ResErr {RES_NOT_FOUND})") }); }
    let r = m.dispose_handle(h).map(|_| 0);
    finish(m, &format!("ReleaseResource {h:#x}"), r)
}
fn load<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    // Resources are loaded whole by Get*; a purgeable one is flagged, never purged, so there is nothing to reload.
    let h = c.args.get(0);
    let r = if m.rm.is_resource(h) { Ok(0) } else { Err(RES_NOT_FOUND) };
    finish(m, &format!("LoadResource {h:#x}"), r)
}
fn detach<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let h = c.args.get(0);
    let r = if m.rm.is_resource(h) { m.rm.forget(h); if let Some(b) = m.mm.block_mut(h) { b.resource = false; } let _ = m.mm.sync_master(&mut m.mem, h); Ok(0) } else { Err(RES_NOT_FOUND) };
    finish(m, &format!("DetachResource {h:#x}"), r)
}
fn size_rsrc<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let h = c.args.get(0);
    let r = if m.rm.is_resource(h) { m.mm.handle_size(h).map_err(|_| RES_NOT_FOUND) } else { Err(RES_NOT_FOUND) };
    finish(m, &format!("SizeRsrc {h:#x}"), r)
}
/// `FUNCTION MaxSizeRsrc(theResource: Handle): LONGINT` ($A821): the size of the resource as stored, i.e. the most the handle can need.
/// Every resource here is loaded whole (and a compressed one is expanded before the program sees it), so the answer is `SizeRsrc`'s;
/// a handle that is not a resource is resNotFound (-192) with a result of 0, exactly as `SizeRsrc`.
fn max_size_rsrc<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let h = c.args.get(0);
    let r = if m.rm.is_resource(h) { m.mm.handle_size(h).map_err(|_| RES_NOT_FOUND) } else { Err(RES_NOT_FOUND) };
    finish(m, &format!("MaxSizeRsrc {h:#x}"), r)
}
fn get_attrs<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let h = c.args.get(0);
    let r = m.rm.by_handle.get(&h).and_then(|&(rn, i)| m.rm.file(rn)?.fork.entry(i).map(|e| u32::from(e.attrs))).ok_or(RES_NOT_FOUND);
    finish(m, &format!("GetResAttrs {h:#x}"), r)
}
fn get_info<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (h, id_p, ty_p, name_p) = (c.args.get(0), c.args.get(1), c.args.get(2), c.args.get(3));
    let Some((t, id, name)) = m.res_info(h) else { m.set_res_err(RES_NOT_FOUND); return Ok(Done { result: 0, log: format!("[res] GetResInfo {h:#x}: not a resource handle") }) };
    let e = Refusal::mem("GetResInfo");
    m.mem.write_u16(id_p, id as u16).map_err(&e)?;
    m.mem.write_u32(ty_p, u32::from_be_bytes(t)).map_err(&e)?;
    if name_p != 0 {
        let n = name.unwrap_or_default();
        m.mem.write_u8(name_p, n.len() as u8).map_err(&e)?;
        m.mem.write_bytes(name_p + 1, &n).map_err(&e)?;
    }
    m.set_res_err(0);
    Ok(Done { result: 0, log: format!("[res] GetResInfo {h:#x} -> '{}' {id}", ty_str(t)) })
}
fn res_error<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    let e = m.mem.read_u16(RES_ERR).map_err(Refusal::mem("ResErr"))?;
    Ok(Done { result: u32::from(e), log: format!("[res] ResError -> {}", e as i16) })
}
fn cur_res_file<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    let c = m.rm.cur;
    Ok(Done { result: c as u16 as u32, log: format!("[res] CurResFile -> {c}") })
}
fn use_res_file<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let rn = c.args.get(0) as u16 as i16;
    if m.rm.file(rn).is_some() { m.rm.cur = rn; m.set_res_err(0); } else { m.set_res_err(RES_F_NOT_FOUND); }
    Ok(Done { result: 0, log: format!("[res] UseResFile {rn}") })
}


/// `GetCursor(id)`: a CURS resource is a handle like any other (`CursHandle`); the cursor image itself is the host's.
fn get_cursor<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let id = c.args.get(0) as u16 as i16;
    let r = m.get_resource(*b"CURS", id, false);
    finish(m, &format!("GetCursor {id}"), r)
}
/// `GetIcon(id)` / `GetPicture(id)` / `GetPattern(id)`: the `ICON` / `PICT` / `PAT ` resource as a handle (NIL, with ResErr, when none).
fn get_icon<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { typed_get(m, c, *b"ICON", "GetIcon") }
fn get_picture<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { typed_get(m, c, *b"PICT", "GetPicture") }
fn get_pattern<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { typed_get(m, c, *b"PAT ", "GetPattern") }
fn typed_get<U: GuestUi>(m: &mut Mac<U>, c: &Call, ty: OsType, name: &str) -> Result<Done, Refusal> {
    let id = c.args.get(0) as u16 as i16;
    let r = m.get_resource(ty, id, false);
    finish(m, &format!("{name} {id}"), r)
}
/// `GetString(stringID)`: the `STR ` resource as a `StringHandle` (NIL, with ResErr, when there is none).
fn get_string<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let id = c.args.get(0) as u16 as i16;
    let r = m.get_resource(*b"STR ", id, false);
    finish(m, &format!("GetString {id}"), r)
}
fn count_types_of<U: GuestUi>(m: &mut Mac<U>, only_cur: bool, what: &str) -> Result<Done, Refusal> {
    let n = m.resource_types(only_cur).len() as u32;
    m.set_res_err(0);
    Ok(Done { result: n, log: format!("[res] {what} -> {n}") })
}
fn count_types<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> { count_types_of(m, false, "CountTypes") }
fn count1_types<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> { count_types_of(m, true, "Count1Types") }
/// `GetIndType(VAR theType, index)`: the index'th (1-based) type; an index out of range leaves theType 0 and sets resNotFound.
fn ind_type<U: GuestUi>(m: &mut Mac<U>, c: &Call, only_cur: bool) -> Result<Done, Refusal> {
    let (at, n) = (c.args.get(0), c.args.get(1) as u16 as i16);
    let t = if n >= 1 { m.resource_types(only_cur).get(n as usize - 1).copied() } else { None };
    m.mem.write_u32(at, t.map_or(0, u32::from_be_bytes)).map_err(Refusal::mem("GetIndType result"))?;
    m.set_res_err(if t.is_some() { 0 } else { RES_NOT_FOUND });
    Ok(Done { result: 0, log: format!("[res] {} #{n} -> {}", c.name, t.map_or(String::from("none"), ty_str)) })
}
fn get_ind_type<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { ind_type(m, c, false) }
fn get1_ind_type<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { ind_type(m, c, true) }
