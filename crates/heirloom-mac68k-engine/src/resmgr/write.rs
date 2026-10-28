//! Changing a resource file (Inside Macintosh: More Macintosh Toolbox, "Resource Manager": `AddResource`, `ChangedResource`,
//! `WriteResource`, `UpdateResFile`, `RmveResource`, `SetResInfo`, `SetResAttrs`, `SetResPurge`, `UniqueID`; the semantics are the
//! Resource Manager's, cross-checked against Executor's `res/resMod.cpp` as facts).
//!
//! The model. The resource map lives in memory ([`heirloom_fmt_resfork::ResFork`], indices that never move); a resource's DATA lives in the
//! guest handle the program holds, and is copied into the map's copy when it is written. The file (the resource fork of a tree node,
//! reached through the File Manager refnum the file was opened with) holds what `UpdateResFile` / `CloseResFile` last wrote.
//!
//! * `AddResource(theData, theType, theID, name)` puts the handle into the CURRENT resource file's map: the handle becomes a resource
//!   handle, the entry is `resChanged` and the map is changed; the bytes reach the file at the next update. A NIL or already-resource
//!   handle is `addResFailed`. The ID is not checked for being new (the Resource Manager does not).
//! * `ChangedResource` sets `resChanged` (a `resProtected` resource is `resAttrErr`) and marks the map changed.
//! * `WriteResource` copies a `resChanged` resource's bytes into the map's copy and clears the bit; a fork not open for writing is
//!   `wrPermErr` (-61).
//! * `UpdateResFile(refNum)` writes every `resChanged` resource and then the whole file when the map changed. A file not open for
//!   writing is left alone with no error, as the real call does. A resource file that is not a File Manager fork (the application's
//!   own image) cannot be written: that is the same no-op, named in the log.
//! * `RmveResource` takes the entry out (`rmvResFailed` for a NIL handle or a `resProtected` resource), the handle stops being a resource.
//! * `SetResInfo` / `SetResAttrs` change the entry only; the program calls `ChangedResource` to make it permanent, as the book says.
use super::{ResFile, ADD_RES_FAILED, ATTR_CHANGED, ATTR_PROTECTED, RES_ATTR_ERR, RES_F_NOT_FOUND, RES_NOT_FOUND, RMV_RES_FAILED};
use crate::fm::err;
use crate::machine::{Call, Done, Mac};
use crate::pascal::{read_pstring_bytes, Arg};
use crate::Refusal;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

/// The first ID `UniqueID` hands out: the IDs below are reserved for the System (Inside Macintosh: "Resource IDs").
const FIRST_UNIQUE_ID: i16 = 128;

impl<U: GuestUi> Mac<U> {
    fn file_mut(&mut self, rn: i16) -> Option<&mut ResFile> { self.rm.files.iter_mut().find(|f| f.ref_num == rn) }

    /// (refnum, index) of the resource `h`.
    fn resource_of(&self, h: u32) -> Result<(i16, usize), i16> { self.rm.by_handle.get(&h).copied().ok_or(RES_NOT_FOUND) }

    /// `AddResource`: the handle becomes a resource of the current file.
    pub(crate) fn add_resource(&mut self, h: u32, ty: [u8; 4], id: i16, name: &[u8]) -> Result<(), i16> {
        if h == 0 || self.mm.block(h).is_none() || self.rm.by_handle.contains_key(&h) { return Err(ADD_RES_FAILED); }
        let rn = self.rm.cur;
        let file = self.file_mut(rn).ok_or(RES_F_NOT_FOUND)?;
        let idx = file.fork.add(ty, id, (!name.is_empty()).then_some(name), ATTR_CHANGED, Vec::new());
        file.loaded.insert(idx, h);
        file.dirty = true;
        self.rm.by_handle.insert(h, (rn, idx));
        if let Some(b) = self.mm.block_mut(h) { b.resource = true; }
        let _ = self.mm.sync_master(&mut self.mem, h);
        Ok(())
    }

    /// `ChangedResource`.
    pub(crate) fn changed_resource(&mut self, h: u32) -> Result<(), i16> {
        let (rn, i) = self.resource_of(h)?;
        let file = self.file_mut(rn).ok_or(RES_F_NOT_FOUND)?;
        let attrs = file.fork.entry(i).ok_or(RES_NOT_FOUND)?.attrs;
        if attrs & ATTR_PROTECTED != 0 { return Err(RES_ATTR_ERR); }
        file.fork.set_attrs(i, attrs | ATTR_CHANGED);
        file.dirty = true;
        Ok(())
    }

