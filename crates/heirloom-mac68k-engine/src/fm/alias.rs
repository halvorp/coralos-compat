//! The Alias Manager and the Folder Manager's `FindFolder`, both reached through `_AliasDispatch` ($A823, selector in D0.W,
//! the routines' Pascal arguments on the stack; Inside Macintosh: Files, chapter 4, and Volume VI, Folder Manager).
//!
//! Selectors (Aliases.h / Folders.h as the Universal Interfaces number them): 0 FindFolder, 2 NewAlias, 3 ResolveAlias,
//! 6 UpdateAlias, 7 GetAliasInfo, 8 NewAliasMinimal, 9 NewAliasMinimalFromFullPath, 12 ResolveAliasFile. Any other selector
//! (MatchAlias, FollowFinderAlias, ...) halts the guest with a named refusal: its frame size is unknown.
//!
//! **What an alias is here.** `AliasRecord` is opaque to programs; this one is the documented fixed part (userType, size,
//! version 2, kind, volume name, file name, file number, file type / creator, dates) followed by tagged variable data with
//! the ABSOLUTE PATH (tag 2) and the end tag. `ResolveAlias` finds the target by that path (the volume, then the folders),
//! and falls back to the file name in the recorded parent directory; a target that is gone answers `fnfErr` (-43) and leaves
//! the FSSpec alone; a moved target found by the fallback sets `wasChanged`. There is no network: AppleShare zone / server
//! info is always empty, and a record from another Alias Manager is read by its fixed part and its path tag only.
//!
//! **Folders.** The System Folder and its standard subfolders are created on the boot volume the first time a program asks
//! for them (a real Mac has them from the start), so `FindFolder` answers for the Preferences, Extensions, Control Panels,
//! Apple Menu, Startup, Fonts and Temporary folders, the per-volume Trash and Desktop Folder, and the System Folder itself.
//! A type it does not know is `fnfErr`.
use super::fsspec::{spec_loc, write_spec};
use super::names::Located;
use super::{bytes_of, err, name_of, FileMgr, BOOT_VOLUME};
use crate::machine::Mac;
use crate::osconv::Regs;
use crate::pascal::{self, Arg};
use crate::Refusal;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use heirloom_ui_abi::GuestUi;
use heirloom_cpu_68k::Cpu;

pub mod sel {
    pub const FIND_FOLDER: u16 = 0;
    pub const NEW_ALIAS: u16 = 2;
    pub const RESOLVE_ALIAS: u16 = 3;
    pub const UPDATE_ALIAS: u16 = 6;
    pub const GET_ALIAS_INFO: u16 = 7;
    pub const NEW_ALIAS_MINIMAL: u16 = 8;
    pub const NEW_ALIAS_MINIMAL_FROM_FULL_PATH: u16 = 9;
    pub const RESOLVE_ALIAS_FILE: u16 = 12;
}

const W: Arg = Arg::W;
const L: Arg = Arg::L;
const B: Arg = Arg::B;
/// The routine's arguments in declaration order (all are functions returning an OSErr).
fn layout(selector: u16) -> Option<&'static [Arg]> {
    Some(match selector {
        sel::FIND_FOLDER => &[W, L, B, L, L],
        sel::NEW_ALIAS => &[L, L, L],
        sel::GET_ALIAS_INFO => &[L, W, L],
        sel::RESOLVE_ALIAS | sel::UPDATE_ALIAS => &[L, L, L, L],
        sel::NEW_ALIAS_MINIMAL => &[L, L],
        sel::NEW_ALIAS_MINIMAL_FROM_FULL_PATH => &[W, L, L, L, L],
        sel::RESOLVE_ALIAS_FILE => &[L, B, L, L],
        _ => return None,
    })
}

// ---- the AliasRecord layout (Inside Macintosh: Files 4-43) ---------------------------------------------------------------
const AR_USER_TYPE: u32 = 0;
const AR_SIZE: u32 = 4;
const AR_VERSION: u32 = 6;
const AR_KIND: u32 = 8;
const AR_VOLUME_NAME: u32 = 10; // Str27
const AR_VOLUME_CREATED: u32 = 38;
const AR_VOLUME_SIG: u32 = 42;
const AR_PARENT_DIR: u32 = 46;
const AR_FILE_NAME: u32 = 50; // Str63
const AR_FILE_NUMBER: u32 = 114;
const AR_FILE_CREATED: u32 = 118;
const AR_FILE_TYPE: u32 = 122;
const AR_FILE_CREATOR: u32 = 126;
const AR_VARIABLE: u32 = 150;
const TAG_PATH: i16 = 2;
const TAG_END: i16 = -1;

