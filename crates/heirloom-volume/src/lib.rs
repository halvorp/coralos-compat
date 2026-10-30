//! The guest's filesystem as ONE tree for every heirloom personality (no_std + alloc).
//!
//! A pseudo-root holds the VOLUMES (AmigaOS devices `SYS:`/`RAM:`, HFS volumes); below them are directories and files.
//! A file has a data fork and a resource fork (`rsrc`: empty on the Amiga), File info (FInfo) (type, creator, flags) and two
//! dates; every node has a comment, an opaque `attrs` word (the platform's own bits: AmigaDOS protection, HFS file
//! attributes) and a name. Names compare case-insensitively and keep their spelling (both AmigaOS and HFS do). The tree
//! knows nothing about paths or error numbers: the AmigaDOS layer (assigns, `Dev:a/b` paths, IoErr codes) and the
//! File Manager layer (volume refnum / dirID / name, OSErr codes) are each engine's own and map [`VolErr`].
//! A volume is writable or not for the GUEST; the host that populates the tree ignores that.
#![cfg_attr(not(test), no_std)]
extern crate alloc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

pub mod snapshot; // the whole tree as bytes and back (an engine in another process gets the volume the host staged)

pub type NodeId = usize;
/// The pseudo-root that holds the volumes.
pub const ROOT: NodeId = 0;

pub enum Kind { Dir(Vec<NodeId>), File(Vec<u8>), Stream(Stream) }

/// A READ-ONLY file whose data fork stays in the host's storage and is read on demand through the tree's [`StreamSource`]
/// (a 401 MB game archive is never copied into the engine's memory). Only a tree whose engine has called
/// [`Tree::enable_streams`] (its file layer reads through [`Tree::read_at`] / [`Tree::file_len`]) accepts one; on any other
/// tree [`Tree::put_stream`] is refused and the host reads the file in full, as before.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stream {
    /// The data fork's length in bytes.
    pub len: u64,
    /// The host's name for the bytes, passed back to [`StreamSource::read_at`].
    pub key: u32,
}

/// Where a tree's [`Stream`] files are read from: the host's storage (CoralFS in Heirloom.app, a host directory in tests).
pub trait StreamSource {
    /// Up to `dst.len()` bytes of stream `key` from `offset`: how many were read (fewer only at the end), `None` when the
    /// storage refused or failed (the file layer reports an I/O error; nothing is invented).
    fn read_at(&self, key: u32, offset: u64, dst: &mut [u8]) -> Option<usize>;
}

/// The Finder-style file info of a file (HFS FInfo): the four-character type and creator, the flag word and the icon position.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FInfo { pub file_type: [u8; 4], pub creator: [u8; 4], pub flags: u16, pub v: i16, pub h: i16 }

pub struct Node {
    pub name: String,
    pub parent: NodeId,
    pub kind: Kind,
    /// The platform's attribute bits (AmigaDOS fib_Protection; HFS ioFlAttrib).
    pub attrs: u32,
    pub comment: String,
    /// Creation and modification time, milliseconds since 1970-01-01 (each layer converts its own epoch and unit).
    pub created_ms: i64,
    pub modified_ms: i64,
    pub finfo: FInfo,
    /// The resource fork of a file (empty when it has none).
    pub rsrc: Vec<u8>,
    writable: bool,
}

/// Why a tree operation was refused; the engines map these to their own error numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolErr { NotFound, Exists, WrongType, ReadOnly, InvalidName, NotEmpty, AcrossVolumes, InUse }

pub struct Tree {
    nodes: Vec<Option<Node>>,
    max_name: usize,
    /// The engine's file layer reads data forks through [`Tree::read_at`]: [`Kind::Stream`] files are accepted.
    streams_enabled: bool,
    streams: Option<alloc::boxed::Box<dyn StreamSource>>,
}

fn same(a: &str, b: &str) -> bool { a.eq_ignore_ascii_case(b) }

impl Tree {
    /// An empty tree (only the root). `max_name` is the longest component the platform allows (Amiga 107, HFS 31).
    pub fn new(max_name: usize) -> Tree {
        let root = Node::new(String::new(), ROOT, Kind::Dir(Vec::new()), false);
        Tree { nodes: alloc::vec![Some(root)], max_name, streams_enabled: false, streams: None }
    }

