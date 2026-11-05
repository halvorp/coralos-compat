//! The HFS Plus File Manager's `FSRef` calls (Files.h, Mac OS 9.0 and CarbonLib) over the SAME volume tree the `FSSpec` / parameter-block calls
//! use (`heirloom_mac68k_engine::fm`): `FSpMakeFSRef`, `PBMakeFSRefSync`, `FSMakeFSRefUnicode`, `FSCreateDirectoryUnicode`, `FSCreateFileUnicode`, `FSGetCatalogInfo`, `FSDeleteObject`,
//! `FSCompareFSRefs`.
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
//! name's Mac Roman bytes as Unicode (`heirloom_mac68k_engine::macroman`, the one table; every Mac Roman character is one UTF-16 unit), and
//! FSMakeFSRefUnicode's name goes the other way (a character Mac Roman lacks cannot name an item on these volumes: refused by name).
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
pub enum FsrFn { FSpMakeFSRef, PBMakeFSRefSync, FSMakeFSRefUnicode, FSGetCatalogInfo, FSDeleteObject, FSCompareFSRefs, FSRefMakePath, FSPathMakeRef, FSSetCatalogInfo, FSOpenIterator, FSCloseIterator, FSGetCatalogInfoBulk, PBGetCatalogInfoSync, FSOpenResFile, FSGetVolumeInfo, FSCreateDirectoryUnicode, FSCreateFileUnicode }

impl FsrFn {
    pub fn by_name(name: &str) -> Option<FsrFn> {
        use FsrFn::*;
        Some(match name {
            "FSpMakeFSRef" => FSpMakeFSRef, "PBMakeFSRefSync" => PBMakeFSRefSync, "FSMakeFSRefUnicode" => FSMakeFSRefUnicode, "FSCreateDirectoryUnicode" => FSCreateDirectoryUnicode, "FSCreateFileUnicode" => FSCreateFileUnicode, "FSGetCatalogInfo" => FSGetCatalogInfo, "PBGetCatalogInfoSync" => PBGetCatalogInfoSync, "FSOpenResFile" => FSOpenResFile,
            "FSDeleteObject" => FSDeleteObject, "FSCompareFSRefs" => FSCompareFSRefs, "FSRefMakePath" => FSRefMakePath, "FSPathMakeRef" => FSPathMakeRef, "FSSetCatalogInfo" => FSSetCatalogInfo,
            "FSOpenIterator" => FSOpenIterator, "FSCloseIterator" => FSCloseIterator, "FSGetCatalogInfoBulk" => FSGetCatalogInfoBulk, "FSGetVolumeInfo" => FSGetVolumeInfo,
            _ => return None,
        })
    }
}

/// errFSNoMoreItems, errFSBadIteratorFlags, errFSBadItemCount, errFSBadIterator (Files.h).
const NO_MORE_ITEMS: i32 = -1417;
const BAD_ITERATOR_FLAGS: i32 = -1423;
const BAD_ITEM_COUNT: i32 = -1424;
const BAD_ITERATOR: i32 = -1420;
const HFS_UNI_STR_SIZE: u32 = 512;
const FSSPEC_SIZE: u32 = 70;

