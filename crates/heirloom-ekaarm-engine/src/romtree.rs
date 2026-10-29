//! The device ROM as a staged tree: a [`RomSource`] over one volume of a heirloom-volume [`Tree`] (os-eka.library receives the
//! ROM this way - the host sends the bundle's `Rom/` subset as a volume snapshot, a named input - because a library cannot reach
//! the host's files). Names match case-insensitively, as Symbian's do (the tree's own rule); an optional `X:` is dropped, `\` and
//! `/` both separate, and a `.` or `..` component is refused, so a guest path cannot leave the ROM.
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_volume::{NodeId, Tree};
use crate::drive::RomSource;

pub struct TreeRom {
    tree: Tree,
    root: NodeId,
}

impl TreeRom {
    /// The ROM in a snapshot holding exactly one volume (its name does not matter); anything else is refused by name.
    pub fn from_snapshot(bytes: &[u8]) -> Result<TreeRom, String> {
        let tree = heirloom_volume::snapshot::decode(bytes).map_err(|e| alloc::format!("the ROM snapshot is refused ({e:?})"))?;
        match tree.children(heirloom_volume::ROOT) {
            [root] => Ok(TreeRom { root: *root, tree }),
            v => Err(alloc::format!("the ROM snapshot holds {} volumes, not one", v.len())),
        }
    }

    fn resolve(&self, path: &str) -> Option<NodeId> {
        let b = path.as_bytes();
        let rest = if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() { &path[2..] } else { path };
        let mut at = self.root;
        for c in rest.split(['\\', '/']).filter(|c| !c.is_empty()) {
            if c == "." || c == ".." {
                return None;
            }
            at = self.tree.find_child(at, c)?;
        }
        Some(at)
    }
}

impl RomSource for TreeRom {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.tree.file_data(self.resolve(path)?).map(<[u8]>::to_vec)
    }
    fn exists(&self, path: &str) -> bool {
        self.resolve(path).is_some()
    }
    fn list(&self, path: &str) -> Option<Vec<String>> {
        let dir = self.resolve(path)?;
        self.tree.is_dir(dir).then(|| self.tree.children(dir).iter().map(|&c| String::from(self.tree.name(c))).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use heirloom_volume::snapshot::encode;

    fn rom() -> TreeRom {
        let mut t = Tree::new(255);
        let v = t.add_volume("ROM", false);
        let res = t.mkdirs(v, ["resource", "apps"]).unwrap();
        t.put_file(res, "FileBrowser.rsc", alloc::vec![1, 2, 3]).unwrap();
        TreeRom::from_snapshot(&encode(&t)).unwrap()
    }

    #[test]
    fn paths_resolve_case_insensitively_with_either_separator_and_a_drive() {
        let r = rom();
        assert_eq!(r.read("z:\\RESOURCE\\Apps\\filebrowser.RSC"), Some(alloc::vec![1, 2, 3]));
        assert_eq!(r.read("/resource/apps/FileBrowser.rsc"), Some(alloc::vec![1, 2, 3]));
        assert!(r.exists("resource\\apps"));
        assert_eq!(r.list("Z:\\resource"), Some(alloc::vec![String::from("apps")]));
        assert_eq!(r.read("resource\\apps"), None, "a directory has no bytes");
        assert_eq!(r.list("resource\\apps\\FileBrowser.rsc"), None, "a file is not listed");
    }

    #[test]
    fn a_path_cannot_leave_the_rom_and_a_bad_snapshot_is_refused() {
        let r = rom();
        assert!(!r.exists("resource\\..\\resource"));
        assert!(!r.exists("missing.rsc"));
        assert!(TreeRom::from_snapshot(b"not a snapshot").is_err());
        let mut two = Tree::new(255);
        two.add_volume("A", false);
        two.add_volume("B", false);
        assert!(TreeRom::from_snapshot(&encode(&two)).err().unwrap().contains("2 volumes"));
    }
}
