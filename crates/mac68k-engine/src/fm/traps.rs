//! The File Manager's trap glue: the classic OS traps `_Open` ... `_SetFPos` (bit 9 of the trap word selects the HFS
//! form: `PBHOpen`, `PBHGetFInfo` ... which read the directory from `ioDirID`), `_HFSDispatch` / `_FSDispatch`
//! (selector in D0), and the ioResult / ioCompletion discipline: every call runs to completion before the trap returns;
//! `ioResult` is set; a synchronous call returns the result in D0, an asynchronous one (bit 10) returns noErr in D0 and
//! then runs `ioCompletion` (if non-nil) with A0 = the block and D0 = ioResult, as the real Mac does from the queue.
use super::catalog::CatInfo;
use super::names::Located;
use super::pb::*;
use super::{bytes_of, err};
use crate::guestcall::GuestCall;
use crate::machine::Mac;
use crate::osconv::Regs;
use crate::Refusal;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use guestui_abi::GuestUi;
use retro_m68k::Cpu;
use retro_volume::FInfo;

type R = Result<String, Refusal>;
/// A call body: the parameter block, whether it is the HFS form. Ok(()) or the OSErr.
type Body<U> = fn(&mut Mac<U>, Pb, bool) -> Result<(), i16>;

const ASYNC: u16 = 0x0400;
const HFS: u16 = 0x0200;
/// The Device Manager is not modelled: `_Open` of a driver name (".Sony", ".Print") finds no such driver.
pub const NS_DRV_ERR: i16 = -56;

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    macro_rules! reg { ($($op:expr => $name:expr, $body:expr;)*) => {$(
        m.register_os($op, { fn h<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> R { run(m, r, $name, $body) } h::<U> })?;
    )*} }
    reg! {
        0xA000 => "Open", open_data::<U>;
        0xA00A => "OpenRF", open_rsrc::<U>;
        0xA001 => "Close", close::<U>;
        0xA002 => "Read", read::<U>;
        0xA003 => "Write", write::<U>;
        0xA011 => "GetEOF", get_eof::<U>;
        0xA012 => "SetEOF", set_eof::<U>;
        0xA018 => "GetFPos", get_fpos::<U>;
        0xA044 => "SetFPos", set_fpos::<U>;
        0xA010 => "Allocate", allocate::<U>;
        0xA045 => "FlushFile", flush_file::<U>;
        0xA008 => "Create", create::<U>;
        0xA009 => "Delete", delete::<U>;
        0xA00B => "Rename", rename::<U>;
        0xA00C => "GetFileInfo", get_file_info::<U>;
        0xA00D => "SetFileInfo", set_file_info::<U>;
        0xA041 => "SetFilLock", set_flock::<U>;
        0xA042 => "RstFilLock", rst_flock::<U>;
        0xA007 => "GetVolInfo", get_vol_info::<U>;
        0xA014 => "GetVol", get_vol::<U>;
        0xA015 => "SetVol", set_vol::<U>;
        0xA013 => "FlushVol", flush_vol::<U>;
    }
    // HFSDispatch ($A260) and FSDispatch ($A060) share one table key (the flag bits are dropped) and one set of selectors.
    m.register_os(0xA260, dispatch::<U>)
}

/// The ioResult / D0 / ioCompletion discipline around one call body.
fn run<U: GuestUi>(m: &mut Mac<U>, r: &mut Regs, name: &str, body: Body<U>) -> R {
    let pb = Pb(r.a0);
    let hfs = r.opcode & HFS != 0;
    let res = body(m, pb, hfs).err().unwrap_or(0);
    let async_call = r.opcode & ASYNC != 0;
    let e = Refusal::mem("parameter block");
    pb.put_i16(&mut m.mem, IO_RESULT, res).map_err(&e)?;
    let done = pb.u32(&m.mem, IO_COMPLETION).map_err(&e)?;
    r.err(if async_call { 0 } else { res });
    if async_call && done != 0 {
        m.pending_call = Some(GuestCall { target: m.mem.strip(done), d0: Some(res as i32 as u32), a0: Some(pb.0), d1: None, restore: Some((0, 0)) });
    }
    Ok(format!("[file] {name}{}{} -> {res}{}", if hfs { " (HFS)" } else { "" }, if async_call { " (async)" } else { "" }, if async_call && done != 0 { " + completion" } else { "" }))
}