    fn node(&self, id: NodeId) -> &Node { self.nodes[id].as_ref().expect("live node id") }
    fn node_mut(&mut self, id: NodeId) -> &mut Node { self.nodes[id].as_mut().expect("live node id") }
    pub fn is_live(&self, id: NodeId) -> bool { self.nodes.get(id).map_or(false, |n| n.is_some()) }
    pub fn get(&self, id: NodeId) -> &Node { self.node(id) }
    pub fn get_mut(&mut self, id: NodeId) -> &mut Node { self.node_mut(id) }
    pub fn name(&self, id: NodeId) -> &str { &self.node(id).name }
    pub fn parent(&self, id: NodeId) -> NodeId { self.node(id).parent }
    pub fn is_dir(&self, id: NodeId) -> bool { matches!(self.node(id).kind, Kind::Dir(_)) }
    /// A volume is a direct child of the root.
    pub fn is_volume(&self, id: NodeId) -> bool { id != ROOT && self.node(id).parent == ROOT }
    pub fn children(&self, id: NodeId) -> &[NodeId] { match &self.node(id).kind { Kind::Dir(c) => c, Kind::File(_) | Kind::Stream(_) => &[] } }
    /// The in-memory data fork of a file; `None` for a directory AND for a [`Kind::Stream`] file (read that with [`Tree::read_at`]).
    pub fn file_data(&self, id: NodeId) -> Option<&[u8]> { match &self.node(id).kind { Kind::File(d) => Some(d), Kind::Dir(_) | Kind::Stream(_) => None } }
    pub fn file_data_mut(&mut self, id: NodeId) -> Option<&mut Vec<u8>> { match &mut self.node_mut(id).kind { Kind::File(d) => Some(d), Kind::Dir(_) | Kind::Stream(_) => None } }
    /// A file of either kind (in memory or a stream).
    pub fn is_file(&self, id: NodeId) -> bool { !self.is_dir(id) }
    pub fn is_stream(&self, id: NodeId) -> bool { matches!(self.node(id).kind, Kind::Stream(_)) }
    /// The data fork's length of a file of either kind; `None` for a directory.
    pub fn file_len(&self, id: NodeId) -> Option<u64> {
        match &self.node(id).kind { Kind::File(d) => Some(d.len() as u64), Kind::Stream(s) => Some(s.len), Kind::Dir(_) => None }
    }
    /// Up to `dst.len()` bytes of the data fork of file `id` from `offset` (fewer at the end, 0 at or past it). `None` for a
    /// directory, and for a stream whose source is missing or failed.
    pub fn read_at(&self, id: NodeId, offset: u64, dst: &mut [u8]) -> Option<usize> {
        match &self.node(id).kind {
            Kind::Dir(_) => None,
            Kind::File(d) => {
                let start = usize::try_from(offset).ok().filter(|&o| o < d.len()).unwrap_or(d.len());
                let n = dst.len().min(d.len() - start);
                dst[..n].copy_from_slice(&d[start..start + n]);
                Some(n)
            }
            Kind::Stream(s) => {
                if offset >= s.len { return Some(0); }
                let n = dst.len().min(usize::try_from(s.len - offset).unwrap_or(usize::MAX));
                let got = self.streams.as_deref()?.read_at(s.key, offset, &mut dst[..n])?;
                Some(got.min(n))
            }
        }
    }
    /// The engine side: this tree's file layer reads every data fork through [`Tree::read_at`] / [`Tree::file_len`], so the host
    /// may put [`Kind::Stream`] files in it.
    pub fn enable_streams(&mut self) { self.streams_enabled = true; }
    pub fn streams_enabled(&self) -> bool { self.streams_enabled }
    /// The host side: where this tree's stream files are read from.
    pub fn set_stream_source(&mut self, src: alloc::boxed::Box<dyn StreamSource>) { self.streams = Some(src); }
    /// Host-side: put a stream file named `leaf` in `dir` (a new name; an existing entry is [`VolErr::Exists`]). Refused with
    /// [`VolErr::WrongType`] when the engine has not called [`Tree::enable_streams`]: its file layer would not see the bytes.
    pub fn put_stream(&mut self, dir: NodeId, leaf: &str, stream: Stream) -> Result<NodeId, VolErr> {
        if !self.streams_enabled { return Err(VolErr::WrongType); }
        self.create(dir, leaf, Kind::Stream(stream))
    }

    /// The volume `id` lives on (the root for the root).
    pub fn volume_of(&self, mut id: NodeId) -> NodeId {
        while id != ROOT && self.node(id).parent != ROOT { id = self.node(id).parent; }
        id
    }
    pub fn is_writable(&self, id: NodeId) -> bool { let d = self.volume_of(id); d != ROOT && self.node(d).writable }
    pub fn volume(&self, name: &str) -> Option<NodeId> { self.children(ROOT).iter().copied().find(|&c| same(self.name(c), name)) }

