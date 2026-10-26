//! Resource files on the File Manager: `OpenResFile`, `CreateResFile`, `CloseResFile` (and `FSpOpenResFile` /
//! `FSpCreateResFile`, [`super::fsspec`]) over a file's RESOURCE FORK in the shared volume tree. An opened file joins
//! the Resource Manager's chain with the SAME refnum the File Manager gave its fork, so `CloseResFile(ref)` and the File
//! Manager's `Close` agree. The fork is read once, at open; changes made through the Resource Manager (`AddResource`,
//! `WriteResource`, [`crate::resmgr`]) reach the file when it is updated or closed (`CloseResFile` updates it first).
use super::names::Located;
use super::err;
use crate::machine::{Call, Done, Mac};
use crate::pascal::{read_pstring_bytes, Arg};
use crate::Refusal;
use alloc::format;
use heirloom_ui_abi::GuestUi;
use heirloom_cpu_68k::Cpu;
use heirloom_fmt_resfork::{writer::ForkBuilder, ResFork};

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    m.register(0xA997, &[Arg::P], Some(Arg::W), open_res_file::<U>)?;
    m.register(0xA9C4, &[Arg::P, Arg::W, Arg::B], Some(Arg::W), open_rf_perm::<U>)?;
    m.register(0xA81A, &[Arg::W, Arg::L, Arg::P, Arg::B], Some(Arg::W), hopen_res_file::<U>)?;
    m.register(0xA81B, &[Arg::W, Arg::L, Arg::P], None, hcreate_res_file::<U>)?;
    m.register(0xA9B1, &[Arg::P], None, create_res_file::<U>)?;
    m.register(0xA99A, &[Arg::W], None, close_res_file::<U>)?;
    m.register(0xA99B, &[Arg::B], None, set_res_load::<U>)
}

impl<U: GuestUi> Mac<U> {
    /// Open `loc`'s resource fork and add it to the resource chain; returns its refnum.
    pub fn open_res_file(&mut self, loc: &Located, perm: u8) -> Result<i16, i16> {
        let node = loc.node.ok_or(err::FNF_ERR)?;
        let r = self.fm.open(loc, true, perm)?;
        let bytes = self.fm.tree.get(node).rsrc.clone();
        match ResFork::parse(bytes) {
            Ok(fork) => { self.rm.open_fork(r, fork); Ok(r) }
            Err(_) => { let _ = self.fm.close(r); Err(err::EOF_ERR) } // no (or a damaged) resource fork
        }
    }

    /// Give an existing file an empty, valid resource fork (CreateResFile on a file without one).
    pub fn new_res_fork(&mut self, loc: &Located) -> Result<(), i16> {
        let node = loc.node.ok_or(err::FNF_ERR)?;
        if !self.fm.tree.is_writable(node) { return Err(err::W_PR_ERR); }
        if self.fm.tree.get(node).rsrc.is_empty() { self.fm.tree.get_mut(node).rsrc = ForkBuilder::new().build(); }
        Ok(())
    }
}

fn set_err<U: GuestUi>(m: &mut Mac<U>, e: i16) { let _ = m.mem.write_u16(crate::lowmem::RES_ERR, e as u16); }

fn open_res_file<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let name = read_pstring_bytes(&m.mem, c.args.get(0))?;
    let loc = m.fm.locate(0, 0, Some(&name)).map_err(|e| Refusal(format!("OpenResFile: name error {e}")));
    let res = match loc { Ok(l) => m.open_res_file(&l, 0), Err(Refusal(_)) => Err(err::FNF_ERR) };
    let (v, e) = match res { Ok(r) => (r as u16 as u32, 0), Err(e) => (0xFFFF, e) };
    set_err(m, e);
    Ok(Done { result: v, log: format!("[res] OpenResFile \"{}\" -> {} (ResErr {e})", name.iter().map(|&b| char::from(b)).collect::<alloc::string::String>(), v as u16 as i16) })
}

