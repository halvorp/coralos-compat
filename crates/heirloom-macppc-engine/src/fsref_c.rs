//! The HFS Plus File Manager's `FSRef` calls (Files.h, Mac OS 9.0 and CarbonLib) over the SAME volume tree the `FSSpec` / parameter-block calls
//! use (`heirloom_mac68k_engine::fm`): `FSpMakeFSRef`, `PBMakeFSRefSync`, `FSMakeFSRefUnicode`, `FSGetCatalogInfo`, `FSDeleteObject`, `FSCompareFSRefs`.
//!
//! An `FSRef` is 80 opaque bytes (Files.h) the program only stores and hands back. Here it is `"HlFR"` (0), the volume's vRefNum (4, a short),
//! the catalog node (8, a long) and zeros: a reference names an EXISTING object (fnfErr otherwise), and one whose object was deleted is
//! fnfErr when used (the node is no longer live, or now lives on another volume), never another object by accident of a reused slot it
//! cannot detect (a reused node on the same volume is the one hole; the File Manager's own FSRef has the same property for a reused node ID).
//!
//! `FSGetCatalogInfo` fills the `FSCatalogInfo` fields its `whichInfo` bitmap asks for (Files.h layout: nodeFlags 0, volume 2, parentDirID 4,
//! nodeID 8, sharingFlags 12, userPrivileges 13, createDate 16, contentModDate 24, attributeModDate 32, accessDate 40, backupDate 48,
//! permissions 56, finderInfo 72, extFinderInfo 88, data sizes 104 / 112, resource sizes 120 / 128, valence 136, textEncodingHint 140; 144
//! bytes); UTCDateTime is {highSeconds, lowSeconds, fraction} from the catalog's 1904-based seconds. `outName` is an HFSUniStr255 of the
//! name's characters as the volume holds them (one UTF-16 unit each).
use crate::clib::arg;
use crate::machine::MacPpc;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_mac68k_engine::fm::names::Located;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

const MAGIC: &[u8; 4] = b"HlFR";
const FSREF_SIZE: u32 = 80;
const CAT_INFO_SIZE: u32 = 144;
const FNF_ERR: i16 = -43;
const PARAM_ERR: i16 = -50;
const DIFF_ERR: i32 = -1302; // diffVolErr is for moves; FSCompareFSRefs answers this one
/// FSRefParam (Files.h): ioResult 16, ioNamePtr 18, ioVRefNum 22, ref 28, whichInfo 32, catInfo 36, nameLength 40, name 44, ioDirID 48,
/// spec 52, parentRef 56, newRef 60, textEncodingHint 64, outName 68.
const P_RESULT: u32 = 16;
const P_NAME_PTR: u32 = 18;
const P_VREF: u32 = 22;
const P_DIR_ID: u32 = 48;
const P_NEW_REF: u32 = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsrFn { FSpMakeFSRef, PBMakeFSRefSync, FSMakeFSRefUnicode, FSGetCatalogInfo, FSDeleteObject, FSCompareFSRefs, FSRefMakePath, FSPathMakeRef, FSSetCatalogInfo }

impl FsrFn {
    pub fn by_name(name: &str) -> Option<FsrFn> {
        use FsrFn::*;
        Some(match name {
            "FSpMakeFSRef" => FSpMakeFSRef, "PBMakeFSRefSync" => PBMakeFSRefSync, "FSMakeFSRefUnicode" => FSMakeFSRefUnicode, "FSGetCatalogInfo" => FSGetCatalogInfo,
            "FSDeleteObject" => FSDeleteObject, "FSCompareFSRefs" => FSCompareFSRefs, "FSRefMakePath" => FSRefMakePath, "FSPathMakeRef" => FSPathMakeRef, "FSSetCatalogInfo" => FSSetCatalogInfo,
            _ => return None,
        })
    }
}

/// UTCDateTime (8 bytes) of 1904-based seconds.
fn utc(secs: u32) -> [u8; 8] { let mut b = [0u8; 8]; b[2..6].copy_from_slice(&secs.to_be_bytes()); b }

impl<U: GuestUi> MacPpc<U> {
    fn fsr_err(&self, who: &str, at: u32) -> String { format!("{who}: {at:#x} leaves guest RAM") }

    /// Write the FSRef of `node` at `p`.
    fn put_fsref(&mut self, p: u32, node: usize) -> Result<(), String> {
        let vref = self.mac.fm.vref_of(self.mac.fm.tree.volume_of(node));
        let mut b = [0u8; FSREF_SIZE as usize];
        b[..4].copy_from_slice(MAGIC);
        b[4..6].copy_from_slice(&vref.to_be_bytes());
        b[8..12].copy_from_slice(&(node as u32).to_be_bytes());
        self.mac.mem.write_bytes(p, &b).map_err(|_| self.fsr_err("FSRef", p))
    }