    /// Add (or find) a volume. `writable` is what the GUEST may do; host population ignores it.
    pub fn add_volume(&mut self, name: &str, writable: bool) -> NodeId {
        if let Some(d) = self.volume(name) { self.node_mut(d).writable = writable; return d; }
        let id = self.push(Node::new(name.to_string(), ROOT, Kind::Dir(Vec::new()), writable));
        self.link(ROOT, id);
        id
    }

    fn push(&mut self, n: Node) -> NodeId { self.nodes.push(Some(n)); self.nodes.len() - 1 }
    fn link(&mut self, dir: NodeId, id: NodeId) { if let Kind::Dir(c) = &mut self.node_mut(dir).kind { c.push(id); } }

    pub fn find_child(&self, dir: NodeId, name: &str) -> Option<NodeId> { self.children(dir).iter().copied().find(|&c| same(self.name(c), name)) }

    /// `name` is one component the platform accepts. `forbidden` are the separator characters of the caller's syntax.
    pub fn check_name(&self, n: &str, forbidden: &[char]) -> Result<(), VolErr> {
        if n.is_empty() || n.len() > self.max_name || n.contains(|c| forbidden.contains(&c)) { Err(VolErr::InvalidName) } else { Ok(()) }
    }

    /// The guest may change the contents of `dir`: a real directory on a writable volume.
    pub fn may_change(&self, dir: NodeId) -> Result<(), VolErr> {
        if dir == ROOT || !self.is_dir(dir) { return Err(VolErr::WrongType); }
        if self.is_writable(dir) { Ok(()) } else { Err(VolErr::ReadOnly) }
    }

    /// Host-side create (no write-protection check). The caller has validated `name` with [`Tree::check_name`] for its
    /// own separators; an empty name or an existing one is refused here.
    pub fn create(&mut self, dir: NodeId, name: &str, kind: Kind) -> Result<NodeId, VolErr> {
        if !self.is_dir(dir) { return Err(VolErr::WrongType); }
        if name.is_empty() || name.len() > self.max_name { return Err(VolErr::InvalidName); }
        if self.find_child(dir, name).is_some() { return Err(VolErr::Exists); }
        let id = self.push(Node::new(name.to_string(), dir, kind, false));
        self.link(dir, id);
        Ok(id)
    }
    /// Guest-side create of an empty file.
    pub fn create_file(&mut self, dir: NodeId, name: &str) -> Result<NodeId, VolErr> { self.may_change(dir)?; self.create(dir, name, Kind::File(Vec::new())) }
    /// Guest-side create of an empty directory.
    pub fn create_dir(&mut self, dir: NodeId, name: &str) -> Result<NodeId, VolErr> { self.may_change(dir)?; self.create(dir, name, Kind::Dir(Vec::new())) }

    /// Guest-side delete of a file or an empty directory.
    pub fn remove(&mut self, id: NodeId) -> Result<(), VolErr> {
        if id == ROOT || self.is_volume(id) { return Err(VolErr::WrongType); }
        let parent = self.node(id).parent;
        self.may_change(parent)?;
        if !self.children(id).is_empty() { return Err(VolErr::NotEmpty); }
        if let Kind::Dir(c) = &mut self.node_mut(parent).kind { c.retain(|&x| x != id); }
        self.nodes[id] = None;
        Ok(())
    }

    /// Guest-side move and/or rename within one volume.
    pub fn rename(&mut self, id: NodeId, new_dir: NodeId, new_name: &str) -> Result<(), VolErr> {
        if id == ROOT || self.is_volume(id) { return Err(VolErr::WrongType); }
        if new_name.is_empty() || new_name.len() > self.max_name { return Err(VolErr::InvalidName); }
        let old_dir = self.node(id).parent;
        self.may_change(old_dir)?;
        self.may_change(new_dir)?;
        if self.volume_of(old_dir) != self.volume_of(new_dir) { return Err(VolErr::AcrossVolumes); }
        if self.find_child(new_dir, new_name).map_or(false, |x| x != id) { return Err(VolErr::Exists); }
        let mut up = new_dir; // a directory cannot move into itself
        while up != ROOT { if up == id { return Err(VolErr::InUse); } up = self.node(up).parent; }
        if let Kind::Dir(c) = &mut self.node_mut(old_dir).kind { c.retain(|&x| x != id); }
        self.link(new_dir, id);
        let n = self.node_mut(id);
        n.parent = new_dir;
        n.name = new_name.to_string();
        Ok(())
    }

