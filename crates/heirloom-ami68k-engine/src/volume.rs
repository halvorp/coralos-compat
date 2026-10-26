//! The guest's filesystem as AmigaOS sees it: the shared [`heirloom_volume::Tree`] (volumes = devices `SYS`, `RAM`, ...,
//! directories, files) plus what is AmigaDOS's own: assigns (`LIBS:` -> `SYS:Libs`), `Dev:a/b` path syntax, and the
//! IoErr codes the tree's [`VolErr`] maps to. The HOST fills the tree and the guest's dos.library reads (and, on a
//! writable device, changes) it. A lock on `SYS:` is the device directory, whose parent is nobody (ParentDir -> 0).
//! The tree itself (names, forks, Finder info, dates) is shared with the Mac engine: `Volume` derefs to it.
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::ops::{Deref, DerefMut};
use crate::dos_err::*;
pub use heirloom_volume::{Kind, Node, NodeId, ROOT};
use heirloom_volume::{Tree, VolErr};

/// The device a lock of 0 means (the boot volume).
pub const BOOT_DEVICE: &str = "SYS";
const MAX_NAME: usize = 107; // fib_FileName is 108 bytes (dos/dos.h MAXFILENAMELENGTH)
const MAX_ASSIGN_DEPTH: u32 = 8;
/// 1978-01-01 (the AmigaDOS epoch) in milliseconds since the Unix epoch.
const AMIGA_EPOCH_MS: i64 = 252_460_800_000;

/// AmigaDOS DateStamps on a shared node: `[days, minutes, ticks]` since 1978-01-01 (a tick is 1/50 s) <-> milliseconds.
pub trait AmigaDate {
    fn date(&self) -> [i32; 3];
    fn set_date(&mut self, d: [i32; 3]);
}
impl AmigaDate for Node {
    fn date(&self) -> [i32; 3] {
        let ms = self.modified_ms - AMIGA_EPOCH_MS;
        let (min, rem) = (ms.div_euclid(60_000), ms.rem_euclid(60_000));
        [min.div_euclid(1440) as i32, min.rem_euclid(1440) as i32, (rem / 20) as i32]
    }
    fn set_date(&mut self, d: [i32; 3]) {
        self.modified_ms = AMIGA_EPOCH_MS + (i64::from(d[0]) * 1440 + i64::from(d[1])) * 60_000 + i64::from(d[2]) * 20;
    }
}

fn err(e: VolErr) -> DosErr {
    match e {
        VolErr::NotFound => ERROR_OBJECT_NOT_FOUND, VolErr::Exists => ERROR_OBJECT_EXISTS, VolErr::WrongType => ERROR_OBJECT_WRONG_TYPE,
        VolErr::ReadOnly => ERROR_DISK_WRITE_PROTECTED, VolErr::InvalidName => ERROR_INVALID_COMPONENT_NAME, VolErr::NotEmpty => ERROR_DIRECTORY_NOT_EMPTY,
        VolErr::AcrossVolumes => ERROR_RENAME_ACROSS_DEVICES, VolErr::InUse => ERROR_OBJECT_IN_USE,
    }
}

pub struct Volume {
    tree: Tree,
    assigns: BTreeMap<String, Vec<String>>,
}
impl Deref for Volume { type Target = Tree; fn deref(&self) -> &Tree { &self.tree } }
impl DerefMut for Volume { fn deref_mut(&mut self) -> &mut Tree { &mut self.tree } }

fn same(a: &str, b: &str) -> bool { a.eq_ignore_ascii_case(b) }

impl Default for Volume { fn default() -> Self { Self::new() } }

impl Volume {
    /// SYS: (read-only for the guest), RAM: (writable) and the usual assigns with their directories.
    pub fn new() -> Volume {
        let mut v = Volume { tree: Tree::new(MAX_NAME), assigns: BTreeMap::new() };
        v.add_device(BOOT_DEVICE, false);
        v.add_device("RAM", true);
        for (name, target) in [("LIBS", "SYS:Libs"), ("DEVS", "SYS:Devs"), ("C", "SYS:C"), ("S", "SYS:S"), ("L", "SYS:L"), ("ENV", "RAM:Env"), ("ENVARC", "RAM:EnvArc"), ("T", "RAM:T")] {
            let _ = v.mkdirs(target);
            v.assign(name, target);
        }
        v.assign("PROGDIR", "SYS:");
        v
    }

    pub fn is_device(&self, id: NodeId) -> bool { self.tree.is_volume(id) }
    /// The device directory `id` lives on.
    pub fn device_of(&self, id: NodeId) -> NodeId { self.tree.volume_of(id) }
    pub fn boot_device(&self) -> NodeId { self.device(BOOT_DEVICE).expect("SYS exists") }
    pub fn device(&self, name: &str) -> Option<NodeId> { self.tree.volume(name) }
    /// Add (or find) a device. `writable` is what the GUEST may do; host population ignores it.
    pub fn add_device(&mut self, name: &str, writable: bool) -> NodeId { self.tree.add_volume(name, writable) }

