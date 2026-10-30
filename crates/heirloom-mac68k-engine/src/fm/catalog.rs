//! The catalog: Create, Delete, Rename, GetFileInfo / SetFileInfo / GetCatInfo / SetCatInfo, DirCreate, CatMove and the
//! file lock (Inside Macintosh: Files, "Manipulating files and directories"). Pure over [`FileMgr`].
use super::names::Located;
use super::{bytes_of, err, name_of, os_err, FileMgr, ALLOC_BLOCK, RT_PAR_ID};
use alloc::vec::Vec;
use heirloom_volume::{FInfo, Kind, NodeId};

/// Seconds between 1904-01-01 (the Mac epoch) and 1970-01-01.
const MAC_TO_UNIX_SECS: i64 = 2_082_844_800;

pub fn mac_secs(ms: i64) -> u32 { (ms.div_euclid(1000) + MAC_TO_UNIX_SECS).clamp(0, i64::from(u32::MAX)) as u32 }
pub fn unix_ms(mac: u32) -> i64 { (i64::from(mac) - MAC_TO_UNIX_SECS) * 1000 }

/// Everything GetCatInfo / GetFileInfo reports about one entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatInfo {
    pub is_dir: bool,
    pub name: Vec<u8>,
    /// The file number (files) or the directory's own dirID.
    pub id: u32,
    pub parent_id: u32,
    pub attrib: u8,
    pub finfo: FInfo,
    pub lg: u32,
    pub py: u32,
    pub rlg: u32,
    pub rpy: u32,
    pub created: u32,
    pub modified: u32,
    /// Items in a directory.
    pub items: u16,
}

fn round_up(n: u32) -> u32 { n.div_ceil(ALLOC_BLOCK) * ALLOC_BLOCK }

impl FileMgr {
    pub(crate) fn touch(&mut self, node: NodeId) { let ms = unix_ms(self.now_secs); self.tree.get_mut(node).modified_ms = ms; }

    /// The catalog record of `node`.
    pub fn cat_info(&self, node: NodeId) -> CatInfo {
        let n = self.tree.get(node);
        let is_dir = matches!(n.kind, Kind::Dir(_));
        let parent_id = if self.tree.is_volume(node) { RT_PAR_ID } else { self.dir_id(n.parent) };
        let mut attrib = (n.attrs & 1) as u8;
        if is_dir { attrib |= super::pb::ATTR_DIR; }
        for (_, f) in self.open_forks().filter(|(_, f)| f.node == node) { attrib |= super::pb::ATTR_OPEN | if f.rsrc { super::pb::ATTR_RSRC_OPEN } else { 0x08 }; }
        let lg = self.tree.file_len(node).map_or(0, |l| u32::try_from(l).unwrap_or(u32::MAX));
        let rlg = n.rsrc.len() as u32;
        CatInfo {
            is_dir, name: bytes_of(&n.name), id: if is_dir { self.dir_id(node) } else { self.file_id(node) }, parent_id, attrib, finfo: n.finfo,
            lg, py: round_up(lg), rlg, rpy: round_up(rlg), created: mac_secs(n.created_ms), modified: mac_secs(n.modified_ms),
            items: self.tree.children(node).len() as u16,
        }
    }

    /// The `index`th (1-based) entry of `dir`; `files_only` skips directories (the pre-HFS PBGetFInfo counts files only).
    pub fn nth_entry(&self, dir: NodeId, index: i16, files_only: bool) -> Option<NodeId> {
        if index < 1 { return None; }
        self.tree.children(dir).iter().copied().filter(|&c| !files_only || !self.tree.is_dir(c)).nth(index as usize - 1)
    }

    /// Create an empty file (type and creator '????', as the File Manager does).
    pub fn create_file(&mut self, loc: &Located) -> Result<NodeId, i16> {
        let name = loc.leaf.as_deref().ok_or(err::BD_NAM_ERR)?;
        if loc.node.is_some() { return Err(err::DUP_FN_ERR); }
        let id = self.tree.create_file(loc.parent, name).map_err(os_err)?;
        let ms = unix_ms(self.now_secs);
        let n = self.tree.get_mut(id);
        n.created_ms = ms;
        n.modified_ms = ms;
        n.finfo = FInfo { file_type: *b"????", creator: *b"????", ..FInfo::default() };
        Ok(id)
    }

