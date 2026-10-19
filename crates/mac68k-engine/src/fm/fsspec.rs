//! The high-level File Manager: FSSpec records and the `FSp...` calls (`_HighLevelFSDispatch`, $AA52; Inside Macintosh:
//! Files, "FSSpec"). They are PASCAL calls with the selector in D0.W, so the arguments are read from the stack
//! ([`crate::pascal`]) with a layout chosen by the selector; the result is the OSErr (or the refnum of `FSpOpenResFile`)
//! in the slot the caller reserved, `FSpCreateResFile` being a procedure.
use super::names::Located;
use super::pb::{FSSPEC_SIZE, FSS_NAME, FSS_PAR_ID, FSS_VREF};
use super::{bytes_of, err, FileMgr};
use crate::machine::Mac;
use crate::osconv::Regs;
use crate::pascal::{self, Arg};
use crate::Refusal;
use alloc::format;
use alloc::string::String;
use guestui_abi::GuestUi;
use retro_m68k::Cpu;
use retro_volume::FInfo;

/// Selectors of `_HighLevelFSDispatch` (Files.h).
pub mod sel {
    pub const MAKE_FSSPEC: u16 = 1;
    pub const OPEN_DF: u16 = 2;
    pub const OPEN_RF: u16 = 3;
    pub const CREATE: u16 = 4;
    pub const DIR_CREATE: u16 = 5;
    pub const DELETE: u16 = 6;
    pub const GET_FINFO: u16 = 7;
    pub const SET_FINFO: u16 = 8;
    pub const SET_FLOCK: u16 = 9;
    pub const RST_FLOCK: u16 = 10;
    pub const RENAME: u16 = 11;
    pub const CAT_MOVE: u16 = 12;
    pub const OPEN_RES_FILE: u16 = 13;
    pub const CREATE_RES_FILE: u16 = 14;
    pub const EXCHANGE_FILES: u16 = 15;
}

const W: Arg = Arg::W;
const L: Arg = Arg::L;
const B: Arg = Arg::B;
/// Arguments in declaration order, and whether the call is a function (it has the 2-byte result slot).
fn layout(selector: u16) -> Option<(&'static [Arg], bool)> {
    Some(match selector {
        sel::MAKE_FSSPEC => (&[W, L, L, L], true),
        sel::OPEN_DF | sel::OPEN_RF => (&[L, B, L], true),
        sel::CREATE => (&[L, L, L, W], true),
        sel::DIR_CREATE => (&[L, W, L], true),
        sel::DELETE | sel::SET_FLOCK | sel::RST_FLOCK => (&[L], true),
        sel::GET_FINFO | sel::SET_FINFO | sel::RENAME | sel::CAT_MOVE | sel::EXCHANGE_FILES => (&[L, L], true),
        sel::OPEN_RES_FILE => (&[L, B], true),
        sel::CREATE_RES_FILE => (&[L, L, L, W], false),
        _ => return None,
    })
}

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> { m.register_os(0xAA52, dispatch::<U>) }

fn mem<U: GuestUi>(_: &Mac<U>) -> impl Fn(retro_guest_mem::MemError) -> i16 { |_| err::PARAM_ERR }

/// An FSSpec in guest memory -> the entry it names.
pub(super) fn spec_loc<U: GuestUi>(m: &Mac<U>, p: u32) -> Result<Located, i16> {
    let g = mem(m);
    let (vref, par) = (m.mem.read_i16(p + FSS_VREF).map_err(&g)?, m.mem.read_u32(p + FSS_PAR_ID).map_err(&g)?);
    let name = pascal::read_pstring_bytes(&m.mem, p + FSS_NAME).map_err(|_| err::BD_NAM_ERR)?;
    m.fm.locate(vref, par, Some(&name))
}