    /// Define `name:` as an alias for the path `target` (replaces an earlier assign).
    pub fn assign(&mut self, name: &str, target: &str) {
        // `DOpus5:` assigned to the root of device DOpus5: is the device itself; an assign of that name would shadow it
        // and point back at itself.
        if target.strip_suffix(':').map_or(false, |t| same(t, name)) && self.device(name).is_some() { return; }
        self.assigns.retain(|k, _| !same(k, name));
        self.assigns.insert(name.to_string(), alloc::vec![target.to_string()]);
    }
    /// Add another directory to a multi-directory assign (`Assign LIBS: DOpus5:Libs ADD`): a path under it resolves
    /// in the first directory that has it.
    pub fn assign_add(&mut self, name: &str, target: &str) {
        if target.strip_suffix(':').map_or(false, |t| same(t, name)) && self.device(name).is_some() { return; }
        match self.assigns.iter_mut().find(|(k, _)| same(k, name)) {
            Some((_, v)) => v.push(target.to_string()),
            None => self.assign(name, target),
        }
    }
    /// Remove an assign; false if it was not one.
    pub fn remove_assign(&mut self, name: &str) -> bool { let n = self.assigns.len(); self.assigns.retain(|k, _| !same(k, name)); self.assigns.len() != n }
    /// Drop one directory from an assign (the assign goes when it was the last).
    pub fn assign_remove_target(&mut self, name: &str, target: &str) -> bool {
        let Some((k, v)) = self.assigns.iter_mut().find(|(k, _)| same(k, name)) else { return false };
        let n = v.len();
        v.retain(|t| !same(t, target));
        let (gone, k) = (v.len() != n, k.clone());
        if self.assigns[&k].is_empty() { self.assigns.remove(&k); }
        gone
    }
    /// Every assign with its directories, in name order.
    pub fn assign_list(&self) -> Vec<(String, Vec<String>)> { self.assigns.iter().map(|(k, v)| (k.clone(), v.clone())).collect() }
    pub fn assign_targets(&self, name: &str) -> Option<&[String]> { self.assigns.iter().find(|(k, _)| same(k, name)).map(|(_, v)| v.as_slice()) }

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
        if id == ROOT || self.tree.is_volume(id) { Err(ERROR_OBJECT_NOT_FOUND) } else { Ok(self.tree.parent(id)) }
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

    /// Guest-side create: a new empty file in `dir`.
    pub fn create_file(&mut self, dir: NodeId, name: &str) -> Result<NodeId, DosErr> { Self::check_name(name)?; self.tree.create_file(dir, name).map_err(err) }
    /// Guest-side create: a new empty directory in `dir`.
    pub fn create_dir(&mut self, dir: NodeId, name: &str) -> Result<NodeId, DosErr> { Self::check_name(name)?; self.tree.create_dir(dir, name).map_err(err) }

    /// Guest-side delete of a file or an empty directory.
    pub fn remove(&mut self, id: NodeId) -> Result<(), DosErr> { self.tree.remove(id).map_err(err) }

    /// Guest-side move and/or rename within one device.
    pub fn rename(&mut self, id: NodeId, new_dir: NodeId, new_name: &str) -> Result<(), DosErr> {
        Self::check_name(new_name)?;
        self.tree.rename(id, new_dir, new_name).map_err(err)
    }

    /// Host-side: make every directory of `path` (`Dev:a/b`), ignoring device protection. The device must exist.
    pub fn mkdirs(&mut self, path: &str) -> Result<NodeId, DosErr> {
        let (starts, from) = self.starts(ROOT, path, 0)?;
        let comps: Vec<String> = path[from..].split('/').filter(|c| !c.is_empty()).map(|c| c.to_string()).collect();
        for c in &comps { Self::check_name(c)?; }
        self.tree.mkdirs(starts[0], comps.iter().map(|c| c.as_str())).map_err(err)
    }

    /// Host-side: put a file at `path` (`Dev:a/b/file`), making directories and replacing an existing file.
    pub fn put_file(&mut self, path: &str, data: Vec<u8>) -> Result<NodeId, DosErr> {
        let i = path.rfind(|c| c == '/' || c == ':').ok_or(ERROR_INVALID_COMPONENT_NAME)?;
        let dir = self.mkdirs(&path[..=i])?;
        let leaf = &path[i + 1..];
        Self::check_name(leaf)?;
        self.tree.put_file(dir, leaf, data).map_err(err)
    }

    /// `Dev:dir/file` for a node (`Dev:` for a device).
    pub fn full_path(&self, id: NodeId) -> String {
        let parts = self.tree.path_components(id);
        let mut s = String::new();
        if let Some(dev) = parts.first() { s.push_str(dev); }
        s.push(':');
        for (i, p) in parts.iter().skip(1).enumerate() { if i > 0 { s.push('/'); } s.push_str(p); }
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
        let dh0 = v.add_device("DH0", true);
        let t = v.create_file(ram, "x").unwrap();
        assert_eq!(v.rename(t, dh0, "y"), Err(ERROR_RENAME_ACROSS_DEVICES));
        assert_eq!(v.rename(f, ram, "y"), Err(ERROR_DISK_WRITE_PROTECTED), "moving out of read-only SYS: is refused too");
        let env = v.resolve(ROOT, "ENV:").unwrap();
        assert_eq!(v.rename(t, env, "y"), Ok(()), "negative control: within one device it works");
    }

    #[test]
    fn multi_directory_assign_and_self_assign() {
        let mut v = vol();
        v.add_device("Opus", false);
        v.put_file("Opus:Libs/Special.library", b"s".to_vec()).unwrap();
        v.assign_add("LIBS", "Opus:Libs");
        assert_eq!(v.full_path(v.resolve(ROOT, "LIBS:special.library").unwrap()), "Opus:Libs/Special.library");
        assert_eq!(v.full_path(v.resolve(ROOT, "LIBS:foo.library").unwrap()), "SYS:Libs/Foo.library", "the first directory still answers");
        v.assign("OPUS", "Opus:"); // an assign naming its own device is a no-op, not a loop
        assert_eq!(v.full_path(v.resolve(ROOT, "opus:Libs").unwrap()), "Opus:Libs");
        assert!(v.assign_remove_target("LIBS", "Opus:Libs"));
        assert_eq!(v.resolve(ROOT, "LIBS:special.library"), Err(ERROR_OBJECT_NOT_FOUND));
    }
}