    /// DirCreate: a new directory; returns it (its dirID is [`FileMgr::dir_id`]).
    pub fn create_dir(&mut self, loc: &Located) -> Result<NodeId, i16> {
        let name = loc.leaf.as_deref().ok_or(err::BD_NAM_ERR)?;
        if loc.node.is_some() { return Err(err::DUP_FN_ERR); }
        let id = self.tree.create_dir(loc.parent, name).map_err(os_err)?;
        let ms = unix_ms(self.now_secs);
        let n = self.tree.get_mut(id);
        n.created_ms = ms;
        n.modified_ms = ms;
        Ok(id)
    }

    /// Delete a file or an empty directory. An open or locked file is refused (fBsyErr / fLckdErr).
    pub fn delete(&mut self, loc: &Located) -> Result<(), i16> {
        let node = loc.node.ok_or(err::FNF_ERR)?;
        if self.tree.is_volume(node) { return Err(err::BD_NAM_ERR); }
        if self.is_open(node, false) { return Err(err::F_BSY_ERR); }
        if self.tree.get(node).attrs & 1 != 0 { return Err(err::F_LCKD_ERR); }
        if !self.tree.is_writable(node) { return Err(err::W_PR_ERR); }
        self.tree.remove(node).map_err(os_err)
    }

    pub fn rename(&mut self, loc: &Located, new_name: &[u8]) -> Result<(), i16> {
        let node = loc.node.ok_or(err::FNF_ERR)?;
        if self.tree.is_volume(node) { return Err(err::PARAM_ERR); } // renaming a volume is not modelled
        let parent = self.tree.parent(node);
        self.tree.rename(node, parent, &name_of(new_name)).map_err(os_err)
    }

    /// CatMove: move `loc` into directory `to` (same volume), keeping its name.
    pub fn cat_move(&mut self, loc: &Located, to: NodeId) -> Result<(), i16> {
        let node = loc.node.ok_or(err::FNF_ERR)?;
        let name = self.tree.name(node).to_string_owned();
        self.tree.rename(node, to, &name).map_err(os_err)
    }

    pub fn set_lock(&mut self, loc: &Located, lock: bool) -> Result<(), i16> {
        let node = loc.node.ok_or(err::FNF_ERR)?;
        if !self.tree.is_writable(node) { return Err(err::W_PR_ERR); }
        let a = &mut self.tree.get_mut(node).attrs;
        if lock { *a |= 1 } else { *a &= !1 }
        Ok(())
    }

    /// SetFileInfo / SetCatInfo: File info (FInfo) (files) and the two dates.
    pub fn set_cat(&mut self, node: NodeId, finfo: Option<FInfo>, created: u32, modified: u32) -> Result<(), i16> {
        if !self.tree.is_writable(node) { return Err(err::W_PR_ERR); }
        let n = self.tree.get_mut(node);
        if let Some(f) = finfo { if !matches!(n.kind, Kind::Dir(_)) { n.finfo = f; } }
        n.created_ms = unix_ms(created);
        n.modified_ms = unix_ms(modified);
        Ok(())
    }
}

trait ToStringOwned { fn to_string_owned(&self) -> alloc::string::String; }
impl ToStringOwned for str { fn to_string_owned(&self) -> alloc::string::String { alloc::string::String::from(self) } }

#[cfg(test)]
mod tests {
    use super::*;

    fn at(fm: &FileMgr, name: &str) -> Located { fm.locate(0, 0, Some(name.as_bytes())).unwrap() }