pub(super) fn write_spec<U: GuestUi>(m: &mut Mac<U>, p: u32, loc: &Located) -> Result<(), i16> {
    let g = mem(m);
    let vol = loc.vol;
    let (name, par) = match (loc.node, &loc.leaf) {
        (Some(n), _) => (bytes_of(m.fm.tree.name(n)), if m.fm.tree.is_volume(n) { super::RT_PAR_ID } else { m.fm.dir_id(m.fm.tree.parent(n)) }),
        (None, Some(l)) => (bytes_of(l), m.fm.dir_id(loc.parent)),
        (None, None) => return Err(err::BD_NAM_ERR),
    };
    let vref = m.fm.vref_of(vol);
    m.mem.write_u16(p + FSS_VREF, vref as u16).map_err(&g)?;
    m.mem.write_u32(p + FSS_PAR_ID, par).map_err(&g)?;
    m.mem.zero(p + FSS_NAME, FSSPEC_SIZE - FSS_NAME).map_err(&g)?;
    m.mem.write_u8(p + FSS_NAME, name.len().min(63) as u8).and_then(|_| m.mem.write_bytes(p + FSS_NAME + 1, &name[..name.len().min(63)])).map_err(&g)
}

fn dispatch<U: GuestUi>(m: &mut Mac<U>, c: &mut Cpu, r: &mut Regs) -> Result<String, Refusal> {
    let selector = c.dreg(0) as u16;
    let Some((lay, func)) = layout(selector) else { return Err(Refusal(format!("HighLevelFSDispatch selector {selector} is not modelled"))) };
    let sp = c.areg(7);
    let args = pascal::read_args(&m.mem, sp, lay)?;
    let res = fsp(m, selector, &|i| args.get(i));
    let (e, value) = match res { Ok(v) => (0, v), Err(e) => (e, 0) };
    // FSpOpenResFile returns the refnum (or -1) instead of an OSErr.
    let result = if selector == sel::OPEN_RES_FILE { if e == 0 { value } else { 0xFFFF } } else { e as u16 as u32 };
    let new_sp = pascal::complete(&mut m.mem, sp, lay, if func { Some((Arg::W, result)) } else { None })?;
    c.set_areg(7, new_sp);
    r.d0 = 0;
    Ok(format!("[file] HighLevelFSDispatch {selector} -> {}", if selector == sel::OPEN_RES_FILE { result as i16 } else { e }))
}