    /// Host-side: make every directory of `components` below `from`, returning the last.
    pub fn mkdirs<'a>(&mut self, from: NodeId, components: impl IntoIterator<Item = &'a str>) -> Result<NodeId, VolErr> {
        let mut cur = from;
        for comp in components {
            cur = match self.find_child(cur, comp) {
                Some(c) if self.is_dir(c) => c,
                Some(_) => return Err(VolErr::WrongType),
                None => self.create(cur, comp, Kind::Dir(Vec::new()))?,
            };
        }
        Ok(cur)
    }

    /// Host-side: put a file named `leaf` in `dir`, replacing the data of an existing one.
    pub fn put_file(&mut self, dir: NodeId, leaf: &str, data: Vec<u8>) -> Result<NodeId, VolErr> {
        match self.find_child(dir, leaf) {
            Some(f) => match self.file_data_mut(f) { Some(d) => { *d = data; Ok(f) } None => Err(VolErr::WrongType) },
            None => self.create(dir, leaf, Kind::File(data)),
        }
    }

    /// The names from the volume down to `id` (volume first); empty for the root.
    pub fn path_components(&self, id: NodeId) -> Vec<&str> {
        let mut parts = Vec::new();
        let mut cur = id;
        while cur != ROOT { parts.push(self.name(cur)); cur = self.node(cur).parent; }
        parts.reverse();
        parts
    }
}

impl Node {
    fn new(name: String, parent: NodeId, kind: Kind, writable: bool) -> Node {
        Node { name, parent, kind, attrs: 0, comment: String::new(), created_ms: 0, modified_ms: 0, finfo: FInfo::default(), rsrc: Vec::new(), writable }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> (Tree, NodeId, NodeId) {
        let mut t = Tree::new(31);
        let ro = t.add_volume("Disk", false);
        let rw = t.add_volume("Work", true);
        (t, ro, rw)
    }

    #[test]
    fn names_are_case_insensitive_and_keep_their_spelling() {
        let (mut t, _, rw) = tree();
        let f = t.create_file(rw, "ReadMe").unwrap();
        assert_eq!(t.find_child(rw, "README"), Some(f));
        assert_eq!(t.name(f), "ReadMe");
        assert_eq!(t.create_file(rw, "readme"), Err(VolErr::Exists));
        assert_eq!(t.find_child(rw, "other"), None, "negative control");
    }

    #[test]
    fn a_read_only_volume_refuses_the_guest_but_not_the_host() {
        let (mut t, ro, rw) = tree();
        assert_eq!(t.create_file(ro, "x"), Err(VolErr::ReadOnly));
        let h = t.put_file(ro, "x", b"host".to_vec()).unwrap();
        assert_eq!(t.remove(h), Err(VolErr::ReadOnly));
        assert!(t.create_file(rw, "x").is_ok(), "negative control: the writable volume accepts");
    }

    #[test]
    fn both_forks_finfo_info_and_dates_live_on_the_node() {
        let (mut t, _, rw) = tree();
        let f = t.create_file(rw, "Doc").unwrap();
        let n = t.get_mut(f);
        n.rsrc = b"RSRC".to_vec();
        n.finfo = FInfo { file_type: *b"TEXT", creator: *b"ttxt", flags: 0x100, v: 3, h: 4 };
        n.modified_ms = 1_700_000_000_000;
        t.file_data_mut(f).unwrap().extend_from_slice(b"data");
        assert_eq!((t.file_data(f).unwrap(), &t.get(f).rsrc[..], t.get(f).finfo.creator), (&b"data"[..], &b"RSRC"[..], *b"ttxt"));
        assert_eq!(t.file_data(rw), None, "a directory has no data fork");
    }

    #[test]
    fn rename_moves_within_a_volume_only_and_never_into_itself() {
        let (mut t, _, rw) = tree();
        let w2 = t.add_volume("W2", true);
        let d = t.create_dir(rw, "d").unwrap();
        let sub = t.create_dir(d, "sub").unwrap();
        let f = t.create_file(rw, "f").unwrap();
        t.rename(f, d, "g").unwrap();
        assert_eq!(t.path_components(f), ["Work", "d", "g"]);
        assert_eq!(t.rename(f, w2, "g"), Err(VolErr::AcrossVolumes));
        assert_eq!(t.rename(d, sub, "d"), Err(VolErr::InUse));
        assert_eq!(t.remove(d), Err(VolErr::NotEmpty));
        assert_eq!(t.remove(rw), Err(VolErr::WrongType), "a volume is not deletable");
    }

    #[test]
    fn name_limits_follow_the_platform() {
        let (mut t, _, rw) = tree();
        assert_eq!(t.create_file(rw, &"a".repeat(32)), Err(VolErr::InvalidName));
        assert!(t.create_file(rw, &"a".repeat(31)).is_ok());
        assert_eq!(t.check_name("a:b", &[':']), Err(VolErr::InvalidName));
        assert_eq!(t.check_name("a:b", &['/']), Ok(()));
    }
}

#[cfg(test)]
mod stream_tests {
    use super::*;
    use core::cell::Cell;
    use alloc::rc::Rc;

