//! The guest's filesystem: an in-memory tree of devices, directories and files the HOST fills and the guest's
//! dos.library reads (and, where a device is writable, changes). Names are case-insensitive like AmigaOS but keep
//! their spelling. Assigns (`LIBS:` -> `SYS:Libs`) are plain path expansions.
//!
//! The root is a pseudo-directory whose children are the devices (`SYS`, `RAM`, ...); a lock on `SYS:` is the device
//! directory, whose parent is nobody (ParentDir -> 0).
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use crate::dos_err::*;

pub type NodeId = usize;
/// The pseudo-root that holds the devices.
pub const ROOT: NodeId = 0;
/// The device a lock of 0 means (the boot volume).
pub const BOOT_DEVICE: &str = "SYS";
const MAX_NAME: usize = 107; // fib_FileName is 108 bytes (dos/dos.h MAXFILENAMELENGTH)
const MAX_ASSIGN_DEPTH: u32 = 8;

pub enum Kind { Dir(Vec<NodeId>), File(Vec<u8>) }

pub struct Node {
    pub name: String,
    pub parent: NodeId,
    pub kind: Kind,
    /// fib_Protection bits (active-low owner rwed: 0 = everything allowed).
    pub protection: u32,
    pub comment: String,
    /// DateStamp days, minutes, ticks.
    pub date: [i32; 3],
    writable: bool,
}

pub struct Volume {
    nodes: Vec<Option<Node>>,
    assigns: BTreeMap<String, Vec<String>>,
}

fn same(a: &str, b: &str) -> bool { a.eq_ignore_ascii_case(b) }

impl Default for Volume { fn default() -> Self { Self::new() } }

impl Volume {
    /// SYS: (read-only for the guest), RAM: (writable) and the usual assigns with their directories.
    pub fn new() -> Volume {
        let root = Node { name: String::new(), parent: ROOT, kind: Kind::Dir(Vec::new()), protection: 0, comment: String::new(), date: [0; 3], writable: false };
        let mut v = Volume { nodes: alloc::vec![Some(root)], assigns: BTreeMap::new() };
        v.add_device(BOOT_DEVICE, false);
        v.add_device("RAM", true);
        for (name, target) in [("LIBS", "SYS:Libs"), ("DEVS", "SYS:Devs"), ("C", "SYS:C"), ("S", "SYS:S"), ("L", "SYS:L"), ("ENV", "RAM:Env"), ("T", "RAM:T")] {
            let _ = v.mkdirs(target);
            v.assign(name, target);
        }
        v.assign("PROGDIR", "SYS:");
        v
    }

    fn node(&self, id: NodeId) -> &Node { self.nodes[id].as_ref().expect("live node id") }
    fn node_mut(&mut self, id: NodeId) -> &mut Node { self.nodes[id].as_mut().expect("live node id") }
    pub fn is_live(&self, id: NodeId) -> bool { self.nodes.get(id).map_or(false, |n| n.is_some()) }
    pub fn get(&self, id: NodeId) -> &Node { self.node(id) }
    pub fn get_mut(&mut self, id: NodeId) -> &mut Node { self.node_mut(id) }
    pub fn name(&self, id: NodeId) -> &str { &self.node(id).name }
    pub fn is_dir(&self, id: NodeId) -> bool { matches!(self.node(id).kind, Kind::Dir(_)) }
    pub fn is_device(&self, id: NodeId) -> bool { id != ROOT && self.node(id).parent == ROOT }
    pub fn children(&self, id: NodeId) -> &[NodeId] { match &self.node(id).kind { Kind::Dir(c) => c, Kind::File(_) => &[] } }
    pub fn file_data(&self, id: NodeId) -> Option<&[u8]> { match &self.node(id).kind { Kind::File(d) => Some(d), Kind::Dir(_) => None } }
    pub fn file_data_mut(&mut self, id: NodeId) -> Option<&mut Vec<u8>> { match &mut self.node_mut(id).kind { Kind::File(d) => Some(d), Kind::Dir(_) => None } }

