//! (vRefNum, dirID, name) -> a node, the HFS way (Inside Macintosh: Files, "Specifying a file or directory").
//!
//! * the START directory: `vRefNum` 0 and `dirID` 0 mean the default directory; a volume refnum with dirID 0 means that
//!   volume's root; a working-directory refnum with dirID 0 means that directory, and with a dirID the volume the WD is on;
//! * a name with a colon that does not begin with one is a FULL path, `Volume:folder:file` (the volume then overrides
//!   vRefNum); a name beginning with `:` is partial, relative to the start directory (`::` = parent); a name with no colon
//!   is an entry of the start directory; a trailing colon names a directory.
use super::{err, os_err, FileMgr, RT_DIR_ID, RT_PAR_ID, ID_BASE, WD_BASE};
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_volume::{NodeId, ROOT};

/// What a (vRefNum, dirID, name) triple points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    pub vol: NodeId,
    /// The directory the entry is (or would be) in; for a directory target, that directory's parent (or itself for a volume root).
    pub parent: NodeId,
    /// The entry's own name as the guest wrote it (None when the target is the start directory itself).
    pub leaf: Option<String>,
    /// The node, if it exists.
    pub node: Option<NodeId>,
}

impl FileMgr {
    /// The volume a vRefNum names (a volume refnum or a working directory's volume); 0 = the default volume.
    pub fn volume_of_vref(&self, vref: i16) -> Result<NodeId, i16> {
        if vref == 0 { return Ok(self.def_vol); }
        if let Some(&(v, _)) = self.wds.get(&vref) { return Ok(v); }
        let idx = (-(i32::from(vref)) - 1) as usize;
        if vref < 0 { if let Some(&v) = self.tree.children(ROOT).get(idx) { return Ok(v); } }
        Err(err::NSV_ERR)
    }

    /// The node of directory `dir_id` on volume `vol`.
    pub fn dir_node(&self, vol: NodeId, dir_id: u32) -> Result<NodeId, i16> {
        match dir_id {
            RT_DIR_ID => Ok(vol),
            RT_PAR_ID => Err(err::DIR_NF_ERR),
            id if id >= ID_BASE => {
                let n = (id - ID_BASE) as usize;
                if self.tree.is_live(n) && n != ROOT && self.tree.is_dir(n) && self.tree.volume_of(n) == vol { Ok(n) } else { Err(err::DIR_NF_ERR) }
            }
            _ => Err(err::DIR_NF_ERR),
        }
    }

    /// The start directory for (vRefNum, dirID).
    pub fn start_dir(&self, vref: i16, dir_id: u32) -> Result<(NodeId, NodeId), i16> {
        if vref == 0 && dir_id == 0 { return Ok((self.def_vol, self.def_dir)); }
        if vref == 0 { return Ok((self.def_vol, self.dir_node(self.def_vol, dir_id)?)); }
        if let Some(&(v, d)) = self.wds.get(&vref) {
            return if dir_id == 0 { Ok((v, d)) } else { Ok((v, self.dir_node(v, dir_id)?)) };
        }
        let vol = self.volume_of_vref(vref)?;
        Ok((vol, if dir_id == 0 { vol } else { self.dir_node(vol, dir_id)? }))
    }

    /// Resolve `name` (None / empty = the start directory itself).
    pub fn locate(&self, vref: i16, dir_id: u32, name: Option<&[u8]>) -> Result<Located, i16> {
        let name = name.filter(|n| !n.is_empty());
        let Some(name) = name else {
            let (vol, dir) = self.start_dir(vref, dir_id)?;
            return Ok(Located { vol, parent: if dir == vol { vol } else { self.tree.parent(dir) }, leaf: None, node: Some(dir) });
        };
        let colon = name.iter().position(|&b| b == b':');
        let (vol, mut cur, rest): (NodeId, NodeId, &[u8]) = match colon {
            Some(0) => { let (v, d) = self.start_dir(vref, dir_id)?; (v, d, &name[1..]) }
            Some(i) => {
                let vname = super::name_of(&name[..i]);
                let v = self.tree.volume(&vname).ok_or(err::NSV_ERR)?;
                (v, v, &name[i + 1..])
            }
            None => { let (v, d) = self.start_dir(vref, dir_id)?; (v, d, name) }
        };
        let trailing_colon = colon.is_some() && rest.last() == Some(&b':');
        let mut comps: Vec<&[u8]> = rest.split(|&b| b == b':').collect();
        if trailing_colon || colon.is_some() && rest.is_empty() { comps.pop(); } // "a:b:" splits into a, b, ""
        // A name with a colon but a bare partial (":" + "") names the start directory; "Vol:" names the volume root.
        if comps.is_empty() { return Ok(Located { vol, parent: if cur == vol { vol } else { self.tree.parent(cur) }, leaf: None, node: Some(cur) }); }
        let last = comps.len() - 1;
        for (i, c) in comps.iter().enumerate() {
            if c.is_empty() { // "::" : the parent directory
                if cur == vol { return Err(err::DIR_NF_ERR); }
                cur = self.tree.parent(cur);
                if i == last { return Ok(Located { vol, parent: if cur == vol { vol } else { self.tree.parent(cur) }, leaf: None, node: Some(cur) }); }
                continue;
            }
            let nm = super::name_of(c);
            if i == last && !trailing_colon {
                self.tree.check_name(&nm, &[]).map_err(os_err)?;
                let node = self.tree.find_child(cur, &nm);
                return Ok(Located { vol, parent: cur, leaf: Some(nm), node });
            }
            match self.tree.find_child(cur, &nm) {
                Some(n) if self.tree.is_dir(n) => { cur = n; }
                Some(_) | None => return Err(err::DIR_NF_ERR),
            }
            if i == last { return Ok(Located { vol, parent: self.tree.parent(cur), leaf: Some(nm), node: Some(cur) }); }
        }
        Err(err::BD_NAM_ERR)
    }