    /// Stream key k holds the bytes `(offset + k) as u8`; counts the reads it served.
    struct Pattern { len: u64, reads: Rc<Cell<u32>> }
    impl StreamSource for Pattern {
        fn read_at(&self, key: u32, offset: u64, dst: &mut [u8]) -> Option<usize> {
            if key == 99 { return None; }
            self.reads.set(self.reads.get() + 1);
            let n = dst.len().min(self.len.saturating_sub(offset) as usize);
            for (i, b) in dst[..n].iter_mut().enumerate() { *b = (offset as usize + i + key as usize) as u8; }
            Some(n)
        }
    }

    #[test]
    fn a_stream_is_read_on_demand_and_clamped_to_its_length() {
        let mut t = Tree::new(31);
        let v = t.add_volume("Game", false);
        let reads = Rc::new(Cell::new(0));
        t.enable_streams();
        t.set_stream_source(alloc::boxed::Box::new(Pattern { len: 1 << 30, reads: reads.clone() }));
        let f = t.put_stream(v, "War3.mpq", Stream { len: 1000, key: 1 }).unwrap();
        assert_eq!(reads.get(), 0, "staging reads nothing");
        assert_eq!((t.file_len(f), t.is_file(f), t.is_stream(f), t.file_data(f)), (Some(1000), true, true, None));
        let mut buf = [0u8; 8];
        assert_eq!(t.read_at(f, 996, &mut buf), Some(4), "clamped to the stream's own length, not the source's");
        assert_eq!(&buf[..4], &[(996u32 + 1) as u8, (997u32 + 1) as u8, (998u32 + 1) as u8, (999u32 + 1) as u8]);
        assert_eq!(t.read_at(f, 1000, &mut buf), Some(0), "at the end: nothing, and the source is not asked");
        assert_eq!(reads.get(), 1);
        let bad = t.put_stream(v, "Broken", Stream { len: 10, key: 99 }).unwrap();
        assert_eq!(t.read_at(bad, 0, &mut buf), None, "a failing source is an error, never zeros");
    }

    #[test]
    fn a_tree_whose_engine_reads_in_memory_only_refuses_a_stream() {
        let mut t = Tree::new(31);
        let v = t.add_volume("Game", false);
        assert_eq!(t.put_stream(v, "War3.mpq", Stream { len: 1, key: 0 }), Err(VolErr::WrongType));
        t.enable_streams();
        let f = t.put_stream(v, "War3.mpq", Stream { len: 1, key: 0 }).unwrap();
        let mut b = [0u8; 1];
        assert_eq!(t.read_at(f, 0, &mut b), None, "NEGATIVE CONTROL: no source set, so no bytes");
        assert_eq!(t.put_file(v, "War3.mpq", alloc::vec![1]), Err(VolErr::WrongType), "a stream is not overwritten with memory bytes");
    }

    #[test]
    fn read_at_serves_in_memory_files_too() {
        let mut t = Tree::new(31);
        let v = t.add_volume("W", true);
        let f = t.put_file(v, "a", b"hello".to_vec()).unwrap();
        let mut b = [0u8; 8];
        assert_eq!((t.read_at(f, 3, &mut b), &b[..2]), (Some(2), &b"lo"[..]));
        assert_eq!(t.read_at(f, 9, &mut b), Some(0));
        assert_eq!(t.read_at(v, 0, &mut b), None, "a directory has no data fork");
    }

    #[test]
    fn a_snapshot_or_merge_of_a_stream_is_refused_by_name() {
        let mut t = Tree::new(31);
        let v = t.add_volume("Game", false);
        t.enable_streams();
        t.put_stream(v, "War3.mpq", Stream { len: 5, key: 0 }).unwrap();
        assert_eq!(crate::snapshot::decode(&crate::snapshot::encode(&t)).err(), Some(crate::snapshot::SnapErr::Stream { record: 1 }));
        let mut dst = Tree::new(31);
        dst.enable_streams();
        assert_eq!(crate::snapshot::merge(&mut dst, &t), Err(VolErr::WrongType));
        assert_eq!(crate::snapshot::merge_owned(&mut dst, t), Err(VolErr::WrongType));
    }
}
