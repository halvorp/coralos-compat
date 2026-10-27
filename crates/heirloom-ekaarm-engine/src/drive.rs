//! The guest's file view: a READ-ONLY drive. The host (the runner, with std) reads a directory and hands the bytes in with
//! [`Drive::put_file`]; this crate stays `no_std`. Backed by `heirloom-volume`'s `Tree` (the volume tree the Amiga and Mac
//! engines share), so lookup is case-insensitive ASCII, as Symbian's is.
//!
//! One tree answers to the drive letters `Z:` and `C:` (the host directory is the whole install tree: ROM-style `resource\`
//! and `sys\bin\` as well as `private\`). Any other letter is [`DriveError::NoDrive`]; a path with no letter is `Z:`.
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_volume::{NodeId, Tree, VolErr};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriveError {
    NoDrive(char),
    NotFound,
    IsDirectory,
    Put(VolErr),
}

pub struct Drive {
    tree: Tree,
    vol: NodeId,
}

impl Default for Drive {
    fn default() -> Self {
        Self::new()
    }
}

/// The path split into components: a leading `X:` is returned separately; `\` and `/` both separate; empty components vanish.
fn split(path: &str) -> (Option<char>, Vec<&str>) {
    let b = path.as_bytes();
    let (letter, rest) = if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() { (Some(b[0] as char), &path[2..]) } else { (None, path) };
    (letter, rest.split(|c| c == '\\' || c == '/').filter(|s| !s.is_empty()).collect())
}

impl Drive {
    pub fn new() -> Self {
        let mut tree = Tree::new(255);
        let vol = tree.add_volume("Z", false);
        Drive { tree, vol }
    }

    /// Host-side: put a file (creating its directories) at `path`.
    pub fn put_file(&mut self, path: &str, bytes: Vec<u8>) -> Result<(), DriveError> {
        let (_, mut comps) = split(path);
        let leaf = comps.pop().ok_or(DriveError::NotFound)?;
        let dir = self.tree.mkdirs(self.vol, comps).map_err(DriveError::Put)?;
        self.tree.put_file(dir, leaf, bytes).map(|_| ()).map_err(DriveError::Put)
    }

    fn resolve(&self, path: &str) -> Result<NodeId, DriveError> {
        let (letter, comps) = split(path);
        if let Some(l) = letter {
            if !l.eq_ignore_ascii_case(&'z') && !l.eq_ignore_ascii_case(&'c') {
                return Err(DriveError::NoDrive(l));
            }
        }
        let mut cur = self.vol;
        for c in comps {
            cur = self.tree.find_child(cur, c).ok_or(DriveError::NotFound)?;
        }
        Ok(cur)
    }

    pub fn read(&self, path: &str) -> Result<&[u8], DriveError> {
        let id = self.resolve(path)?;
        self.tree.file_data(id).ok_or(DriveError::IsDirectory)
    }

    /// The names in the directory at `path` (stored spelling).
    pub fn list(&self, path: &str) -> Result<Vec<String>, DriveError> {
        let id = self.resolve(path)?;
        if !self.tree.is_dir(id) {
            return Err(DriveError::NotFound);
        }
        Ok(self.tree.children(id).iter().map(|&c| String::from(self.tree.name(c))).collect())
    }

    pub fn is_empty(&self) -> bool {
        self.tree.children(self.vol).is_empty()
    }
}