    /// The device directory `id` lives on.
    pub fn device_of(&self, mut id: NodeId) -> NodeId {
        while id != ROOT && self.node(id).parent != ROOT { id = self.node(id).parent; }
        id
    }
    pub fn is_writable(&self, id: NodeId) -> bool { let d = self.device_of(id); d != ROOT && self.node(d).writable }
    pub fn boot_device(&self) -> NodeId { self.device(BOOT_DEVICE).expect("SYS exists") }
    pub fn device(&self, name: &str) -> Option<NodeId> { self.children(ROOT).iter().copied().find(|&c| same(self.name(c), name)) }

    /// Add (or find) a device. `writable` is what the GUEST may do; host population ignores it.
    pub fn add_device(&mut self, name: &str, writable: bool) -> NodeId {
        if let Some(d) = self.device(name) { self.node_mut(d).writable = writable; return d; }
        let id = self.push(Node { name: name.to_string(), parent: ROOT, kind: Kind::Dir(Vec::new()), protection: 0, comment: String::new(), date: [0; 3], writable });
        self.link(ROOT, id);
        id
    }

    fn push(&mut self, n: Node) -> NodeId { self.nodes.push(Some(n)); self.nodes.len() - 1 }
    fn link(&mut self, dir: NodeId, id: NodeId) { if let Kind::Dir(c) = &mut self.node_mut(dir).kind { c.push(id); } }

    /// Define `name:` as an alias for the path `target` (replaces an earlier assign).
    pub fn assign(&mut self, name: &str, target: &str) {
        self.assigns.retain(|k, _| !same(k, name));
        self.assigns.insert(name.to_string(), alloc::vec![target.to_string()]);
    }
    /// Add another directory to a multi-directory assign (`Assign LIBS: DOpus5:Libs ADD`): a path under it resolves
    /// in the first directory that has it.
    pub fn assign_add(&mut self, name: &str, target: &str) {
        match self.assigns.iter_mut().find(|(k, _)| same(k, name)) {
            Some((_, v)) => v.push(target.to_string()),
            None => self.assign(name, target),
        }
    }
    pub fn assign_targets(&self, name: &str) -> Option<&[String]> { self.assigns.iter().find(|(k, _)| same(k, name)).map(|(_, v)| v.as_slice()) }

    pub fn find_child(&self, dir: NodeId, name: &str) -> Option<NodeId> { self.children(dir).iter().copied().find(|&c| same(self.name(c), name)) }

    fn child(&self, dir: NodeId, name: &str) -> Result<NodeId, DosErr> {
        if !self.is_dir(dir) { return Err(ERROR_OBJECT_WRONG_TYPE); }
        self.find_child(dir, name).ok_or(ERROR_OBJECT_NOT_FOUND)
    }

    /// Find where a path starts and what is left of it: `Dev:x`, `Assign:x`, `:x` (this volume's root) or relative.
    /// An assign with several directories gives several candidate starts.
    fn starts(&self, cwd: NodeId, path: &str, depth: u32) -> Result<(Vec<NodeId>, usize), DosErr> {
        let Some(i) = path.find(':') else { return Ok((alloc::vec![cwd], 0)) };
        let vol = &path[..i];
        if vol.is_empty() { return Ok((alloc::vec![self.device_of(cwd)], i + 1)); }
        if let Some(targets) = self.assign_targets(vol) {
            if depth >= MAX_ASSIGN_DEPTH { return Err(ERROR_TOO_MANY_LEVELS); }
            let found: Vec<NodeId> = targets.iter().filter_map(|t| self.resolve_depth(ROOT, t, depth + 1).ok()).collect();
            return if found.is_empty() { Err(ERROR_OBJECT_NOT_FOUND) } else { Ok((found, i + 1)) };
        }
        self.device(vol).map(|d| (alloc::vec![d], i + 1)).ok_or(ERROR_DEVICE_NOT_MOUNTED)
    }