fn mem_err(what: &'static str) -> impl Fn(retro_guest_mem::MemError) -> i16 { move |_| { let _ = what; err::PARAM_ERR } }

/// (ioVRefNum, ioDirID, ioNamePtr) of a block, the directory read only for the HFS form.
fn target<U: GuestUi>(m: &Mac<U>, pb: Pb, dir_id: Option<u32>) -> Result<Located, i16> {
    let g = mem_err("block");
    let vref = pb.i16(&m.mem, IO_VREF_NUM).map_err(&g)?;
    let np = pb.u32(&m.mem, IO_NAME_PTR).map_err(&g)?;
    let name = if np == 0 { None } else { Some(read_name(m, np)?) };
    if name.as_deref().is_some_and(|n| n.first() == Some(&b'.')) { return Err(NS_DRV_ERR); }
    m.fm.locate(vref, dir_id.unwrap_or(0), name.as_deref())
}
fn read_name<U: GuestUi>(m: &Mac<U>, np: u32) -> Result<Vec<u8>, i16> {
    crate::pascal::read_pstring_bytes(&m.mem, np).map_err(|_| err::BD_NAM_ERR)
}
fn dir_of<U: GuestUi>(m: &Mac<U>, pb: Pb, hfs: bool) -> Result<Option<u32>, i16> {
    if hfs { pb.u32(&m.mem, IO_DIR_ID).map(Some).map_err(mem_err("dirID")) } else { Ok(None) }
}
fn loc<U: GuestUi>(m: &Mac<U>, pb: Pb, hfs: bool) -> Result<Located, i16> { target(m, pb, dir_of(m, pb, hfs)?) }

fn put_name<U: GuestUi>(m: &mut Mac<U>, np: u32, name: &[u8]) -> Result<(), i16> {
    if np == 0 { return Ok(()); }
    let n = name.len().min(255);
    m.mem.write_u8(np, n as u8).and_then(|_| m.mem.write_bytes(np + 1, &name[..n])).map_err(mem_err("name"))
}

// ---- forks ----------------------------------------------------------------------------------------------------------

fn open_fork<U: GuestUi>(m: &mut Mac<U>, pb: Pb, hfs: bool, rsrc: bool) -> Result<(), i16> {
    let l = loc(m, pb, hfs)?;
    let perm = pb.u8(&m.mem, IO_PERMSSN).map_err(mem_err("perm"))?;
    let r = m.fm.open(&l, rsrc, perm)?;
    pb.put_i16(&mut m.mem, IO_REF_NUM, r).map_err(mem_err("refnum"))
}
fn open_data<U: GuestUi>(m: &mut Mac<U>, pb: Pb, hfs: bool) -> Result<(), i16> { open_fork(m, pb, hfs, false) }
fn open_rsrc<U: GuestUi>(m: &mut Mac<U>, pb: Pb, hfs: bool) -> Result<(), i16> { open_fork(m, pb, hfs, true) }

fn close<U: GuestUi>(m: &mut Mac<U>, pb: Pb, _: bool) -> Result<(), i16> {
    let r = pb.i16(&m.mem, IO_REF_NUM).map_err(mem_err("refnum"))?;
    m.fm.close(r)?;
    m.rm.close(r);
    Ok(())
}

fn read<U: GuestUi>(m: &mut Mac<U>, pb: Pb, _: bool) -> Result<(), i16> {
    let g = mem_err("block");
    let (r, buf, req) = (pb.i16(&m.mem, IO_REF_NUM).map_err(&g)?, pb.u32(&m.mem, IO_BUFFER).map_err(&g)?, pb.u32(&m.mem, IO_REQ_COUNT).map_err(&g)?);
    let (mode, off) = (pb.i16(&m.mem, IO_POS_MODE).map_err(&g)? as u16, pb.u32(&m.mem, IO_POS_OFFSET).map_err(&g)? as i32);
    pb.put_u32(&mut m.mem, IO_ACT_COUNT, 0).map_err(&g)?;
    let out = m.fm.read(r, mode, off, req)?;
    m.mem.write_bytes(buf, &out.data).map_err(&g)?;
    pb.put_u32(&mut m.mem, IO_ACT_COUNT, out.data.len() as u32).map_err(&g)?;
    pb.put_u32(&mut m.mem, IO_POS_OFFSET, m.fm.mark(r)?).map_err(&g)?;
    if out.result == 0 { Ok(()) } else { Err(out.result) }
}

fn write<U: GuestUi>(m: &mut Mac<U>, pb: Pb, _: bool) -> Result<(), i16> {
    let g = mem_err("block");
    let (r, buf, req) = (pb.i16(&m.mem, IO_REF_NUM).map_err(&g)?, pb.u32(&m.mem, IO_BUFFER).map_err(&g)?, pb.u32(&m.mem, IO_REQ_COUNT).map_err(&g)?);
    let (mode, off) = (pb.i16(&m.mem, IO_POS_MODE).map_err(&g)? as u16, pb.u32(&m.mem, IO_POS_OFFSET).map_err(&g)? as i32);
    pb.put_u32(&mut m.mem, IO_ACT_COUNT, 0).map_err(&g)?;
    let data = m.mem.span(buf, req).and_then(|s| m.mem.bytes(s).map(<[u8]>::to_vec)).map_err(&g)?;
    let n = m.fm.write(r, mode, off, &data)?;
    pb.put_u32(&mut m.mem, IO_ACT_COUNT, n).map_err(&g)?;
    pb.put_u32(&mut m.mem, IO_POS_OFFSET, m.fm.mark(r)?).map_err(&g)
}

fn get_eof<U: GuestUi>(m: &mut Mac<U>, pb: Pb, _: bool) -> Result<(), i16> {
    let g = mem_err("block");
    let e = m.fm.eof(pb.i16(&m.mem, IO_REF_NUM).map_err(&g)?)?;
    pb.put_u32(&mut m.mem, IO_MISC, e).map_err(&g)
}
fn set_eof<U: GuestUi>(m: &mut Mac<U>, pb: Pb, _: bool) -> Result<(), i16> {
    let g = mem_err("block");
    m.fm.set_eof(pb.i16(&m.mem, IO_REF_NUM).map_err(&g)?, pb.u32(&m.mem, IO_MISC).map_err(&g)?)
}
fn get_fpos<U: GuestUi>(m: &mut Mac<U>, pb: Pb, _: bool) -> Result<(), i16> {
    let g = mem_err("block");
    let mk = m.fm.mark(pb.i16(&m.mem, IO_REF_NUM).map_err(&g)?)?;
    pb.put_u32(&mut m.mem, IO_POS_OFFSET, mk).map_err(&g)
}
fn set_fpos<U: GuestUi>(m: &mut Mac<U>, pb: Pb, _: bool) -> Result<(), i16> {
    let g = mem_err("block");
    let r = pb.i16(&m.mem, IO_REF_NUM).map_err(&g)?;
    let (mode, off) = (pb.i16(&m.mem, IO_POS_MODE).map_err(&g)? as u16, pb.u32(&m.mem, IO_POS_OFFSET).map_err(&g)? as i32);
    let res = m.fm.set_mark(r, mode, off);
    pb.put_u32(&mut m.mem, IO_POS_OFFSET, m.fm.mark(r)?).map_err(&g)?;
    res
}
fn allocate<U: GuestUi>(m: &mut Mac<U>, pb: Pb, _: bool) -> Result<(), i16> {
    // Allocation is not a concern of an in-memory volume: the request is granted whole (ioActCount = ioReqCount).
    let g = mem_err("block");
    m.fm.eof(pb.i16(&m.mem, IO_REF_NUM).map_err(&g)?)?;
    let n = pb.u32(&m.mem, IO_REQ_COUNT).map_err(&g)?;
    pb.put_u32(&mut m.mem, IO_ACT_COUNT, n).map_err(&g)
}
fn flush_file<U: GuestUi>(m: &mut Mac<U>, pb: Pb, _: bool) -> Result<(), i16> { m.fm.eof(pb.i16(&m.mem, IO_REF_NUM).map_err(mem_err("block"))?).map(|_| ()) }

// ---- the catalog ------------------------------------------------------------------------------------------------------

fn create<U: GuestUi>(m: &mut Mac<U>, pb: Pb, hfs: bool) -> Result<(), i16> { let l = loc(m, pb, hfs)?; m.fm.now_secs = m.clock(); m.fm.create_file(&l).map(|_| ()) }
fn delete<U: GuestUi>(m: &mut Mac<U>, pb: Pb, hfs: bool) -> Result<(), i16> { let l = loc(m, pb, hfs)?; m.fm.delete(&l) }
fn rename<U: GuestUi>(m: &mut Mac<U>, pb: Pb, hfs: bool) -> Result<(), i16> {
    let l = loc(m, pb, hfs)?;
    let np = pb.u32(&m.mem, IO_MISC).map_err(mem_err("block"))?;
    let new = read_name(m, np)?;
    if new.contains(&b':') { return Err(err::BD_NAM_ERR); }
    m.fm.rename(&l, &new)
}
fn set_flock<U: GuestUi>(m: &mut Mac<U>, pb: Pb, hfs: bool) -> Result<(), i16> { let l = loc(m, pb, hfs)?; m.fm.set_lock(&l, true) }
fn rst_flock<U: GuestUi>(m: &mut Mac<U>, pb: Pb, hfs: bool) -> Result<(), i16> { let l = loc(m, pb, hfs)?; m.fm.set_lock(&l, false) }

fn finfo_bytes(f: &FInfo) -> [u8; 16] {
    let mut b = [0u8; 16];
    b[0..4].copy_from_slice(&f.file_type);
    b[4..8].copy_from_slice(&f.creator);
    b[8..10].copy_from_slice(&f.flags.to_be_bytes());
    b[10..12].copy_from_slice(&f.v.to_be_bytes());
    b[12..14].copy_from_slice(&f.h.to_be_bytes());
    b
}
fn finfo_of(b: &[u8]) -> FInfo {
    FInfo { file_type: [b[0], b[1], b[2], b[3]], creator: [b[4], b[5], b[6], b[7]], flags: u16::from_be_bytes([b[8], b[9]]), v: i16::from_be_bytes([b[10], b[11]]), h: i16::from_be_bytes([b[12], b[13]]) }
}

/// Fill a FileParam / HFileInfo / DirInfo block from a catalog record. `cat` = the HFS-sized block (to offset 108).
fn fill_info<U: GuestUi>(m: &mut Mac<U>, pb: Pb, i: &CatInfo, cat: bool) -> Result<(), i16> {
    let g = mem_err("block");
    let mm = &mut m.mem;
    pb.put_u8(mm, IO_FL_ATTRIB, i.attrib).map_err(&g)?;
    pb.put_u8(mm, IO_AC_USER, 0).map_err(&g)?;
    pb.put_bytes(mm, IO_FL_FNDR_INFO, &if i.is_dir { [0; 16] } else { finfo_bytes(&i.finfo) }).map_err(&g)?;
    pb.put_u32(mm, IO_DIR_ID, i.id).map_err(&g)?;
    if i.is_dir {
        pb.put_i16(mm, IO_DR_NM_FLS, i.items as i16).map_err(&g)?;
        pb.put_bytes(mm, 54, &[0; 18]).map_err(&g)?;
    } else {
        pb.put_i16(mm, IO_FL_ST_BLK, 0).map_err(&g)?;
        for (off, v) in [(IO_FL_LG_LEN, i.lg), (IO_FL_PY_LEN, i.py)] { pb.put_u32(mm, off, v).map_err(&g)?; }
        pb.put_i16(mm, IO_FL_RST_BLK, 0).map_err(&g)?;
        for (off, v) in [(IO_FL_RLG_LEN, i.rlg), (IO_FL_RPY_LEN, i.rpy)] { pb.put_u32(mm, off, v).map_err(&g)?; }
    }
    pb.put_u32(mm, IO_FL_CR_DAT, i.created).map_err(&g)?;
    pb.put_u32(mm, IO_FL_MD_DAT, i.modified).map_err(&g)?;
    if cat {
        pb.put_u32(mm, IO_FL_BK_DAT, 0).map_err(&g)?;
        pb.put_bytes(mm, 84, &[0; 16]).map_err(&g)?;
        pb.put_u32(mm, IO_FL_PAR_ID, i.parent_id).map_err(&g)?;
        if !i.is_dir { pb.put_u32(mm, IO_FL_CLP_SIZ, super::ALLOC_BLOCK).map_err(&g)?; }
    }
    Ok(())
}

/// GetFileInfo (`cat` false: files only, indexes count files) and GetCatInfo / PBHGetFInfo (`cat` true).
fn get_info<U: GuestUi>(m: &mut Mac<U>, pb: Pb, hfs: bool, cat: bool) -> Result<(), i16> {
    let g = mem_err("block");
    let index = pb.i16(&m.mem, IO_FDIR_INDEX).map_err(&g)?;
    let np = pb.u32(&m.mem, IO_NAME_PTR).map_err(&g)?;
    let (vref, dir_id) = (pb.i16(&m.mem, IO_VREF_NUM).map_err(&g)?, if hfs { pb.u32(&m.mem, IO_DIR_ID).map_err(&g)? } else { 0 });
    let node = if index > 0 {
        let (_, dir) = m.fm.start_dir(vref, dir_id)?;
        m.fm.nth_entry(dir, index, !cat && !hfs).ok_or(err::FNF_ERR)?
    } else if index < 0 {
        let (_, dir) = m.fm.start_dir(vref, dir_id)?;
        dir
    } else {
        let name = if np == 0 { None } else { Some(read_name(m, np)?) };
        m.fm.locate(vref, dir_id, name.as_deref())?.node.ok_or(err::FNF_ERR)?
    };
    let info = m.fm.cat_info(node);
    if info.is_dir && !cat && !hfs { return Err(err::FNF_ERR); }
    if index != 0 { put_name(m, np, &info.name)?; }
    let ref_num = m.fm.open_forks().find(|(_, f)| f.node == node && !f.rsrc).map_or(0, |(&r, _)| r);
    fill_info(m, pb, &info, cat || hfs)?;
    pb.put_i16(&mut m.mem, IO_REF_NUM, ref_num).map_err(&g)
}
fn get_file_info<U: GuestUi>(m: &mut Mac<U>, pb: Pb, hfs: bool) -> Result<(), i16> { get_info(m, pb, hfs, false) }
fn get_cat_info<U: GuestUi>(m: &mut Mac<U>, pb: Pb, _: bool) -> Result<(), i16> { get_info(m, pb, true, true) }

fn set_info<U: GuestUi>(m: &mut Mac<U>, pb: Pb, hfs: bool) -> Result<(), i16> {
    let g = mem_err("block");
    let l = loc(m, pb, hfs)?;
    let node = l.node.ok_or(err::FNF_ERR)?;
    let finfo = finfo_of(&pb.bytes(&m.mem, IO_FL_FNDR_INFO, 16).map_err(&g)?);
    let (cr, md) = (pb.u32(&m.mem, IO_FL_CR_DAT).map_err(&g)?, pb.u32(&m.mem, IO_FL_MD_DAT).map_err(&g)?);
    m.fm.set_cat(node, Some(finfo), cr, md)
}
fn set_file_info<U: GuestUi>(m: &mut Mac<U>, pb: Pb, hfs: bool) -> Result<(), i16> { set_info(m, pb, hfs) }
fn set_cat_info<U: GuestUi>(m: &mut Mac<U>, pb: Pb, _: bool) -> Result<(), i16> { set_info(m, pb, true) }

fn dir_create<U: GuestUi>(m: &mut Mac<U>, pb: Pb, _: bool) -> Result<(), i16> {
    let l = loc(m, pb, true)?;
    m.fm.now_secs = m.clock();
    let d = m.fm.create_dir(&l)?;
    let id = m.fm.dir_id(d);
    pb.put_u32(&mut m.mem, IO_DIR_ID, id).map_err(mem_err("block"))
}
fn cat_move<U: GuestUi>(m: &mut Mac<U>, pb: Pb, _: bool) -> Result<(), i16> {
    let g = mem_err("block");
    let l = loc(m, pb, true)?;
    let (np, dir) = (pb.u32(&m.mem, IO_NEW_NAME).map_err(&g)?, pb.u32(&m.mem, IO_NEW_DIR_ID).map_err(&g)?);
    let vref = pb.i16(&m.mem, IO_VREF_NUM).map_err(&g)?;
    let name = if np == 0 { None } else { Some(read_name(m, np)?) };
    let to = m.fm.locate(vref, dir, name.as_deref())?.node.ok_or(err::DIR_NF_ERR)?;
    m.fm.cat_move(&l, to)
}

// ---- volumes and working directories -----------------------------------------------------------------------------------

fn get_vol_info<U: GuestUi>(m: &mut Mac<U>, pb: Pb, _: bool) -> Result<(), i16> {
    let g = mem_err("block");
    let (index, vref, np) = (pb.i16(&m.mem, IO_VOL_INDEX).map_err(&g)?, pb.i16(&m.mem, IO_VREF_NUM).map_err(&g)?, pb.u32(&m.mem, IO_NAME_PTR).map_err(&g)?);
    let name = if np == 0 || index >= 0 { None } else { Some(read_name(m, np)?) };
    let vol = m.fm.pick_volume(index, vref, name.as_deref())?;
    let i = m.fm.vol_info(vol);
    put_name(m, np, &i.name)?;
    let mm = &mut m.mem;
    pb.put_i16(mm, IO_VREF_NUM, i.vref).map_err(&g)?;
    pb.put_u32(mm, IO_VCR_DATE, i.created).map_err(&g)?;
    pb.put_u32(mm, IO_VLS_BK_UP, 0).map_err(&g)?;
    pb.put_i16(mm, IO_VATRB, i.attrib as i16).map_err(&g)?;
    pb.put_i16(mm, IO_VNM_FLS, i.files_in_root as i16).map_err(&g)?;
    pb.put_i16(mm, IO_VDIR_ST, 3).map_err(&g)?;
    pb.put_i16(mm, IO_VBL_LN, 0).map_err(&g)?;
    pb.put_i16(mm, IO_VNM_AL_BLKS, i.total_blocks as i16).map_err(&g)?;
    pb.put_u32(mm, IO_VAL_BLK_SIZ, super::ALLOC_BLOCK).map_err(&g)?;
    pb.put_u32(mm, IO_VCLP_SIZ, super::ALLOC_BLOCK).map_err(&g)?;
    pb.put_i16(mm, IO_AL_BL_ST, 8).map_err(&g)?;
    pb.put_u32(mm, IO_VNXT_FNUM, 0x1000).map_err(&g)?;
    pb.put_i16(mm, IO_VFR_BLK, i.free_blocks as i16).map_err(&g)?;
    pb.put_i16(mm, IO_VSIG_WORD, 0x4244).map_err(&g)?; // 'BD': an HFS volume
    pb.put_u32(mm, IO_VFIL_CNT, i.file_count).map_err(&g)?;
    pb.put_u32(mm, IO_VDIR_CNT, i.dir_count).map_err(&g)
}

fn get_vol<U: GuestUi>(m: &mut Mac<U>, pb: Pb, hfs: bool) -> Result<(), i16> {
    let g = mem_err("block");
    let (vref, name) = m.fm.default_vref();
    let np = pb.u32(&m.mem, IO_NAME_PTR).map_err(&g)?;
    put_name(m, np, &name)?;
    pb.put_i16(&mut m.mem, IO_VREF_NUM, vref).map_err(&g)?;
    if hfs {
        let (vol, dir) = (m.fm.def_vol, m.fm.def_dir);
        let (vv, id) = (m.fm.vref_of(vol), m.fm.dir_id(dir));
        pb.put_i16(&mut m.mem, IO_WD_VREF_NUM, vv).map_err(&g)?;
        pb.put_u32(&mut m.mem, IO_WD_DIR_ID, id).map_err(&g)?;
    }
    Ok(())
}
fn set_vol<U: GuestUi>(m: &mut Mac<U>, pb: Pb, hfs: bool) -> Result<(), i16> { let l = loc(m, pb, hfs)?; m.fm.set_default(&l) }
fn flush_vol<U: GuestUi>(m: &mut Mac<U>, pb: Pb, _: bool) -> Result<(), i16> {
    // Volumes live in memory: a flush has nothing to write, but it still validates the volume it names.
    let l = target(m, pb, None)?;
    l.node.ok_or(err::NSV_ERR).map(|_| ())
}

fn open_wd<U: GuestUi>(m: &mut Mac<U>, pb: Pb, _: bool) -> Result<(), i16> {
    let g = mem_err("block");
    let l = target(m, pb, Some(pb.u32(&m.mem, IO_WD_DIR_ID).map_err(&g)?))?;
    let dir = l.node.filter(|&n| m.fm.tree.is_dir(n)).ok_or(err::DIR_NF_ERR)?;
    let wd = m.fm.wd_for(l.vol, dir);
    pb.put_i16(&mut m.mem, IO_VREF_NUM, wd).map_err(&g)
}
fn close_wd<U: GuestUi>(m: &mut Mac<U>, pb: Pb, _: bool) -> Result<(), i16> { m.fm.close_wd(pb.i16(&m.mem, IO_VREF_NUM).map_err(mem_err("block"))?) }
fn get_wd_info<U: GuestUi>(m: &mut Mac<U>, pb: Pb, _: bool) -> Result<(), i16> {
    let g = mem_err("block");
    let vref = pb.i16(&m.mem, IO_VREF_NUM).map_err(&g)?;
    let (vol, dir) = m.fm.wd(vref).or_else(|| m.fm.volume_of_vref(vref).ok().map(|v| (v, v))).ok_or(err::NSV_ERR)?;
    let (vv, id) = (m.fm.vref_of(vol), m.fm.dir_id(dir));
    let name = bytes_of(m.fm.tree.name(vol));
    let np = pb.u32(&m.mem, IO_NAME_PTR).map_err(&g)?;
    put_name(m, np, &name)?;
    pb.put_u32(&mut m.mem, IO_WD_PROC_ID, 0).map_err(&g)?;
    pb.put_i16(&mut m.mem, IO_WD_VREF_NUM, vv).map_err(&g)?;
    pb.put_u32(&mut m.mem, IO_WD_DIR_ID, id).map_err(&g)
}

fn get_fcb_info<U: GuestUi>(m: &mut Mac<U>, pb: Pb, _: bool) -> Result<(), i16> {
    let g = mem_err("block");
    let idx = pb.u32(&m.mem, IO_FCB_INDX).map_err(&g)?;
    let r = if idx > 0 { m.fm.nth_open(idx).ok_or(err::RF_NUM_ERR)? } else { pb.i16(&m.mem, IO_REF_NUM).map_err(&g)? };
    let (node, rsrc) = m.fm.fork_node(r)?;
    let (info, f) = (m.fm.cat_info(node), m.fm.open_fork(r).cloned().ok_or(err::RF_NUM_ERR)?);
    let (len, vol) = (m.fm.eof(r)?, m.fm.tree.volume_of(node));
    let np = pb.u32(&m.mem, IO_NAME_PTR).map_err(&g)?;
    put_name(m, np, &info.name)?;
    let vv = m.fm.vref_of(vol);
    let mm = &mut m.mem;
    pb.put_i16(mm, IO_REF_NUM, r).map_err(&g)?;
    pb.put_u32(mm, IO_FCB_FL_NM, info.id).map_err(&g)?;
    pb.put_i16(mm, IO_FCB_FLAGS, (if f.perm.write { 0x0100 } else { 0 }) | (if rsrc { 0x0200 } else { 0 })).map_err(&g)?;
    pb.put_i16(mm, IO_FCB_ST_BLK, 0).map_err(&g)?;
    pb.put_u32(mm, IO_FCB_EOF, len).map_err(&g)?;
    pb.put_u32(mm, IO_FCB_PLEN, len.div_ceil(super::ALLOC_BLOCK) * super::ALLOC_BLOCK).map_err(&g)?;
    pb.put_u32(mm, IO_FCB_CR_PS, f.mark).map_err(&g)?;
    pb.put_i16(mm, IO_FCB_VREF_NUM, vv).map_err(&g)?;
    pb.put_u32(mm, IO_FCB_CLP_SIZ, super::ALLOC_BLOCK).map_err(&g)?;
    pb.put_u32(mm, IO_FCB_PAR_ID, info.parent_id).map_err(&g)
}

// ---- _HFSDispatch / _FSDispatch -------------------------------------------------------------------------------------------

/// Selectors (D0.W), from Files.h's `HFSDispatch` table.
pub mod sel {
    pub const OPEN_WD: u16 = 1;
    pub const CLOSE_WD: u16 = 2;
    pub const CAT_MOVE: u16 = 5;
    pub const DIR_CREATE: u16 = 6;
    pub const GET_WD_INFO: u16 = 7;
    pub const GET_FCB_INFO: u16 = 8;
    pub const GET_CAT_INFO: u16 = 9;
    pub const SET_CAT_INFO: u16 = 10;
    pub const OPEN_DF: u16 = 0x1A;
}

fn dispatch<U: GuestUi>(m: &mut Mac<U>, c: &mut Cpu, r: &mut Regs) -> R {
    let selector = c.dreg(0) as u16;
    let body: Option<(&str, Body<U>)> = match selector {
        sel::OPEN_WD => Some(("OpenWD", open_wd::<U>)),
        sel::CLOSE_WD => Some(("CloseWD", close_wd::<U>)),
        sel::CAT_MOVE => Some(("CatMove", cat_move::<U>)),
        sel::DIR_CREATE => Some(("DirCreate", dir_create::<U>)),
        sel::GET_WD_INFO => Some(("GetWDInfo", get_wd_info::<U>)),
        sel::GET_FCB_INFO => Some(("GetFCBInfo", get_fcb_info::<U>)),
        sel::GET_CAT_INFO => Some(("GetCatInfo", get_cat_info::<U>)),
        sel::SET_CAT_INFO => Some(("SetCatInfo", set_cat_info::<U>)),
        sel::OPEN_DF => Some(("OpenDF", open_data::<U>)),
        _ => None,
    };
    match body {
        Some((name, f)) => { r.opcode |= HFS; run(m, r, name, f) }
        None => Err(Refusal(format!("HFSDispatch selector {selector:#x} is not modelled"))),
    }
}
