//! The Resource Manager over [`retro_resfork`]: resources become guest Handles (a master pointer + a block from the
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
use alloc::vec::Vec;
use guestui_abi::GuestUi;
use retro_m68k::Cpu;
use retro_resfork::{OsType, ResFork};

/// The application's resource file reference number (an arbitrary small positive number; 0 is the System file).
pub const APP_REF_NUM: i16 = 2;
pub const RES_NOT_FOUND: i16 = -192;
pub const RES_F_NOT_FOUND: i16 = -193;
/// resPurgeable and resLocked in a resource's attribute byte.
const ATTR_PURGEABLE: u8 = 0x20;
const ATTR_LOCKED: u8 = 0x10;

pub struct ResFile {
    pub ref_num: i16,
    pub fork: ResFork,
    /// entry index -> the Handle it was loaded into (while that handle lives).
    loaded: BTreeMap<usize, u32>,
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
    pub fn open_fork(&mut self, ref_num: i16, fork: ResFork) {
        self.files.push(ResFile { ref_num, fork, loaded: BTreeMap::new() });
        self.cur = ref_num;
    }
    fn file(&self, ref_num: i16) -> Option<&ResFile> { self.files.iter().find(|f| f.ref_num == ref_num) }
    /// The files a plain `Get*` searches: the current one, then the older ones. `only_cur` limits it to the current one.
    fn chain(&self, only_cur: bool) -> Vec<i16> {
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
        let attrs = file.fork.entry(entry).map(|e| e.attrs).unwrap_or(0);
        let h = self.mm.new_handle(&mut self.mem, &mut self.heap, data.len() as u32, false)?;
        let p = self.mm.block(h).map(|b| b.ptr).ok_or(RES_NOT_FOUND)?;
        self.mem.write_bytes(p, &data).map_err(|_| RES_NOT_FOUND)?;
        if let Some(b) = self.mm.block_mut(h) { b.resource = true; b.purgeable = attrs & ATTR_PURGEABLE != 0; b.locked = attrs & ATTR_LOCKED != 0; }
        self.rm.by_handle.insert(h, (rn, entry));
        if let Some(f) = self.rm.files.iter_mut().find(|f| f.ref_num == rn) { f.loaded.insert(entry, h); }
        Ok(h)
    }

    /// GetResource / Get1Resource.
    pub fn get_resource(&mut self, ty: OsType, id: i16, only_cur: bool) -> Result<u32, i16> {
        for rn in self.rm.chain(only_cur) {
            if let Some(e) = self.rm.file(rn).and_then(|f| f.fork.find(ty, id)) { return self.load_entry(rn, e); }
        }
        Err(RES_NOT_FOUND)
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
    m.register(0xA9A3, &[Arg::L], None, release::<U>)?;
    m.register(0xA9A2, &[Arg::L], None, load::<U>)?;
    m.register(0xA992, &[Arg::L], None, detach::<U>)?;
    m.register(0xA9A5, &[Arg::L], Some(Arg::L), size_rsrc::<U>)?;
    m.register(0xA9A6, &[Arg::L], Some(Arg::W), get_attrs::<U>)?;
    m.register(0xA9A8, &[Arg::L, Arg::L, Arg::L, Arg::L], None, get_info::<U>)?;
    m.register(0xA9AF, &[], Some(Arg::W), res_error::<U>)?;
    m.register(0xA994, &[], Some(Arg::W), cur_res_file::<U>)?;
    m.register(0xA998, &[Arg::W], None, use_res_file::<U>)
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
    let r = if m.rm.is_resource(h) { m.rm.forget(h); if let Some(b) = m.mm.block_mut(h) { b.resource = false; } Ok(0) } else { Err(RES_NOT_FOUND) };
    finish(m, &format!("DetachResource {h:#x}"), r)
}
fn size_rsrc<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let h = c.args.get(0);
    let r = if m.rm.is_resource(h) { m.mm.handle_size(h).map_err(|_| RES_NOT_FOUND) } else { Err(RES_NOT_FOUND) };
    finish(m, &format!("SizeRsrc {h:#x}"), r)
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