fn fsp<U: GuestUi>(m: &mut Mac<U>, selector: u16, a: &dyn Fn(usize) -> u32) -> Result<u32, i16> {
    let g = mem(m);
    m.fm.now_secs = m.clock();
    match selector {
        sel::MAKE_FSSPEC => {
            let np = a(2);
            let name = if np == 0 { None } else { Some(pascal::read_pstring_bytes(&m.mem, np).map_err(|_| err::BD_NAM_ERR)?) };
            let l = m.fm.locate(a(0) as u16 as i16, a(1), name.as_deref())?;
            write_spec(m, a(3), &l)?;
            if l.node.is_none() { Err(err::FNF_ERR) } else { Ok(0) } // the spec is valid for a create even then
        }
        sel::OPEN_DF | sel::OPEN_RF => {
            let l = spec_loc(m, a(0))?;
            let r = m.fm.open(&l, selector == sel::OPEN_RF, a(1) as u8)?;
            m.mem.write_u16(a(2), r as u16).map_err(&g)?;
            Ok(0)
        }
        sel::CREATE | sel::CREATE_RES_FILE => {
            let l = spec_loc(m, a(0))?;
            if selector == sel::CREATE_RES_FILE && l.node.is_some() { return m.new_res_fork(&l).map(|_| 0); }
            let id = m.fm.create_file(&l)?;
            let f = &mut m.fm.tree.get_mut(id).finfo;
            f.creator = a(1).to_be_bytes();
            f.file_type = a(2).to_be_bytes();
            if selector == sel::CREATE_RES_FILE { let l = Located { node: Some(id), ..l }; m.new_res_fork(&l)?; }
            Ok(0)
        }
        sel::DIR_CREATE => {
            let l = spec_loc(m, a(0))?;
            let d = m.fm.create_dir(&l)?;
            let id = m.fm.dir_id(d);
            if a(2) != 0 { m.mem.write_u32(a(2), id).map_err(&g)?; }
            Ok(0)
        }
        sel::DELETE => { let l = spec_loc(m, a(0))?; m.fm.delete(&l).map(|_| 0) }
        sel::SET_FLOCK | sel::RST_FLOCK => { let l = spec_loc(m, a(0))?; m.fm.set_lock(&l, selector == sel::SET_FLOCK).map(|_| 0) }
        sel::GET_FINFO => {
            let node = spec_loc(m, a(0))?.node.ok_or(err::FNF_ERR)?;
            let i = m.fm.cat_info(node);
            let f = &i.finfo;
            let mut b = [0u8; 16];
            b[0..4].copy_from_slice(&f.file_type);
            b[4..8].copy_from_slice(&f.creator);
            b[8..10].copy_from_slice(&f.flags.to_be_bytes());
            b[10..12].copy_from_slice(&f.v.to_be_bytes());
            b[12..14].copy_from_slice(&f.h.to_be_bytes());
            m.mem.write_bytes(a(1), &b).map_err(&g)?;
            Ok(0)
        }
        sel::SET_FINFO => {
            let node = spec_loc(m, a(0))?.node.ok_or(err::FNF_ERR)?;
            let b = m.mem.span(a(1), 16).and_then(|s| m.mem.bytes(s).map(<[u8]>::to_vec)).map_err(&g)?;
            let f = FInfo { file_type: [b[0], b[1], b[2], b[3]], creator: [b[4], b[5], b[6], b[7]], flags: u16::from_be_bytes([b[8], b[9]]), v: i16::from_be_bytes([b[10], b[11]]), h: i16::from_be_bytes([b[12], b[13]]) };
            let (cr, md) = { let n = m.fm.tree.get(node); (super::catalog::mac_secs(n.created_ms), super::catalog::mac_secs(n.modified_ms)) };
            m.fm.set_cat(node, Some(f), cr, md).map(|_| 0)
        }
        sel::RENAME => {
            let l = spec_loc(m, a(0))?;
            let new = pascal::read_pstring_bytes(&m.mem, a(1)).map_err(|_| err::BD_NAM_ERR)?;
            m.fm.rename(&l, &new).map(|_| 0)
        }
        sel::CAT_MOVE => {
            let l = spec_loc(m, a(0))?;
            let to = spec_loc(m, a(1))?.node.filter(|&n| m.fm.tree.is_dir(n)).ok_or(err::DIR_NF_ERR)?;
            m.fm.cat_move(&l, to).map(|_| 0)
        }
        sel::OPEN_RES_FILE => {
            let l = spec_loc(m, a(0))?;
            m.open_res_file(&l, a(1) as u8).map(|r| r as u16 as u32)
        }
        sel::EXCHANGE_FILES => {
            let (x, y) = (spec_loc(m, a(0))?.node.ok_or(err::FNF_ERR)?, spec_loc(m, a(1))?.node.ok_or(err::FNF_ERR)?);
            m.fm.exchange(x, y).map(|_| 0)
        }
        _ => Err(err::PARAM_ERR),
    }
}

impl FileMgr {
    /// Swap the contents (both forks) and File info (FInfo) of two files, keeping their names and places (FSpExchangeFiles).
    pub fn exchange(&mut self, x: retro_volume::NodeId, y: retro_volume::NodeId) -> Result<(), i16> {
        if x == y || self.tree.file_data(x).is_none() || self.tree.file_data(y).is_none() { return Err(err::PARAM_ERR); }
        if !self.tree.is_writable(x) || !self.tree.is_writable(y) { return Err(err::W_PR_ERR); }
        if self.is_open(x, false) || self.is_open(y, false) { return Err(err::F_BSY_ERR); }
        let (dx, dy) = (core::mem::take(self.tree.file_data_mut(x).expect("file")), core::mem::take(self.tree.file_data_mut(y).expect("file")));
        *self.tree.file_data_mut(x).expect("file") = dy;
        *self.tree.file_data_mut(y).expect("file") = dx;
        let (rx, ry) = (core::mem::take(&mut self.tree.get_mut(x).rsrc), core::mem::take(&mut self.tree.get_mut(y).rsrc));
        self.tree.get_mut(x).rsrc = ry;
        self.tree.get_mut(y).rsrc = rx;
        let (fx, fy) = (self.tree.get(x).finfo, self.tree.get(y).finfo);
        self.tree.get_mut(x).finfo = fy;
        self.tree.get_mut(y).finfo = fx;
        Ok(())
    }
}