/// The standard folders: (Folder Manager type, name). `macs` is the System Folder; the others live inside it.
const SYSTEM_SUB: [(&[u8; 4], &str); 8] = [
    (b"pref", "Preferences"), (b"extn", "Extensions"), (b"ctrl", "Control Panels"), (b"amnu", "Apple Menu Items"),
    (b"strt", "Startup Items"), (b"font", "Fonts"), (b"temp", "Temporary Items"), (b"prnt", "PrintMonitor Documents"),
];

impl FileMgr {
    /// The boot volume's System Folder with every standard subfolder: a real Mac has them from the start.
    pub fn install_system_folder(&mut self) {
        for (ty, _) in SYSTEM_SUB { let _ = self.find_folder(0, *ty); }
    }

    /// The working-directory reference number of the boot volume's System Folder: `SysEnvirons`' `sysVRefNum`.
    pub fn system_folder_wd(&mut self) -> Result<i16, i16> {
        let (vref, dir_id) = self.find_folder(0, *b"macs")?;
        let vol = self.volume_of_vref(vref)?;
        let dir = self.dir_node(vol, dir_id)?;
        Ok(self.wd_for(vol, dir))
    }

    /// `FindFolder`: the (vRefNum, dirID) of a standard folder, made on first use. `vref` 0 / -32768 (kOnSystemDisk) is the boot volume.
    pub fn find_folder(&mut self, vref: i16, folder_type: [u8; 4]) -> Result<(i16, u32), i16> {
        let vol = if vref == 0 || vref == i16::MIN { self.tree.volume(BOOT_VOLUME).unwrap_or(self.def_vol) } else { self.volume_of_vref(vref)? };
        let make = |fm: &mut FileMgr, parent, name: &str| -> Result<_, i16> {
            match fm.tree.find_child(parent, name) {
                Some(d) if fm.tree.is_dir(d) => Ok(d),
                Some(_) => Err(err::DUP_FN_ERR),
                None => { let l = Located { vol, parent, leaf: Some(name.into()), node: None }; fm.create_dir(&l) }
            }
        };
        let dir = match &folder_type {
            b"trsh" => make(self, vol, "Trash")?,
            b"desk" => make(self, vol, "Desktop Folder")?,
            b"macs" => make(self, vol, "System Folder")?,
            t => {
                let (_, name) = SYSTEM_SUB.iter().find(|(k, _)| *k == t).ok_or(err::FNF_ERR)?;
                let sys = make(self, vol, "System Folder")?;
                make(self, sys, name)?
            }
        };
        Ok((self.vref_of(vol), self.dir_id(dir)))
    }

    /// The full path `Volume:folder:...:name` of a node.
    pub fn full_path(&self, node: heirloom_volume::NodeId) -> Vec<u8> {
        let mut p = bytes_of(&self.tree.path_components(node).join(":"));
        if self.tree.is_dir(node) { p.push(b':'); }
        if self.tree.is_volume(node) { p.push(b':'); }
        p
    }
}

fn put_pstr<U: GuestUi>(m: &mut Mac<U>, at: u32, s: &[u8], cap: usize) -> Result<(), i16> {
    let n = s.len().min(cap);
    m.mem.write_u8(at, n as u8).and_then(|_| m.mem.write_bytes(at + 1, &s[..n])).map_err(|_| err::PARAM_ERR)
}