    /// A working-directory refnum for (vol, dir): the volume refnum itself for a volume root, else a WD (made on demand,
    /// reused when it already exists).
    pub fn wd_for(&mut self, vol: NodeId, dir: NodeId) -> i16 {
        if dir == vol { return self.vref_of(vol); }
        if let Some((&r, _)) = self.wds.iter().find(|(_, &(v, d))| v == vol && d == dir) { return r; }
        self.next_wd += 1;
        let r = WD_BASE - self.next_wd;
        self.wds.insert(r, (vol, dir));
        r
    }
    pub fn close_wd(&mut self, wd: i16) -> Result<(), i16> { self.wds.remove(&wd).map(|_| ()).ok_or(err::NSV_ERR) }
    pub fn wd(&self, wd: i16) -> Option<(NodeId, NodeId)> { self.wds.get(&wd).copied() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fm() -> FileMgr {
        let mut f = FileMgr::new();
        let root = f.def_vol;
        let d = f.tree.create_dir(root, "Docs").unwrap();
        f.tree.create_dir(d, "Sub").unwrap();
        f.tree.create_file(d, "Note").unwrap();
        f.tree.create_file(root, "Top").unwrap();
        f.add_volume("Other", false);
        f
    }

    #[test]
    fn full_partial_and_plain_names_resolve_the_hfs_way() {
        let f = fm();
        let root = f.def_vol;
        let top = f.locate(0, 0, Some(b"Top")).unwrap();
        assert_eq!((top.parent, top.leaf.as_deref(), top.node.is_some()), (root, Some("Top"), true));
        let full = f.locate(0, 0, Some(b"Macintosh HD:Docs:Note")).unwrap();
        assert_eq!(f.tree.path_components(full.node.unwrap()), ["Macintosh HD", "Docs", "Note"]);
        let partial = f.locate(0, 0, Some(b":Docs:Sub")).unwrap();
        assert!(partial.node.is_some());
        assert_eq!(f.locate(0, 0, Some(b"macintosh hd:docs:note")).unwrap().node, full.node, "case-insensitive");
        let dir = f.locate(0, 0, Some(b"Macintosh HD:Docs:")).unwrap();
        assert_eq!(f.tree.name(dir.node.unwrap()), "Docs", "a trailing colon names a directory");
        let vol = f.locate(0, 0, Some(b"Macintosh HD:")).unwrap();
        assert_eq!(vol.node, Some(root));
        // negative controls: the right error numbers
        assert_eq!(f.locate(0, 0, Some(b"Nowhere:x")), Err(err::NSV_ERR));
        assert_eq!(f.locate(0, 0, Some(b"Macintosh HD:Nope:x")), Err(err::DIR_NF_ERR));
        assert_eq!(f.locate(0, 0, Some(b"Absent")).unwrap().node, None, "a missing leaf resolves to 'would be here'");
        assert_eq!(f.locate(7, 0, None), Err(err::NSV_ERR));
    }

    #[test]
    fn dir_ids_vrefnums_and_working_directories_agree() {
        let mut f = fm();
        let docs = f.locate(0, 0, Some(b":Docs")).unwrap().node.unwrap();
        let id = f.dir_id(docs);
        assert_eq!(f.dir_id(f.def_vol), RT_DIR_ID);
        assert_eq!(f.locate(-1, id, Some(b"Note")).unwrap().parent, docs, "volume refnum + dirID");
        let wd = f.wd_for(f.def_vol, docs);
        assert_eq!(f.wd_for(f.def_vol, docs), wd, "a WD is reused");
        assert_eq!(f.locate(wd, 0, Some(b"Note")).unwrap().parent, docs, "a working directory refnum");
        assert_eq!(f.wd_for(f.def_vol, f.def_vol), -1, "a volume root is the volume refnum itself");
        assert_eq!(f.volume_of_vref(-2), Ok(f.tree.volume("Other").unwrap()));
        assert_eq!(f.dir_node(f.def_vol, 9999), Err(err::DIR_NF_ERR));
        assert_eq!(f.locate(0, 0, Some(b":Docs::Top")).unwrap().node.is_some(), true, "'::' goes to the parent");
        f.close_wd(wd).unwrap();
        assert_eq!(f.locate(wd, 0, Some(b"Note")), Err(err::NSV_ERR), "a closed WD is gone");
    }
}