    /// The node an FSRef at `p` names: paramErr when it is not one of ours, fnfErr when its object is gone.
    fn fsref_node(&self, p: u32) -> Result<usize, i16> {
        let b = self.mac.mem.span(p, 12).and_then(|s| self.mac.mem.bytes(s).map(<[u8]>::to_vec)).map_err(|_| PARAM_ERR)?;
        if &b[..4] != MAGIC { return Err(PARAM_ERR) }
        let (vref, node) = (i16::from_be_bytes([b[4], b[5]]), u32::from_be_bytes([b[8], b[9], b[10], b[11]]) as usize);
        let fm = &self.mac.fm;
        if !fm.tree.is_live(node) || fm.volume_of_vref(vref).ok() != Some(fm.tree.volume_of(node)) { return Err(FNF_ERR) }
        Ok(node)
    }

    /// The located object of (vRefNum, dirID, name): its node, or fnfErr.
    fn fsr_locate(&self, vref: i16, dir: u32, name: Option<&[u8]>) -> Result<usize, i16> {
        let l: Located = self.mac.fm.locate(vref, dir, name)?;
        l.node.ok_or(FNF_ERR)
    }

    pub(crate) fn fsr_call(&mut self, f: FsrFn, cpu: &mut Cpu) -> Result<(), String> {
        use FsrFn::*;
        let name = format!("{f:?}");
        let args: Vec<u32> = (0..6).map(|i| arg(self, cpu, i).unwrap_or(0)).collect();
        let r: i32 = match f {
            // FSpMakeFSRef(const FSSpec *source, FSRef *newRef)
            FSpMakeFSRef => match heirloom_mac68k_engine::fm::fsspec::spec_loc(&self.mac, args[0]).and_then(|l| l.node.ok_or(FNF_ERR)) {
                Ok(node) => { self.put_fsref(args[1], node)?; 0 }
                Err(e) => i32::from(e),
            },
            // PBMakeFSRefSync(FSRefParam *): ioVRefNum, ioDirID, ioNamePtr -> newRef; ioResult = the result
            PBMakeFSRefSync => {
                let pb = args[0];
                let m = &self.mac.mem;
                let (vref, dir, np) = (m.read_u16(pb + P_VREF).map_err(|_| self.fsr_err(&name, pb))? as i16, m.read_u32(pb + P_DIR_ID).map_err(|_| self.fsr_err(&name, pb))?, m.read_u32(pb + P_NAME_PTR).map_err(|_| self.fsr_err(&name, pb))?);
                let leaf = if np == 0 { None } else { Some(heirloom_mac68k_engine::pascal::read_pstring_bytes(m, np).map_err(|e| e.0)?) };
                let out = m.read_u32(pb + P_NEW_REF).map_err(|_| self.fsr_err(&name, pb))?;
                let e = match self.fsr_locate(vref, dir, leaf.as_deref()) { Ok(node) => { self.put_fsref(out, node)?; 0 } Err(e) => e };
                self.mac.mem.write_u16(pb + P_RESULT, e as u16).map_err(|_| self.fsr_err(&name, pb))?;
                i32::from(e)
            }
            // FSMakeFSRefUnicode(const FSRef *parentRef, UniCharCount nameLength, const UniChar *name, TextEncoding hint, FSRef *newRef)
            FSMakeFSRefUnicode => match self.fsref_node(args[0]) {
                Err(e) => i32::from(e),
                Ok(parent) if !self.mac.fm.tree.is_dir(parent) => i32::from(FNF_ERR),
                Ok(parent) => {
                    let mut leaf = Vec::with_capacity(args[1] as usize);
                    for i in 0..args[1] {
                        let u = self.mac.mem.read_u16(args[2] + 2 * i).map_err(|_| self.fsr_err(&name, args[2]))?;
                        leaf.push(u8::try_from(u).map_err(|_| format!("{name}: U+{u:04X} has no single-byte form on this volume (names are bytes)"))?);
                    }
                    let (vref, dir) = (self.mac.fm.vref_of(self.mac.fm.tree.volume_of(parent)), self.mac.fm.dir_id(parent));
                    match self.fsr_locate(vref, dir, Some(&leaf)) { Ok(node) => { self.put_fsref(args[4], node)?; 0 } Err(e) => i32::from(e) }
                }
            },
            // FSGetCatalogInfo(const FSRef *ref, FSCatalogInfoBitmap whichInfo, FSCatalogInfo *catalogInfo, HFSUniStr255 *outName, FSSpec *fsSpec, FSRef *parentRef)
            FSGetCatalogInfo => match self.fsref_node(args[0]) {
                Err(e) => i32::from(e),
                Ok(node) => { self.fsr_catalog_info(node, &args)?; 0 }
            },
            FSDeleteObject => match self.fsref_node(args[0]) {
                Err(e) => i32::from(e),
                Ok(node) => {
                    let fm = &self.mac.fm;
                    if fm.tree.is_dir(node) && !fm.tree.children(node).is_empty() { -47 } else { // fBsyErr: a folder with items is not deleted
                        let loc = Located { vol: fm.tree.volume_of(node), parent: fm.tree.parent(node), leaf: None, node: Some(node) };
                        self.mac.fm.delete(&loc).err().map_or(0, i32::from)
                    }
                }
            },
            // FSRefMakePath(ref, path, maxPathSize): Mac OS 9's CarbonLib 1.4 makes the object's HFS FULL PATH (read off its FSRefMakePath: the FSSpec,
            // then the parent chain, ':'-joined, a folder ending in ':'), NUL-terminated; one that does not fit is -210, CarbonLib's answer there
            FSRefMakePath => match self.fsref_node(args[0]) {
                Err(e) => i32::from(e),
                Ok(node) => {
                    let mut p = self.mac.fm.full_path(node);
                    if p.len() + 1 > args[2] as usize { -210 } else {
                        p.push(0);
                        self.mac.mem.write_bytes(args[1], &p).map_err(|_| self.fsr_err(&name, args[1]))?;
                        0
                    }
                }
            },
            // FSPathMakeRef(path, &ref, &isDirectory): the inverse: a NUL-terminated full path (Volume:a:b) to the object's FSRef
            FSPathMakeRef => {
                let path = self.cstr(args[0], &name)?;
                match self.mac.fm.locate(0, 0, Some(&path)).and_then(|l| l.node.ok_or(FNF_ERR)) {
                    Err(e) => i32::from(e),
                    Ok(node) => {
                        self.put_fsref(args[1], node)?;
                        if args[2] != 0 { self.mac.mem.write_u8(args[2], u8::from(self.mac.fm.tree.is_dir(node))).map_err(|_| self.fsr_err(&name, args[2]))?; }
                        0
                    }
                }
            }
            // FSSetCatalogInfo(ref, whichInfo, catalogInfo): the settable fields of an HFS volume - nodeFlags' lock bit (0x2), createDate (0x20),
            // contentModDate (0x40), finderInfo (0x800: type, creator, flags, location); the rest (permissions, the other dates, extFinderInfo)
            // has nowhere to go on this volume and is named in the log, as Mac OS 9 ignores what HFS does not keep
            FSSetCatalogInfo => match self.fsref_node(args[0]) {
                Err(e) => i32::from(e),
                Ok(node) => {
                    let (which, cat) = (args[1], args[2]);
                    let rd = |s: &Self, o: u32| s.mac.mem.read_u32(cat + o).map_err(|_| s.fsr_err("FSSetCatalogInfo catalogInfo", cat));
                    let info = self.mac.fm.cat_info(node);
                    let mut finfo = info.finfo;
                    if which & 0x0800 != 0 && !info.is_dir {
                        finfo.file_type = rd(self, 72)?.to_be_bytes();
                        finfo.creator = rd(self, 76)?.to_be_bytes();
                        let w = rd(self, 80)?;
                        finfo.flags = (w >> 16) as u16;
                        finfo.v = w as u16 as i16;
                        finfo.h = (rd(self, 84)? >> 16) as u16 as i16;
                    }
                    let created = if which & 0x0020 != 0 { rd(self, 18)? } else { info.created };
                    let modified = if which & 0x0040 != 0 { rd(self, 26)? } else { info.modified };
                    let mut e = self.mac.fm.set_cat(node, Some(finfo), created, modified).err().map_or(0, i32::from);
                    if e == 0 && which & 0x0002 != 0 {
                        let lock = self.mac.mem.read_u16(cat).map_err(|_| self.fsr_err("FSSetCatalogInfo catalogInfo", cat))? & 1 != 0;
                        let fm = &self.mac.fm;
                        let loc = Located { vol: fm.tree.volume_of(node), parent: fm.tree.parent(node), leaf: None, node: Some(node) };
                        e = self.mac.fm.set_lock(&loc, lock).err().map_or(0, i32::from);
                    }
                    let ignored = which & !(0x0002 | 0x0020 | 0x0040 | 0x0800);
                    if ignored != 0 { self.mac.log.push(format!("[file] FSSetCatalogInfo: whichInfo bits {ignored:#x} have no place on this volume (not kept)")); }
                    e
                }
            },
            // FSCompareFSRefs(ref1, ref2): noErr when both name one object, diffVolErr on two volumes, errFSRefsDifferent (-1420) otherwise
            FSCompareFSRefs => match (self.fsref_node(args[0]), self.fsref_node(args[1])) {
                (Err(e), _) | (_, Err(e)) => i32::from(e),
                (Ok(a), Ok(b)) if a == b => 0,
                (Ok(a), Ok(b)) if self.mac.fm.tree.volume_of(a) != self.mac.fm.tree.volume_of(b) => DIFF_ERR,
                _ => -1420,
            },
        };
        cpu.gpr[3] = r as u32;
        Ok(())
    }