/// `OpenRFPerm(fileName, vRefNum, denyModes)`: `OpenResFile` with a vRefNum and the File Manager's permission byte
/// (0 fsCurPerm, 1 fsRdPerm, 2 fsWrPerm, 3 fsRdWrPerm, 4 fsRdWrShPerm; Inside Macintosh: More Macintosh Toolbox). The
/// fork is opened by the same path as `OpenResFile`, so a denied permission (a write on a locked file or read-only
/// volume) is the File Manager's own error in `ResErr` and the call answers -1.
fn open_rf_perm<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let name = read_pstring_bytes(&m.mem, c.args.get(0))?;
    let (vref, perm) = (c.args.get(1) as u16 as i16, c.args.get(2) as u8);
    let res = m.fm.locate(vref, 0, Some(&name)).and_then(|l| m.open_res_file(&l, perm));
    let (v, e) = match res { Ok(r) => (r as u16 as u32, 0), Err(e) => (0xFFFF, e) };
    set_err(m, e);
    Ok(Done { result: v, log: format!("[res] OpenRFPerm \"{}\" vRef {vref} perm {perm} -> {} (ResErr {e})", name.iter().map(|&b| char::from(b)).collect::<alloc::string::String>(), v as u16 as i16) })
}

/// `HOpenResFile(vRefNum, dirID, fileName, permission)`: `OpenResFile` in an explicit directory, with the File Manager's
/// permission byte (0 = the current permission). The answer is the refnum, or -1 with the error in `ResErr`.
fn hopen_res_file<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (vref, dir, perm) = (c.args.get(0) as u16 as i16, c.args.get(1), c.args.get(3) as u8);
    let name = read_pstring_bytes(&m.mem, c.args.get(2))?;
    let res = m.fm.locate(vref, dir, Some(&name)).and_then(|l| m.open_res_file(&l, perm));
    let (v, e) = match res { Ok(r) => (r as u16 as u32, 0), Err(e) => (0xFFFF, e) };
    set_err(m, e);
    Ok(Done { result: v, log: format!("[res] HOpenResFile \"{}\" vRef {vref} dirID {dir} perm {perm} -> {} (ResErr {e})", name.iter().map(|&b| char::from(b)).collect::<alloc::string::String>(), v as u16 as i16) })
}

/// `HCreateResFile(vRefNum, dirID, fileName)`: `CreateResFile` in an explicit directory.
fn hcreate_res_file<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (vref, dir) = (c.args.get(0) as u16 as i16, c.args.get(1));
    let name = read_pstring_bytes(&m.mem, c.args.get(2))?;
    m.fm.now_secs = m.clock();
    let res = m.fm.locate(vref, dir, Some(&name)).and_then(|l| {
        let l = if l.node.is_none() { let id = m.fm.create_file(&l)?; Located { node: Some(id), ..l } } else { l };
        m.new_res_fork(&l)
    });
    let e = res.err().unwrap_or(0);
    set_err(m, e);
    Ok(Done { result: 0, log: format!("[res] HCreateResFile \"{}\" vRef {vref} dirID {dir} (ResErr {e})", name.iter().map(|&b| char::from(b)).collect::<alloc::string::String>()) })
}

fn create_res_file<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let name = read_pstring_bytes(&m.mem, c.args.get(0))?;
    m.fm.now_secs = m.clock();
    let res = m.fm.locate(0, 0, Some(&name)).and_then(|l| {
        let l = if l.node.is_none() { let id = m.fm.create_file(&l)?; Located { node: Some(id), ..l } } else { l };
        m.new_res_fork(&l)
    });
    let e = res.err().unwrap_or(0);
    set_err(m, e);
    Ok(Done { result: 0, log: format!("[res] CreateResFile \"{}\" (ResErr {e})", name.iter().map(|&b| char::from(b)).collect::<alloc::string::String>()) })
}

fn close_res_file<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let r = c.args.get(0) as u16 as i16;
    // CloseResFile updates the file first (`UpdateResFile`), then lets go of it
    let updated = m.update_res_file(r);
    let e = if m.rm.close(r) { let _ = m.fm.close(r); updated.err().unwrap_or(0) } else { crate::resmgr::RES_F_NOT_FOUND };
    set_err(m, e);
    Ok(Done { result: 0, log: format!("[res] CloseResFile {r} (ResErr {e})") })
}

/// `SetResLoad(load)`: the ResLoad global (a Boolean in the high byte); with it FALSE, `Get*Resource` would return an empty
/// handle. This engine always loads, so the flag is recorded and honest about not changing behaviour.
fn set_res_load<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let on = c.args.get(0) as u8;
    m.mem.write_u16(crate::lowmem::RES_LOAD, u16::from(on) << 8).map_err(Refusal::mem("ResLoad"))?;
    Ok(Done { result: 0, log: format!("[res] SetResLoad {} (recorded; resources are always loaded)", on != 0) })
}