    /// Is the resource file `rn` open for writing on a File Manager fork? The fork's node when it is.
    fn writable_fork(&self, rn: i16) -> Option<heirloom_volume::NodeId> {
        let f = self.fm.open_fork(rn)?;
        (f.perm.write && f.rsrc).then_some(f.node)
    }

    /// Copy the bytes of resource `i` of file `rn` from its handle into the map when it is `resChanged`.
    fn copy_changed(&mut self, rn: i16, i: usize) {
        let Some(file) = self.rm.files.iter().find(|f| f.ref_num == rn) else { return };
        let (Some(e), Some(&h)) = (file.fork.entry(i), file.loaded.get(&i)) else { return };
        if e.attrs & ATTR_CHANGED == 0 { return; }
        let attrs = e.attrs;
        let Some(bytes) = self.handle_bytes(h) else { return };
        if let Some(file) = self.file_mut(rn) { file.fork.set_data(i, bytes); file.fork.set_attrs(i, attrs & !ATTR_CHANGED); file.dirty = true; }
    }

    /// `WriteResource`.
    pub(crate) fn write_resource(&mut self, h: u32) -> Result<(), i16> {
        let (rn, i) = self.resource_of(h)?;
        if self.writable_fork(rn).is_none() { return Err(err::WR_PERM_ERR); }
        self.copy_changed(rn, i);
        Ok(())
    }

    /// `UpdateResFile`: the changed resources and the map go to the file when it is open for writing.
    pub(crate) fn update_res_file(&mut self, rn: i16) -> Result<(), i16> {
        let Some(file) = self.rm.files.iter().find(|f| f.ref_num == rn) else { return Err(RES_F_NOT_FOUND) };
        let Some(node) = self.writable_fork(rn) else {
            let why = if self.fm.open_fork(rn).is_none() { "the application's own file is not on a volume" } else { "the file is not open for writing" };
            self.log.push(format!("[res] UpdateResFile {rn}: nothing is written ({why})"));
            return Ok(());
        };
        let changed: Vec<usize> = (0..file.fork.len()).filter(|&i| file.fork.entry(i).is_some_and(|e| e.attrs & ATTR_CHANGED != 0)).collect();
        for i in changed { self.copy_changed(rn, i); }
        let file = self.file_mut(rn).ok_or(RES_F_NOT_FOUND)?;
        if !file.dirty { return Ok(()); }
        file.dirty = false;
        let bytes = file.fork.to_bytes();
        self.fm.now_secs = self.clock();
        self.fm.tree.get_mut(node).rsrc = bytes;
        self.fm.touch(node);
        Ok(())
    }

    /// `RmveResource`.
    pub(crate) fn remove_resource(&mut self, h: u32) -> Result<(), i16> {
        if h == 0 { return Err(RMV_RES_FAILED); }
        let (rn, i) = self.resource_of(h)?;
        let file = self.file_mut(rn).ok_or(RES_F_NOT_FOUND)?;
        if file.fork.entry(i).ok_or(RES_NOT_FOUND)?.attrs & ATTR_PROTECTED != 0 { return Err(RMV_RES_FAILED); }
        file.fork.remove(i);
        file.loaded.remove(&i);
        file.dirty = true;
        self.rm.by_handle.remove(&h);
        if let Some(b) = self.mm.block_mut(h) { b.resource = false; }
        let _ = self.mm.sync_master(&mut self.mem, h);
        Ok(())
    }

    /// An ID no resource of `ty` has in the files searched (the current one only for `Unique1ID`).
    fn unique_id(&self, ty: [u8; 4], only_cur: bool) -> Option<i16> {
        let used = |id: i16| self.rm.chain(only_cur).iter().any(|&rn| self.rm.file(rn).is_some_and(|f| f.fork.find(ty, id).is_some()));
        (FIRST_UNIQUE_ID..=i16::MAX).find(|&id| !used(id))
    }
}

fn four(v: u32) -> [u8; 4] { v.to_be_bytes() }
fn ty_str(t: [u8; 4]) -> String { t.iter().map(|&b| char::from(b)).collect() }
fn pstr_text(b: &[u8]) -> String { b.iter().map(|&c| char::from(c)).collect() }