    /// Resolve `path` (relative to the directory `cwd`) to an existing node.
    pub fn resolve(&self, cwd: NodeId, path: &str) -> Result<NodeId, DosErr> { self.resolve_depth(cwd, path, 0) }

    fn resolve_depth(&self, cwd: NodeId, path: &str, depth: u32) -> Result<NodeId, DosErr> {
        let (starts, from) = self.starts(cwd, path, depth)?;
        let mut last = ERROR_OBJECT_NOT_FOUND;
        for s in starts {
            match self.walk(s, &path[from..]) { Ok(n) => return Ok(n), Err(e) => last = e }
        }
        Err(last)
    }

    fn walk(&self, mut cur: NodeId, rest: &str) -> Result<NodeId, DosErr> {
        let mut comp = String::new();
        for ch in rest.chars() {
            if ch != '/' { comp.push(ch); continue; }
            // "a/b": a separator. A '/' with no component before it is "parent".
            cur = if comp.is_empty() { self.parent_of(cur)? } else { self.child(cur, &comp)? };
            comp.clear();
        }
        if !comp.is_empty() { cur = self.child(cur, &comp)?; }
        Ok(cur)
    }

    /// The directory containing `id`; a device (or the root) has none.
    pub fn parent_of(&self, id: NodeId) -> Result<NodeId, DosErr> {
        if id == ROOT || self.node(id).parent == ROOT { Err(ERROR_OBJECT_NOT_FOUND) } else { Ok(self.node(id).parent) }
    }

    /// Resolve everything but the last component of `path`: (the directory it would live in, the leaf name).
    pub fn resolve_parent(&self, cwd: NodeId, path: &str) -> Result<(NodeId, String), DosErr> {
        let (dir, leaf) = match path.rfind(|c| c == '/' || c == ':') {
            Some(i) => (self.resolve(cwd, &path[..=i])?, &path[i + 1..]),
            None => (cwd, path),
        };
        Self::check_name(leaf)?;
        if !self.is_dir(dir) { return Err(ERROR_OBJECT_WRONG_TYPE); }
        Ok((dir, leaf.to_string()))
    }

    fn check_name(n: &str) -> Result<(), DosErr> {
        if n.is_empty() || n.len() > MAX_NAME || n.contains(|c| c == '/' || c == ':') { Err(ERROR_INVALID_COMPONENT_NAME) } else { Ok(()) }
    }

    fn may_change(&self, dir: NodeId) -> Result<(), DosErr> {
        if dir == ROOT || !self.is_dir(dir) { return Err(ERROR_OBJECT_WRONG_TYPE); }
        if self.is_writable(dir) { Ok(()) } else { Err(ERROR_DISK_WRITE_PROTECTED) }
    }

    /// Guest-side create: a new empty file in `dir`.
    pub fn create_file(&mut self, dir: NodeId, name: &str) -> Result<NodeId, DosErr> { self.may_change(dir)?; self.create(dir, name, Kind::File(Vec::new())) }
    /// Guest-side create: a new empty directory in `dir`.
    pub fn create_dir(&mut self, dir: NodeId, name: &str) -> Result<NodeId, DosErr> { self.may_change(dir)?; self.create(dir, name, Kind::Dir(Vec::new())) }

    fn create(&mut self, dir: NodeId, name: &str, kind: Kind) -> Result<NodeId, DosErr> {
        Self::check_name(name)?;
        if self.find_child(dir, name).is_some() { return Err(ERROR_OBJECT_EXISTS); }
        let id = self.push(Node { name: name.to_string(), parent: dir, kind, protection: 0, comment: String::new(), date: [0; 3], writable: false });
        self.link(dir, id);
        Ok(id)
    }