impl<U: GuestUi> Mac<U> {
    /// Build an AliasRecord for `target` as seen from `from` (if any), in a new handle.
    fn make_alias(&mut self, target: &Located) -> Result<u32, i16> {
        let node = target.node.ok_or(err::FNF_ERR)?;
        let vol = target.vol;
        let info = self.fm.cat_info(node);
        let path = self.fm.full_path(node);
        let mut rec = alloc::vec![0u8; AR_VARIABLE as usize];
        rec[AR_USER_TYPE as usize..][..4].copy_from_slice(b"\0\0\0\0");
        rec[AR_VERSION as usize..][..2].copy_from_slice(&2i16.to_be_bytes());
        rec[AR_KIND as usize..][..2].copy_from_slice(&(info.is_dir as i16).to_be_bytes());
        let vn = bytes_of(self.fm.tree.name(vol));
        rec[AR_VOLUME_NAME as usize] = vn.len().min(27) as u8;
        rec[AR_VOLUME_NAME as usize + 1..][..vn.len().min(27)].copy_from_slice(&vn[..vn.len().min(27)]);
        rec[AR_VOLUME_CREATED as usize..][..4].copy_from_slice(&info.created.to_be_bytes());
        rec[AR_VOLUME_SIG as usize..][..2].copy_from_slice(&0x4244u16.to_be_bytes()); // 'BD': HFS
        rec[AR_PARENT_DIR as usize..][..4].copy_from_slice(&info.parent_id.to_be_bytes());
        rec[AR_FILE_NAME as usize] = info.name.len().min(63) as u8;
        rec[AR_FILE_NAME as usize + 1..][..info.name.len().min(63)].copy_from_slice(&info.name[..info.name.len().min(63)]);
        rec[AR_FILE_NUMBER as usize..][..4].copy_from_slice(&info.id.to_be_bytes());
        rec[AR_FILE_CREATED as usize..][..4].copy_from_slice(&info.created.to_be_bytes());
        rec[AR_FILE_TYPE as usize..][..4].copy_from_slice(&info.finfo.file_type);
        rec[AR_FILE_CREATOR as usize..][..4].copy_from_slice(&info.finfo.creator);
        let padded = (path.len() + 1) & !1;
        rec.extend_from_slice(&TAG_PATH.to_be_bytes());
        rec.extend_from_slice(&(path.len() as i16).to_be_bytes());
        rec.extend_from_slice(&path);
        rec.resize(rec.len() + padded - path.len(), 0);
        rec.extend_from_slice(&TAG_END.to_be_bytes());
        rec.extend_from_slice(&0i16.to_be_bytes());
        let size = rec.len() as u16;
        rec[AR_SIZE as usize..][..2].copy_from_slice(&size.to_be_bytes());
        let h = self.mm.new_handle(&mut self.mem, &mut self.heap, rec.len() as u32, false)?;
        let p = self.mm.block(h).map(|b| b.ptr).ok_or(err::PARAM_ERR)?;
        self.mem.write_bytes(p, &rec).map_err(|_| err::PARAM_ERR)?;
        Ok(h)
    }

    /// The target an AliasRecord names, and whether it was found somewhere other than its recorded path.
    fn resolve_record(&self, rec: &[u8]) -> Result<(Located, bool), i16> {
        if rec.len() < AR_VARIABLE as usize || &rec[AR_VERSION as usize..][..2] != 2i16.to_be_bytes() { return Err(err::PARAM_ERR); }
        let mut path: Option<&[u8]> = None;
        let mut at = AR_VARIABLE as usize;
        while at + 4 <= rec.len() {
            let (tag, len) = (i16::from_be_bytes([rec[at], rec[at + 1]]), i16::from_be_bytes([rec[at + 2], rec[at + 3]]).max(0) as usize);
            if tag == TAG_END { break; }
            if tag == TAG_PATH { path = rec.get(at + 4..at + 4 + len); }
            at += 4 + ((len + 1) & !1);
        }
        if let Some(p) = path {
            if let Ok(l) = self.fm.locate(0, 0, Some(p)) { if l.node.is_some() { return Ok((l, false)); } }
        }
        // Moved: the same name under the recorded parent directory of the recorded volume.
        let vn = &rec[AR_VOLUME_NAME as usize + 1..][..(rec[AR_VOLUME_NAME as usize] as usize).min(27)];
        let name = &rec[AR_FILE_NAME as usize + 1..][..(rec[AR_FILE_NAME as usize] as usize).min(63)];
        let vol = self.fm.tree.volume(&name_of(vn)).ok_or(err::NSV_ERR)?;
        let parent = u32::from_be_bytes(rec[AR_PARENT_DIR as usize..][..4].try_into().expect("4 bytes"));
        let l = self.fm.locate(self.fm.vref_of(vol), parent, Some(name))?;
        if l.node.is_some() { Ok((l, true)) } else { Err(err::FNF_ERR) }
    }

    fn alias_bytes(&self, h: u32) -> Result<Vec<u8>, i16> { self.handle_bytes(h).ok_or(err::PARAM_ERR) }
}

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> { m.register_os(0xA823, dispatch::<U>) }