fn finish<U: GuestUi>(m: &mut Mac<U>, what: String, r: Result<(), i16>) -> Result<Done, Refusal> {
    let e = r.err().unwrap_or(0);
    m.set_res_err(e);
    Ok(Done { result: 0, log: format!("[res] {what} (ResErr {e})") })
}

fn add_resource<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (h, ty, id) = (c.args.get(0), four(c.args.get(1)), c.args.get(2) as u16 as i16);
    let name = if c.args.get(3) == 0 { Vec::new() } else { read_pstring_bytes(&m.mem, c.args.get(3))? };
    let r = m.add_resource(h, ty, id, &name);
    finish(m, format!("AddResource {h:#x} '{}' {id} \"{}\"", ty_str(ty), pstr_text(&name)), r)
}
fn changed_resource<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let h = c.args.get(0);
    let r = m.changed_resource(h);
    finish(m, format!("ChangedResource {h:#x}"), r)
}
fn write_resource<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let h = c.args.get(0);
    let r = m.write_resource(h);
    finish(m, format!("WriteResource {h:#x}"), r)
}
fn update_res_file<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let rn = c.args.get(0) as u16 as i16;
    let r = m.update_res_file(rn);
    finish(m, format!("UpdateResFile {rn}"), r)
}
fn rmve_resource<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let h = c.args.get(0);
    let r = m.remove_resource(h);
    finish(m, format!("RmveResource {h:#x}"), r)
}
fn set_res_info<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (h, id) = (c.args.get(0), c.args.get(1) as u16 as i16);
    let name = if c.args.get(2) == 0 { None } else { Some(read_pstring_bytes(&m.mem, c.args.get(2))?) };
    let r = (|| {
        let (rn, i) = m.resource_of(h)?;
        let file = m.file_mut(rn).ok_or(RES_F_NOT_FOUND)?;
        if file.fork.entry(i).ok_or(RES_NOT_FOUND)?.attrs & ATTR_PROTECTED != 0 { return Err(RES_ATTR_ERR); }
        file.fork.set_info(i, id, name.as_deref());
        Ok(())
    })();
    finish(m, format!("SetResInfo {h:#x} {id}"), r)
}
fn set_res_attrs<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (h, attrs) = (c.args.get(0), c.args.get(1) as u8);
    let r = (|| {
        let (rn, i) = m.resource_of(h)?;
        let file = m.file_mut(rn).ok_or(RES_F_NOT_FOUND)?;
        file.fork.set_attrs(i, attrs).then_some(()).ok_or(RES_NOT_FOUND)
    })();
    finish(m, format!("SetResAttrs {h:#x} {attrs:#04x}"), r)
}
/// `SetResPurge(install)`: the Resource Manager's own purge-writing safeguard; this engine never purges, so there is nothing to install.
fn set_res_purge<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    finish(m, format!("SetResPurge {} (nothing is ever purged)", c.args.get(0) as u8 != 0), Ok(()))
}
fn unique_ids<U: GuestUi>(m: &mut Mac<U>, c: &Call, only_cur: bool) -> Result<Done, Refusal> {
    let ty = four(c.args.get(0));
    let id = m.unique_id(ty, only_cur);
    m.set_res_err(0);
    match id {
        Some(id) => Ok(Done { result: u32::from(id as u16), log: format!("[res] {} '{}' -> {id}", c.name, ty_str(ty)) }),
        None => Err(Refusal(format!("{} '{}': every ID from {FIRST_UNIQUE_ID} up is in use", c.name, ty_str(ty)))),
    }
}
fn unique_id_trap<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { unique_ids(m, c, false) }
fn unique1_id_trap<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { unique_ids(m, c, true) }

pub(super) fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    use Arg::{B, L, P, W};
    m.register(0xA9AB, &[L, L, W, P], None, add_resource::<U>)?;
    m.register(0xA9AA, &[L], None, changed_resource::<U>)?;
    m.register(0xA9B0, &[L], None, write_resource::<U>)?;
    m.register(0xA999, &[W], None, update_res_file::<U>)?;
    m.register(0xA9AD, &[L], None, rmve_resource::<U>)?;
    m.register(0xA9A9, &[L, W, P], None, set_res_info::<U>)?;
    m.register(0xA9A7, &[L, W], None, set_res_attrs::<U>)?;
    m.register(0xA993, &[B], None, set_res_purge::<U>)?;
    m.register(0xA9C1, &[L], Some(W), unique_id_trap::<U>)?;
    m.register(0xA810, &[L], Some(W), unique1_id_trap::<U>)
}