    /// Guest-side delete of a file or an empty directory.
    pub fn remove(&mut self, id: NodeId) -> Result<(), DosErr> {
        if self.is_device(id) || id == ROOT { return Err(ERROR_OBJECT_WRONG_TYPE); }
        let parent = self.node(id).parent;
        self.may_change(parent)?;
        if !self.children(id).is_empty() { return Err(ERROR_DIRECTORY_NOT_EMPTY); }
        if let Kind::Dir(c) = &mut self.node_mut(parent).kind { c.retain(|&x| x != id); }
        self.nodes[id] = None;
        Ok(())
    }

    /// Guest-side rename/move within one device.
    pub fn rename(&mut self, id: NodeId, new_dir: NodeId, new_name: &str) -> Result<(), DosErr> {
        if self.is_device(id) || id == ROOT { return Err(ERROR_OBJECT_WRONG_TYPE); }
        Self::check_name(new_name)?;
        let old_dir = self.node(id).parent;
        self.may_change(old_dir)?;
        self.may_change(new_dir)?;
        if self.device_of(old_dir) != self.device_of(new_dir) { return Err(ERROR_RENAME_ACROSS_DEVICES); }
        if self.find_child(new_dir, new_name).map_or(false, |x| x != id) { return Err(ERROR_OBJECT_EXISTS); }
        let mut up = new_dir; // a directory cannot move into itself
        while up != ROOT { if up == id { return Err(ERROR_OBJECT_IN_USE); } up = self.node(up).parent; }
        if let Kind::Dir(c) = &mut self.node_mut(old_dir).kind { c.retain(|&x| x != id); }
        self.link(new_dir, id);
        let n = self.node_mut(id);
        n.parent = new_dir;
        n.name = new_name.to_string();
        Ok(())
    }

    /// Host-side: make every directory of `path` (`Dev:a/b`), ignoring device protection. The device must exist.
    pub fn mkdirs(&mut self, path: &str) -> Result<NodeId, DosErr> {
        let (starts, from) = self.starts(ROOT, path, 0)?;
        let mut cur = starts[0];
        for comp in path[from..].split('/').filter(|c| !c.is_empty()) {
            cur = match self.find_child(cur, comp) {
                Some(c) if self.is_dir(c) => c,
                Some(_) => return Err(ERROR_OBJECT_WRONG_TYPE),
                None => self.create(cur, comp, Kind::Dir(Vec::new()))?,
            };
        }
        Ok(cur)
    }

    /// Host-side: put a file at `path` (`Dev:a/b/file`), making directories and replacing an existing file.
    pub fn put_file(&mut self, path: &str, data: Vec<u8>) -> Result<NodeId, DosErr> {
        let i = path.rfind(|c| c == '/' || c == ':').ok_or(ERROR_INVALID_COMPONENT_NAME)?;
        let dir = self.mkdirs(&path[..=i])?;
        let leaf = &path[i + 1..];
        Self::check_name(leaf)?;
        match self.find_child(dir, leaf) {
            Some(f) => match self.file_data_mut(f) { Some(d) => { *d = data; Ok(f) } None => Err(ERROR_OBJECT_WRONG_TYPE) },
            None => self.create(dir, leaf, Kind::File(data)),
        }
    }