fn dispatch<U: GuestUi>(m: &mut Mac<U>, c: &mut Cpu, r: &mut Regs) -> Result<String, Refusal> {
    let selector = r.d0 as u16;
    let Some(lay) = layout(selector) else {
        c.halt();
        return Err(Refusal(format!("AliasDispatch selector {selector} is not modelled (the guest is stopped: its frame size is unknown)")));
    };
    let sp = c.areg(7);
    let args = pascal::read_args(&m.mem, sp, lay)?;
    m.fm.now_secs = m.clock();
    let (e, log) = match alias_call(m, selector, &|i| args.get(i)) {
        Ok(log) => (0, log),
        Err((e, log)) => (e, log),
    };
    let new_sp = pascal::complete(&mut m.mem, sp, lay, Some((Arg::W, e as u16 as u32)))?;
    c.set_areg(7, new_sp);
    r.d0 = 0;
    Ok(format!("[alias] {log} -> {e}"))
}

fn alias_call<U: GuestUi>(m: &mut Mac<U>, selector: u16, a: &dyn Fn(usize) -> u32) -> Result<String, (i16, String)> {
    let bad = |e: i16| (e, format!("selector {selector}"));
    let put_b = |m: &mut Mac<U>, p: u32, v: bool| if p == 0 { Ok(()) } else { m.mem.write_u8(p, u8::from(v)).map_err(|_| err::PARAM_ERR) };
    match selector {
        sel::FIND_FOLDER => {
            let (vref, ty, create) = (a(0) as u16 as i16, (a(1)).to_be_bytes(), a(2) as u8 != 0);
            let _ = create; // the folders exist (made on first use), so createFolder changes nothing
            let (v, d) = m.fm.find_folder(vref, ty).map_err(|e| (e, format!("FindFolder '{}'", name_of(&ty))))?;
            m.mem.write_u16(a(3), v as u16).and_then(|_| m.mem.write_u32(a(4), d)).map_err(|_| bad(err::PARAM_ERR))?;
            Ok(format!("FindFolder '{}' -> vRef {v} dirID {d}", name_of(&ty)))
        }
        sel::NEW_ALIAS | sel::NEW_ALIAS_MINIMAL => {
            let (target, out) = if selector == sel::NEW_ALIAS { (a(1), a(2)) } else { (a(0), a(1)) };
            let l = spec_loc(m, target).map_err(bad)?;
            if l.node.is_none() { return Err((err::FNF_ERR, "NewAlias: the target does not exist".into())); }
            let h = m.make_alias(&l).map_err(bad)?;
            m.mem.write_u32(out, h).map_err(|_| bad(err::PARAM_ERR))?;
            Ok(format!("NewAlias -> {h:#x}"))
        }
        sel::NEW_ALIAS_MINIMAL_FROM_FULL_PATH => {
            let (n, p) = (a(0) as u16 as usize, a(1));
            let path = m.mem.span(p, n as u32).and_then(|s| m.mem.bytes(s).map(<[u8]>::to_vec)).map_err(|_| bad(err::PARAM_ERR))?;
            let l = m.fm.locate(0, 0, Some(&path)).map_err(bad)?;
            if l.node.is_none() { return Err((err::FNF_ERR, "NewAliasMinimalFromFullPath: no such file".into())); }
            let h = m.make_alias(&l).map_err(bad)?;
            m.mem.write_u32(a(4), h).map_err(|_| bad(err::PARAM_ERR))?;
            Ok(format!("NewAliasMinimalFromFullPath -> {h:#x}"))
        }
        sel::RESOLVE_ALIAS | sel::UPDATE_ALIAS => {
            let update = selector == sel::UPDATE_ALIAS;
            let (alias, spec, changed) = if update { (a(2), a(1), a(3)) } else { (a(1), a(2), a(3)) };
            let rec = m.alias_bytes(alias).map_err(bad)?;
            if update {
                let l = spec_loc(m, spec).map_err(bad)?;
                if l.node.is_none() { return Err((err::FNF_ERR, "UpdateAlias: the target does not exist".into())); }
                let h = m.make_alias(&l).map_err(bad)?;
                let fresh = m.alias_bytes(h).map_err(bad)?;
                m.set_handle_contents(alias, &fresh).map_err(bad)?;
                let _ = m.dispose_handle(h);
                put_b(m, changed, fresh != rec).map_err(bad)?;
                return Ok("UpdateAlias".into());
            }
            let (l, moved) = m.resolve_record(&rec).map_err(|e| (e, "ResolveAlias: the target is gone".into()))?;
            write_spec(m, spec, &l).map_err(bad)?;
            put_b(m, changed, moved).map_err(bad)?;
            Ok(format!("ResolveAlias -> {}{}", name_of(&m.fm.full_path(l.node.expect("found"))), if moved { " (moved)" } else { "" }))
        }
        sel::GET_ALIAS_INFO => {
            let (alias, index, out) = (a(0), a(1) as u16 as i16, a(2));
            let rec = m.alias_bytes(alias).map_err(bad)?;
            if rec.len() < AR_VARIABLE as usize { return Err(bad(err::PARAM_ERR)); }
            let pstr = |off: u32, cap: usize| rec[off as usize + 1..][..(rec[off as usize] as usize).min(cap)].to_vec();
            // asiAliasName 0, asiParentName 1, asiVolumeName -1, asiZoneName -3, asiServerName -2
            let s = match index { 0 => pstr(AR_FILE_NAME, 63), -1 => pstr(AR_VOLUME_NAME, 27), -2 | -3 => Vec::new(), _ => return Err((err::PARAM_ERR, format!("GetAliasInfo index {index}"))) };
            put_pstr(m, out, &s, 63).map_err(bad)?;
            Ok(format!("GetAliasInfo {index} = \"{}\"", name_of(&s)))
        }
        sel::RESOLVE_ALIAS_FILE => {
            // A file that is not an alias (Finder flag kIsAlias, 0x8000) resolves to itself: wasAliased FALSE.
            let (spec, chains, is_folder, was_aliased) = (a(0), a(1) as u8 != 0, a(2), a(3));
            let mut l = spec_loc(m, spec).map_err(bad)?;
            let mut aliased = false;
            for _ in 0..if chains { 32 } else { 1 } {
                let Some(node) = l.node else { break };
                if m.fm.tree.is_dir(node) || m.fm.tree.get(node).finfo.flags & 0x8000 == 0 { break; }
                let rec = m.alias_resource(node).ok_or_else(|| (err::FNF_ERR, "ResolveAliasFile: an alias file without its 'alis' resource".to_string()))?;
                l = m.resolve_record(&rec).map_err(|e| (e, "ResolveAliasFile: the target is gone".into()))?.0;
                aliased = true;
            }
            if aliased { write_spec(m, spec, &l).map_err(bad)?; }
            let dir = l.node.is_some_and(|n| m.fm.tree.is_dir(n));
            put_b(m, is_folder, dir).and_then(|_| put_b(m, was_aliased, aliased)).map_err(bad)?;
            Ok(format!("ResolveAliasFile aliased={aliased}"))
        }
        _ => Err(bad(err::PARAM_ERR)),
    }
}