    #[test]
    fn create_delete_rename_lock_and_info() {
        let mut fm = FileMgr::new();
        let l = at(&fm, "a");
        let id = fm.create_file(&l).unwrap();
        let info = fm.cat_info(id);
        assert_eq!((info.finfo.file_type, info.finfo.creator, info.lg, info.is_dir), (*b"????", *b"????", 0, false));
        assert_eq!(info.created, 3_600_000_000);
        assert_eq!(fm.create_file(&at(&fm, "A")), Err(err::DUP_FN_ERR), "case-insensitive duplicate");
        assert_eq!(fm.locate(0, 0, Some("x".repeat(32).as_bytes())), Err(err::BD_NAM_ERR), "a 32-byte name is refused");
        fm.rename(&at(&fm, "a"), b"b").unwrap();
        assert!(fm.locate(0, 0, Some(b"a")).unwrap().node.is_none());
        assert_eq!(fm.rename(&at(&fm, "b"), b"b"), Ok(()), "renaming to itself is fine");
        let d = fm.create_dir(&at(&fm, "dir")).unwrap();
        assert_eq!(fm.rename(&at(&fm, "b"), b"dir"), Err(err::DUP_FN_ERR));
        assert_eq!(fm.cat_info(d).attrib & super::super::pb::ATTR_DIR, 0x10);
        fm.set_lock(&at(&fm, "b"), true).unwrap();
        assert_eq!(fm.delete(&at(&fm, "b")), Err(err::F_LCKD_ERR));
        fm.set_lock(&at(&fm, "b"), false).unwrap();
        let r = fm.open(&at(&fm, "b"), false, 1).unwrap();
        assert_eq!(fm.delete(&at(&fm, "b")), Err(err::F_BSY_ERR), "open files cannot be deleted");
        assert_eq!(fm.cat_info(fm.locate(0, 0, Some(b"b")).unwrap().node.unwrap()).attrib & 0x88, 0x88);
        fm.close(r).unwrap();
        fm.delete(&at(&fm, "b")).unwrap();
        assert_eq!(fm.delete(&at(&fm, "b")), Err(err::FNF_ERR));
        fm.cat_move(&at(&fm, "dir"), fm.def_vol).unwrap();
    }

    #[test]
    fn read_only_volumes_and_non_empty_directories_are_refused() {
        let mut fm = FileMgr::new();
        let ro = fm.add_volume("CD", false);
        fm.tree.put_file(ro, "f", vec![1]).unwrap();
        let l = fm.locate(0, 0, Some(b"CD:f")).unwrap();
        assert_eq!(fm.delete(&l), Err(err::W_PR_ERR));
        assert_eq!(fm.create_file(&fm.locate(0, 0, Some(b"CD:g")).unwrap()), Err(err::W_PR_ERR));
        let d = fm.create_dir(&at(&fm, "d")).unwrap();
        fm.create_file(&fm.locate(0, 0, Some(b":d:x")).unwrap()).unwrap();
        assert_eq!(fm.delete(&fm.locate(0, 0, Some(b":d:")).unwrap()), Err(err::F_BSY_ERR), "a non-empty directory");
        let _ = d;
    }

    #[test]
    fn nth_entry_and_dates_round_trip() {
        let mut fm = FileMgr::new();
        fm.create_dir(&at(&fm, "d")).unwrap();
        fm.create_file(&at(&fm, "f1")).unwrap();
        fm.create_file(&at(&fm, "f2")).unwrap();
        let root = fm.def_vol;
        assert_eq!(fm.nth_entry(root, 1, false).map(|n| fm.tree.name(n)), Some("d"));
        assert_eq!(fm.nth_entry(root, 1, true).map(|n| fm.tree.name(n)), Some("f1"), "files only skips the directory");
        assert_eq!(fm.nth_entry(root, 4, false), None);
        assert_eq!(fm.nth_entry(root, 0, false), None);
        let f = fm.nth_entry(root, 2, false).unwrap();
        fm.set_cat(f, Some(FInfo { file_type: *b"TEXT", creator: *b"ttxt", flags: 0x40, v: 5, h: 6 }), 3_000_000_000, 3_100_000_000).unwrap();
        let i = fm.cat_info(f);
        assert_eq!((i.finfo.file_type, i.created, i.modified), (*b"TEXT", 3_000_000_000, 3_100_000_000));
        assert_eq!(mac_secs(unix_ms(123_456_789)), 123_456_789);
    }
}