    /// `Dev:dir/file` for a node (`Dev:` for a device).
    pub fn full_path(&self, id: NodeId) -> String {
        let mut parts = Vec::new();
        let mut cur = id;
        while cur != ROOT && !self.is_device(cur) { parts.push(self.name(cur)); cur = self.node(cur).parent; }
        let mut s = String::new();
        if cur != ROOT { s.push_str(self.name(cur)); }
        s.push(':');
        for (i, p) in parts.iter().rev().enumerate() { if i > 0 { s.push('/'); } s.push_str(p); }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vol() -> Volume {
        let mut v = Volume::new();
        v.put_file("SYS:Libs/Foo.library", b"abc".to_vec()).unwrap();
        v.put_file("SYS:Libs/mui/Bar.mcc", b"x".to_vec()).unwrap();
        v
    }

    #[test]
    fn assigns_and_case_insensitive_names_resolve() {
        let v = vol();
        let a = v.resolve(ROOT, "LIBS:foo.LIBRARY").unwrap();
        assert_eq!(v.full_path(a), "SYS:Libs/Foo.library");
        assert_eq!(v.resolve(ROOT, "sys:libs/foo.library").unwrap(), a);
        assert_eq!(v.resolve(ROOT, "libs:mui/bar.mcc").map(|n| v.full_path(n)).unwrap(), "SYS:Libs/mui/Bar.mcc");
        // negative controls: the failures are the right AmigaDOS codes
        assert_eq!(v.resolve(ROOT, "LIBS:nope"), Err(ERROR_OBJECT_NOT_FOUND));
        assert_eq!(v.resolve(ROOT, "NODEV:x"), Err(ERROR_DEVICE_NOT_MOUNTED));
        assert_eq!(v.resolve(ROOT, "LIBS:Foo.library/x"), Err(ERROR_OBJECT_WRONG_TYPE));
    }

    #[test]
    fn relative_parent_and_volume_root_paths() {
        let v = vol();
        let libs = v.resolve(ROOT, "LIBS:").unwrap();
        assert_eq!(v.resolve(libs, "mui/Bar.mcc").map(|n| v.full_path(n)).unwrap(), "SYS:Libs/mui/Bar.mcc");
        assert_eq!(v.resolve(libs, "/").unwrap(), v.boot_device());
        assert_eq!(v.resolve(libs, ":Libs").unwrap(), libs);
        assert_eq!(v.resolve(libs, "mui//Foo.library").map(|n| v.full_path(n)).unwrap(), "SYS:Libs/Foo.library");
        assert_eq!(v.resolve(libs, "//"), Err(ERROR_OBJECT_NOT_FOUND), "a device has no parent");
        assert_eq!(v.parent_of(v.boot_device()), Err(ERROR_OBJECT_NOT_FOUND));
    }

    #[test]
    fn ram_is_writable_sys_is_not() {
        let mut v = vol();
        let ram = v.device("RAM").unwrap();
        let f = v.create_file(ram, "a").unwrap();
        v.file_data_mut(f).unwrap().extend_from_slice(b"hi");
        assert_eq!(v.create_file(ram, "A"), Err(ERROR_OBJECT_EXISTS));
        let libs = v.resolve(ROOT, "LIBS:").unwrap();
        assert_eq!(v.create_file(libs, "x"), Err(ERROR_DISK_WRITE_PROTECTED));
        assert_eq!(v.create_file(ram, "b/c"), Err(ERROR_INVALID_COMPONENT_NAME));
        v.rename(f, ram, "B").unwrap();
        assert_eq!(v.resolve(ROOT, "ram:b").unwrap(), f);
        assert_eq!(v.resolve(ROOT, "ram:a"), Err(ERROR_OBJECT_NOT_FOUND));
        v.remove(f).unwrap();
        assert!(!v.is_live(f));
        let foo = v.resolve(ROOT, "LIBS:Foo.library").unwrap();
        assert_eq!(v.remove(foo), Err(ERROR_DISK_WRITE_PROTECTED));
    }

    #[test]
    fn put_file_replaces_and_rename_across_devices_is_refused() {
        let mut v = vol();
        let f = v.put_file("SYS:Libs/Foo.library", b"new".to_vec()).unwrap();
        assert_eq!(v.file_data(f).unwrap(), b"new");
        let ram = v.device("RAM").unwrap();
        let env = v.resolve(ROOT, "ENV:").unwrap();
        let t = v.create_file(env, "x").unwrap();
        assert_eq!(v.rename(t, ram, "y"), Ok(()));
        assert_eq!(v.rename(f, ram, "y"), Err(ERROR_DISK_WRITE_PROTECTED));
    }
}