impl<U: GuestUi> Mac<U> {
    /// The data of resource `'alis'` 0 in an alias file's resource fork (where the Finder keeps the record).
    fn alias_resource(&self, node: heirloom_volume::NodeId) -> Option<Vec<u8>> {
        let fork = heirloom_fmt_resfork::ResFork::parse(self.fm.tree.get(node).rsrc.clone()).ok()?;
        fork.find(*b"alis", 0).and_then(|i| fork.data(i)).map(<[u8]>::to_vec)
    }

    /// Replace a handle's bytes (resizing it).
    fn set_handle_contents(&mut self, h: u32, bytes: &[u8]) -> Result<(), i16> {
        self.mm.set_handle_size(&mut self.mem, &mut self.heap, h, bytes.len() as u32)?;
        let p = self.mm.block(h).map(|b| b.ptr).ok_or(err::PARAM_ERR)?;
        self.mem.write_bytes(p, bytes).map_err(|_| err::PARAM_ERR)
    }
}

/// Run `_AliasDispatch` routine `selector` with its Pascal arguments as VALUES (declaration order), as the PowerPC adapter does: the SAME body the 68k trap runs, no stack frame.
/// The OSErr it answers (0 = noErr); a selector this engine does not model is a refusal.
pub fn alias_value_call<U: GuestUi>(m: &mut Mac<U>, selector: u16, values: &[u32]) -> Result<i16, Refusal> {
    let lay = layout(selector).ok_or_else(|| Refusal(format!("AliasDispatch selector {selector} is not modelled")))?;
    if lay.len() != values.len() { return Err(Refusal(format!("AliasDispatch selector {selector} takes {} arguments, {} given", lay.len(), values.len()))); }
    m.fm.now_secs = m.clock();
    let (e, log) = match alias_call(m, selector, &|i| values[i]) { Ok(log) => (0, log), Err((e, log)) => (e, log) };
    m.log.push(format!("[alias] {log} -> {e}"));
    Ok(e)
}