/// Open catalog iterators (`FSOpenIterator`): id -> (the folder, its items as they were at open, how many were returned). A growable table;
/// an iterator id is never reused.
#[derive(Default)]
pub struct FsrState { iters: alloc::collections::BTreeMap<u32, (usize, Vec<usize>, usize)>, next: u32 }

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

    /// The FSSetCatalogInfo fields of `which` from the FSCatalogInfo at `cat` onto `node`: the result code.
    fn fsr_set_cat(&mut self, node: usize, which: u32, cat: u32) -> Result<i32, String> {
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
        Ok(e)
    }

    /// A Unicode name of `len` UniChars at `p` as Mac Roman bytes (names on these volumes are Mac Roman; a character it lacks is refused by name).
    fn unicode_leaf(&self, who: &str, p: u32, len: u32) -> Result<Vec<u8>, String> {
        let mut leaf = Vec::with_capacity(len as usize);
        for i in 0..len {
            let u = self.mac.mem.read_u16(p + 2 * i).map_err(|_| self.fsr_err(who, p))?;
            leaf.push(char::from_u32(u32::from(u)).and_then(heirloom_mac68k_engine::macroman::from_char).ok_or_else(|| format!("{who}: U+{u:04X} has no Mac Roman byte (names on these volumes are Mac Roman)"))?);
        }
        Ok(leaf)
    }

    pub(crate) fn fsr_call(&mut self, f: FsrFn, cpu: &mut Cpu) -> Result<(), String> {
        use FsrFn::*;
        let name = format!("{f:?}");
        let args: Vec<u32> = (0..9).map(|i| arg(self, cpu, i).unwrap_or(0)).collect();
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
                    let leaf = self.unicode_leaf(&name, args[2], args[1])?;
                    let (vref, dir) = (self.mac.fm.vref_of(self.mac.fm.tree.volume_of(parent)), self.mac.fm.dir_id(parent));
                    match self.fsr_locate(vref, dir, Some(&leaf)) { Ok(node) => { self.put_fsref(args[4], node)?; 0 } Err(e) => i32::from(e) }
                }
            },
            // FSCreateDirectoryUnicode(parentRef, nameLength, name, whichInfo, catalogInfo, newRef, newSpec, newDirID)
            // FSCreateFileUnicode(parentRef, nameLength, name, whichInfo, catalogInfo, newRef, newSpec): the File Manager's DirCreate /
            // Create under a Unicode name (Mac Roman on these volumes), then FSSetCatalogInfo's fields; dupFNErr when it exists
            FSCreateDirectoryUnicode | FSCreateFileUnicode => match self.fsref_node(args[0]) {
                Err(e) => i32::from(e),
                Ok(parent) if !self.mac.fm.tree.is_dir(parent) => -120, // dirNFErr
                Ok(parent) => {
                    let leaf = self.unicode_leaf(&name, args[2], args[1])?;
                    let (vref, dir) = (self.mac.fm.vref_of(self.mac.fm.tree.volume_of(parent)), self.mac.fm.dir_id(parent));
                    let made = self.mac.fm.locate(vref, dir, Some(&leaf)).and_then(|loc| if f == FSCreateDirectoryUnicode { self.mac.fm.create_dir(&loc) } else { self.mac.fm.create_file(&loc) });
                    match made {
                        Err(e) => i32::from(e),
                        Ok(node) => {
                            let e = if args[3] != 0 && args[4] != 0 { self.fsr_set_cat(node, args[3], args[4])? } else { 0 };
                            if args[5] != 0 { self.put_fsref(args[5], node)?; }
                            if args[6] != 0 {
                                let fm = &self.mac.fm;
                                let loc = Located { vol: fm.tree.volume_of(node), parent: fm.tree.parent(node), leaf: None, node: Some(node) };
                                heirloom_mac68k_engine::fm::fsspec::write_spec(&mut self.mac, args[6], &loc).map_err(|e| format!("{name} newSpec: OSErr {e}"))?;
                            }
                            if f == FSCreateDirectoryUnicode && args[7] != 0 { let id = self.mac.fm.dir_id(node); self.mac.mem.write_u32(args[7], id).map_err(|_| self.fsr_err(&name, args[7]))?; }
                            self.mac.log.push(format!("[file] {name} {:?} in dirID {dir} -> node {node}", String::from_utf8_lossy(&leaf)));
                            e
                        }
                    }
                }
            },
            // FSGetCatalogInfo(const FSRef *ref, FSCatalogInfoBitmap whichInfo, FSCatalogInfo *catalogInfo, HFSUniStr255 *outName, FSSpec *fsSpec, FSRef *parentRef)
            // FSOpenIterator(const FSRef *container, FSIteratorFlags flags, FSIterator *iterator): kFSIterateFlat (0) over a folder's items;
            // kFSIterateSubtree (1) is only for FSCatalogSearch, which is not provided: errFSBadIteratorFlags, named
            FSOpenIterator => match self.fsref_node(args[0]) {
                Err(e) => i32::from(e),
                Ok(_) if args[1] != 0 => { self.mac.log.push(format!("[file] FSOpenIterator flags {:#x}: only kFSIterateFlat is provided", args[1])); BAD_ITERATOR_FLAGS }
                Ok(dir) if !self.mac.fm.tree.is_dir(dir) => -120, // dirNFErr
                Ok(dir) => {
                    let items: Vec<usize> = self.mac.fm.tree.children(dir).to_vec();
                    self.fsr.next += 1;
                    let id = 0x4954_0000 | self.fsr.next; // 'IT' + a counter: never 0, never reused
                    self.fsr.iters.insert(id, (dir, items, 0));
                    self.mac.mem.write_u32(args[2], id).map_err(|_| self.fsr_err(&name, args[2]))?;
                    0
                }
            },
            FSCloseIterator => if self.fsr.iters.remove(&args[0]).is_some() { 0 } else { BAD_ITERATOR },
            // FSGetCatalogInfoBulk(iterator, maximumObjects, &actualObjects, &containerChanged, whichInfo, catalogInfos[], refs[], specs[], names[])
            FSGetCatalogInfoBulk => {
                let (it, max, actual, changed, which, cats, refs, specs, names) = (args[0], args[1], args[2], args[3], args[4], args[5], args[6], args[7], args[8]);
                match self.fsr.iters.get(&it).cloned() {
                    None => BAD_ITERATOR,
                    Some(_) if max == 0 => BAD_ITEM_COUNT,
                    Some((dir, items, done)) => {
                        // the items still in the folder, in the order they were at open, up to maximumObjects
                        let (mut live, mut next) = (Vec::new(), done);
                        while next < items.len() && live.len() < max as usize {
                            let n = items[next];
                            next += 1;
                            if self.mac.fm.tree.is_live(n) && self.mac.fm.tree.parent(n) == dir { live.push(n); }
                        }
                        for (i, &node) in live.iter().enumerate() {
                            let i = i as u32;
                            let at = |base: u32, size: u32| if base == 0 { 0 } else { base + i * size };
                            self.fsr_catalog_info(node, &[0, which, at(cats, CAT_INFO_SIZE), at(names, HFS_UNI_STR_SIZE), at(specs, FSSPEC_SIZE), 0])?;
                            if refs != 0 { self.put_fsref(refs + i * FSREF_SIZE, node)?; }
                        }
                        if let Some(e) = self.fsr.iters.get_mut(&it) { e.2 = next; }
                        if actual != 0 { self.mac.mem.write_u32(actual, live.len() as u32).map_err(|_| self.fsr_err(&name, actual))?; }
                        if changed != 0 { self.mac.mem.write_u8(changed, 0).map_err(|_| self.fsr_err(&name, changed))?; }
                        if live.is_empty() { NO_MORE_ITEMS } else { 0 }
                    }
                }
            }
            // PBGetCatalogInfoSync(FSRefParam *pb): FSGetCatalogInfo's parameter-block form (Files.h FSRefParam: ref 28, whichInfo 32, catInfo 36,
            // spec 52, parentRef 56, outName 68), the OSErr also in ioResult (16)
            PBGetCatalogInfoSync => {
                let pb = args[0];
                let f = |m: &Self, off: u32| m.mac.mem.read_u32(pb + off).map_err(|_| m.fsr_err(&name, pb));
                let (r, which, cat, spec, parent, out_name) = (f(self, 28)?, f(self, 32)?, f(self, 36)?, f(self, 52)?, f(self, 56)?, f(self, 68)?);
                let e = match self.fsref_node(r) { Err(e) => i32::from(e), Ok(node) => { self.fsr_catalog_info(node, &[r, which, cat, out_name, spec, parent])?; 0 } };
                self.mac.mem.write_u16(pb + P_RESULT, e as u16).map_err(|_| self.fsr_err(&name, pb))?;
                e
            }
            // FSOpenResFile(const FSRef *ref, SInt8 permission) -> refNum (Resources.h, Mac OS 9 / CarbonLib): OpenRFPerm's FSRef form; the fork is
            // opened by the same path, ResErr set; -1 on failure
            FSOpenResFile => {
                let r = match self.fsref_node(args[0]) {
                    Err(e) => Err(e),
                    Ok(node) => {
                        let fm = &self.mac.fm;
                        // the fork is opened by name in its folder, as OpenRFPerm's is (a node with no leaf is a directory to the File Manager)
                        let loc = Located { vol: fm.tree.volume_of(node), parent: fm.tree.parent(node), leaf: Some(fm.tree.name(node).into()), node: Some(node) };
                        self.mac.open_res_file(&loc, args[1] as u8)
                    }
                };
                let (v, e) = match r { Ok(rn) => (i32::from(rn), 0i16), Err(e) => (-1, e) };
                let _ = self.mac.mem.write_u16(heirloom_mac68k_engine::lowmem::RES_ERR, e as u16);
                self.mac.log.push(format!("[res] FSOpenResFile -> {v} (ResErr {e})"));
                v
            }
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
                Ok(node) => self.fsr_set_cat(node, args[1], args[2])?,
            },
            // FSGetVolumeInfo(FSVolumeRefNum volume, ItemCount volumeIndex, FSVolumeRefNum *actualVolume, FSVolumeInfoBitmap whichInfo, FSVolumeInfo *info,
            // HFSUniStr255 *volumeName, FSRef *rootDirectory): volumeIndex > 0 picks the index-th mounted volume (nsvErr past the last), else `volume` names it
            // (nsvErr when it names none). The FSVolumeInfo fields asked for in whichInfo are filled from the same geometry PBHGetVInfo / PBXGetVolInfo report
            // (Files.h layout, pack(2): dates 0/8/16/24, fileCount 32, folderCount 36, totalBytes 40, freeBytes 48, blockSize 56, totalBlocks 60, freeBlocks
            // 64, nextAllocation 68, rsrcClumpSize 72, dataClumpSize 76, nextCatalogID 80, finderInfo 84, flags 116, filesystemID 118, signature 120,
            // driveNumber 122, driverRefNum 124; 126 bytes); fields not asked for are left as the caller had them
            FSGetVolumeInfo => {
                let (volume, index, actual, which, info_p, name_p, root_p) = (args[0] as u16 as i16, args[1], args[2], args[3], args[4], args[5], args[6]);
                let picked = if index > 0 { self.mac.fm.pick_volume(i16::try_from(index).unwrap_or(i16::MAX), 0, None) } else if volume == 0 { Err(-35) } else { self.mac.fm.volume_of_vref(volume) };
                match picked {
                    Err(e) => i32::from(e),
                    Ok(vol) => {
                        let i = self.mac.fm.vol_info(vol);
                        let mm = &mut self.mac.mem;
                        let e = |_| format!("{name}: the FSVolumeInfo at {info_p:#x} leaves guest RAM");
                        if actual != 0 { mm.write_u16(actual, i.vref as u16).map_err(|_| format!("{name}: actualVolume leaves guest RAM"))?; }
                        if info_p != 0 {
                            let (total, free) = (u64::from(i.total_blocks) * u64::from(i.block_size), u64::from(i.free_blocks) * u64::from(i.block_size));
                            if which & 0x0001 != 0 { mm.write_bytes(info_p, &utc(i.created)).map_err(e)?; }
                            if which & 0x0002 != 0 { mm.write_bytes(info_p + 8, &utc(i.created)).map_err(e)?; }
                            if which & 0x0004 != 0 { mm.write_bytes(info_p + 16, &[0; 8]).map_err(e)?; }
                            if which & 0x0008 != 0 { mm.write_bytes(info_p + 24, &[0; 8]).map_err(e)?; }
                            if which & 0x0010 != 0 { mm.write_bytes(info_p + 32, &i.file_count.to_be_bytes()).map_err(e)?; }
                            if which & 0x0020 != 0 { mm.write_bytes(info_p + 36, &i.dir_count.to_be_bytes()).map_err(e)?; }
                            if which & 0x0040 != 0 { mm.write_bytes(info_p + 40, &total.to_be_bytes()).map_err(e)?; mm.write_bytes(info_p + 48, &free.to_be_bytes()).map_err(e)?; }
                            if which & 0x0080 != 0 {
                                mm.write_bytes(info_p + 56, &i.block_size.to_be_bytes()).map_err(e)?;
                                mm.write_bytes(info_p + 60, &u32::from(i.total_blocks).to_be_bytes()).map_err(e)?;
                                mm.write_bytes(info_p + 64, &u32::from(i.free_blocks).to_be_bytes()).map_err(e)?;
                            }
                            if which & 0x0100 != 0 { mm.write_bytes(info_p + 68, &0u32.to_be_bytes()).map_err(e)?; }
                            if which & 0x0200 != 0 { mm.write_bytes(info_p + 72, &i.block_size.to_be_bytes()).map_err(e)?; }
                            if which & 0x0400 != 0 { mm.write_bytes(info_p + 76, &i.block_size.to_be_bytes()).map_err(e)?; }
                            if which & 0x0800 != 0 { mm.write_bytes(info_p + 80, &0x1000u32.to_be_bytes()).map_err(e)?; }
                            if which & 0x1000 != 0 { mm.write_bytes(info_p + 84, &[0; 32]).map_err(e)?; }
                            if which & 0x2000 != 0 { mm.write_bytes(info_p + 116, &i.attrib.to_be_bytes()).map_err(e)?; }
                            // filesystemID 0 (HFS, the Mac OS's own) and the signature 'BD', as PBHGetVInfo answers
                            if which & 0x4000 != 0 { mm.write_bytes(info_p + 118, &[0, 0, 0x42, 0x44]).map_err(e)?; }
                            if which & 0x8000 != 0 { mm.write_bytes(info_p + 122, &i.drive.to_be_bytes()).map_err(e)?; mm.write_bytes(info_p + 124, &0u16.to_be_bytes()).map_err(e)?; }
                        }
                        if name_p != 0 {
                            let mut u = Vec::with_capacity(2 + 2 * i.name.len());
                            u.extend_from_slice(&(i.name.len() as u16).to_be_bytes());
                            for &c in &i.name { u.extend_from_slice(&(u32::from(heirloom_mac68k_engine::macroman::to_char(c)) as u16).to_be_bytes()); }
                            self.mac.mem.write_bytes(name_p, &u).map_err(|_| self.fsr_err("FSGetVolumeInfo volumeName", name_p))?;
                        }
                        if root_p != 0 { self.put_fsref(root_p, vol)?; }
                        self.mac.log.push(format!("[file] FSGetVolumeInfo(volume {volume}, index {index}, which {which:#x}) -> \"{}\" vRef {}", heirloom_mac68k_engine::macroman::decode(&i.name), i.vref));
                        0
                    }
                }
            }
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
            for &c in &info.name { u.extend_from_slice(&(u32::from(heirloom_mac68k_engine::macroman::to_char(c)) as u16).to_be_bytes()); }
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