    fn fsr_catalog_info(&mut self, node: usize, args: &[u32]) -> Result<(), String> {
        let (which, cat, out_name, spec, parent_ref) = (args[1], args[2], args[3], args[4], args[5]);
        let fm = &self.mac.fm;
        let info = fm.cat_info(node);
        let vref = fm.vref_of(fm.tree.volume_of(node));
        if cat != 0 {
            let mut b = Vec::new();
            b.resize(CAT_INFO_SIZE as usize, 0);
            let put = |b: &mut Vec<u8>, at: usize, v: &[u8]| b[at..at + v.len()].copy_from_slice(v);
            let flags: u16 = (if info.attrib & 1 != 0 { 0x0001 } else { 0 }) | (if info.is_dir { 0x0010 } else { 0 }) | (if info.attrib & 0x80 != 0 { 0x0080 } else { 0 });
            if which & 0x0002 != 0 { put(&mut b, 0, &flags.to_be_bytes()); }
            if which & 0x0004 != 0 { put(&mut b, 2, &vref.to_be_bytes()); }
            if which & 0x0008 != 0 { put(&mut b, 4, &info.parent_id.to_be_bytes()); }
            if which & 0x0010 != 0 { put(&mut b, 8, &info.id.to_be_bytes()); }
            if which & 0x0020 != 0 { put(&mut b, 16, &utc(info.created)); }
            if which & 0x0040 != 0 { put(&mut b, 24, &utc(info.modified)); }
            if which & 0x0080 != 0 { put(&mut b, 32, &utc(info.modified)); }
            if which & 0x0100 != 0 { put(&mut b, 40, &utc(info.modified)); }
            if which & 0x0800 != 0 {
                let f = &info.finfo;
                let mut fi = [0u8; 16];
                if !info.is_dir { fi[..4].copy_from_slice(&f.file_type); fi[4..8].copy_from_slice(&f.creator); fi[8..10].copy_from_slice(&f.flags.to_be_bytes()); }
                put(&mut b, 72, &fi);
            }
            if which & 0x4000 != 0 && !info.is_dir { put(&mut b, 108, &info.lg.to_be_bytes()); put(&mut b, 116, &info.py.to_be_bytes()); }
            if which & 0x8000 != 0 && !info.is_dir { put(&mut b, 124, &info.rlg.to_be_bytes()); put(&mut b, 132, &info.rpy.to_be_bytes()); }
            if which & 0x0001_0000 != 0 && info.is_dir { put(&mut b, 136, &u32::from(info.items).to_be_bytes()); }
            self.mac.mem.write_bytes(cat, &b).map_err(|_| self.fsr_err("FSGetCatalogInfo catalogInfo", cat))?;
        }
        if out_name != 0 {
            let mut u = Vec::with_capacity(2 + 2 * info.name.len());
            u.extend_from_slice(&(info.name.len() as u16).to_be_bytes());
            for &c in &info.name { u.extend_from_slice(&u16::from(c).to_be_bytes()); }
            self.mac.mem.write_bytes(out_name, &u).map_err(|_| self.fsr_err("FSGetCatalogInfo outName", out_name))?;
        }
        if spec != 0 {
            let fm = &self.mac.fm;
            let loc = Located { vol: fm.tree.volume_of(node), parent: fm.tree.parent(node), leaf: None, node: Some(node) };
            heirloom_mac68k_engine::fm::fsspec::write_spec(&mut self.mac, spec, &loc).map_err(|e| format!("FSGetCatalogInfo fsSpec: OSErr {e}"))?;
        }
        if parent_ref != 0 {
            // a volume's root has no parent object: its parentRef is zeroed, as the File Manager does for the root
            if self.mac.fm.tree.is_volume(node) { self.mac.mem.zero(parent_ref, FSREF_SIZE).map_err(|_| self.fsr_err("FSGetCatalogInfo parentRef", parent_ref))?; }
            else { let p = self.mac.fm.tree.parent(node); self.put_fsref(parent_ref, p)?; }
        }
        Ok(())
    }
}
