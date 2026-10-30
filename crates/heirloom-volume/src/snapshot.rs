//! The tree's own serialisation: a whole [`Tree`] as bytes and back, so a host can hand a staged guest volume to an engine
//! running in another process (docs/heirloom/23-ENGINE-SPLIT.md phase 1: Heirloom.app stages the bundle's files, mac68k.engine
//! runs the guest on them). ONE format, one home: here, beside the tree it describes.
//!
//! Layout (little-endian): `"HVS1"`, `max_name: u32`, `count: u32`, then `count` node records in pre-order (a parent always
//! before its children): `parent: u32` (the record index of the parent; `u32::MAX` = the pseudo-root, i.e. the record is a
//! volume), `kind: u8` (0 directory, 1 file, 2 a stream file: its bytes stay in storage the receiver reads itself, so its `data`
//! run is `len: u64` + the stream's LOCATOR in UTF-8, never the bytes; one without a locator is [`SnapErr::Stream`]), `writable: u8`
//! (volumes only; 0 elsewhere), `name`, `attrs: u32`, `comment`, `created_ms: i64`, `modified_ms: i64`, FInfo (`type[4] creator[4]
//! flags:u16 v:i16 h:i16`), `data` (files; a stream's length + locator), `rsrc`. A tree holding a stream decodes with streams
//! enabled; the receiver sets its own [`crate::StreamSource`].
//! Strings and byte runs are `len: u32` + bytes. Every length is checked against what remains before anything is allocated;
//! a malformed snapshot is a named [`SnapErr`], never a panic and never a partial tree.
use crate::{FInfo, Kind, NodeId, Tree, ROOT};
use alloc::string::String;
use alloc::vec::Vec;

const MAGIC: &[u8; 4] = b"HVS1";
const VOLUME_PARENT: u32 = u32::MAX;

/// Why a snapshot was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapErr {
    BadMagic,
    Truncated(&'static str),
    BadUtf8(&'static str),
    /// A record's parent is not an earlier directory record.
    BadParent { record: u32 },
    BadKind { record: u32, kind: u8 },
    /// A stream file record ([`crate::Kind::Stream`]) without its length and locator: nothing says where its bytes are.
    Stream { record: u32 },
    /// The tree refused the node (an empty, over-long or repeated name).
    Refused { record: u32 },
    TrailingBytes,
}

fn put_u32(out: &mut Vec<u8>, v: u32) { out.extend_from_slice(&v.to_le_bytes()); }
fn put_bytes(out: &mut Vec<u8>, b: &[u8]) { put_u32(out, b.len() as u32); out.extend_from_slice(b); }

/// The whole tree as bytes.
pub fn encode(tree: &Tree) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    put_u32(&mut out, tree.max_name as u32);
    let count_at = out.len();
    put_u32(&mut out, 0);
    let mut count = 0u32;
    // Pre-order walk with an explicit stack: (node, its record's parent index).
    let mut stack: Vec<(NodeId, u32)> = tree.children(ROOT).iter().rev().map(|&v| (v, VOLUME_PARENT)).collect();
    while let Some((id, parent)) = stack.pop() {
        let n = tree.get(id);
        let me = count;
        count += 1;
        put_u32(&mut out, parent);
        out.push(match n.kind { Kind::Dir(_) => 0, Kind::File(_) => 1, Kind::Stream(_) => 2 });
        out.push((parent == VOLUME_PARENT && n.writable) as u8);
        put_bytes(&mut out, n.name.as_bytes());
        put_u32(&mut out, n.attrs);
        put_bytes(&mut out, n.comment.as_bytes());
        out.extend_from_slice(&n.created_ms.to_le_bytes());
        out.extend_from_slice(&n.modified_ms.to_le_bytes());
        out.extend_from_slice(&n.finfo.file_type);
        out.extend_from_slice(&n.finfo.creator);
        out.extend_from_slice(&n.finfo.flags.to_le_bytes());
        out.extend_from_slice(&n.finfo.v.to_le_bytes());
        out.extend_from_slice(&n.finfo.h.to_le_bytes());
        match (&n.kind, tree.stream_locator(id)) {
            (Kind::Stream(s), Some(locator)) => {
                put_u32(&mut out, (8 + locator.len()) as u32);
                out.extend_from_slice(&s.len.to_le_bytes());
                out.extend_from_slice(locator.as_bytes());
            }
            _ => put_bytes(&mut out, tree.file_data(id).unwrap_or(&[])),
        }
        put_bytes(&mut out, &n.rsrc);
        for &c in tree.children(id).iter().rev() {
            stack.push((c, me));
        }
    }
    out[count_at..count_at + 4].copy_from_slice(&count.to_le_bytes());
    out
}

struct Rd<'a> { b: &'a [u8], at: usize }

impl<'a> Rd<'a> {
    fn take(&mut self, n: usize, what: &'static str) -> Result<&'a [u8], SnapErr> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.b.len()).ok_or(SnapErr::Truncated(what))?;
        let s = &self.b[self.at..end];
        self.at = end;
        Ok(s)
    }
    fn arr<const N: usize>(&mut self, what: &'static str) -> Result<[u8; N], SnapErr> {
        let mut a = [0u8; N];
        a.copy_from_slice(self.take(N, what)?);
        Ok(a)
    }
    fn u32(&mut self, what: &'static str) -> Result<u32, SnapErr> { Ok(u32::from_le_bytes(self.arr(what)?)) }
    fn bytes(&mut self, what: &'static str) -> Result<&'a [u8], SnapErr> {
        let n = self.u32(what)? as usize;
        self.take(n, what)
    }
    fn string(&mut self, what: &'static str) -> Result<String, SnapErr> {
        core::str::from_utf8(self.bytes(what)?).map(String::from).map_err(|_| SnapErr::BadUtf8(what))
    }
}

/// A tree from [`encode`]'s bytes. Node ids are the new tree's own; names, kinds, contents and metadata are the snapshot's.
pub fn decode(bytes: &[u8]) -> Result<Tree, SnapErr> {
    let mut r = Rd { b: bytes, at: 0 };
    if r.take(4, "magic")? != MAGIC {
        return Err(SnapErr::BadMagic);
    }
    let max_name = r.u32("max_name")? as usize;
    let count = r.u32("count")?;
    // Every record is at least 56 bytes: refuse an impossible count before reserving anything.
    if count as usize > (bytes.len() - r.at) / 56 {
        return Err(SnapErr::Truncated("record count"));
    }
    let mut tree = Tree::new(max_name);
    let mut ids: Vec<NodeId> = Vec::with_capacity(count as usize);
    for record in 0..count {
        let parent = r.u32("parent")?;
        let kind = r.take(1, "kind")?[0];
        let writable = r.take(1, "writable")?[0] != 0;
        let name = r.string("name")?;
        let attrs = r.u32("attrs")?;
        let comment = r.string("comment")?;
        let created_ms = i64::from_le_bytes(r.arr("created")?);
        let modified_ms = i64::from_le_bytes(r.arr("modified")?);
        let finfo = FInfo {
            file_type: r.arr("finfo")?,
            creator: r.arr("finfo")?,
            flags: u16::from_le_bytes(r.arr("finfo")?),
            v: i16::from_le_bytes(r.arr("finfo")?),
            h: i16::from_le_bytes(r.arr("finfo")?),
        };
        let data = r.bytes("data")?;
        let rsrc = r.bytes("rsrc")?;
        let id = if parent == VOLUME_PARENT {
            if kind != 0 || name.is_empty() || tree.volume(&name).is_some() {
                return Err(if kind != 0 { SnapErr::BadKind { record, kind } } else { SnapErr::Refused { record } });
            }
            tree.add_volume(&name, writable)
        } else {
            let dir = *ids.get(parent as usize).ok_or(SnapErr::BadParent { record })?;
            if !tree.is_dir(dir) {
                return Err(SnapErr::BadParent { record });
            }
            match kind {
                0 => tree.create(dir, &name, Kind::Dir(Vec::new())),
                1 => tree.create(dir, &name, Kind::File(data.to_vec())),
                2 => {
                    let (len, locator) = data.split_at_checked(8).ok_or(SnapErr::Stream { record })?;
                    let locator = core::str::from_utf8(locator).map_err(|_| SnapErr::BadUtf8("stream locator"))?;
                    if locator.is_empty() {
                        return Err(SnapErr::Stream { record });
                    }
                    tree.enable_streams();
                    tree.put_stream(dir, &name, u64::from_le_bytes(len.try_into().expect("8 bytes")), locator)
                }
                _ => return Err(SnapErr::BadKind { record, kind }),
            }
            .map_err(|_| SnapErr::Refused { record })?
        };
        let n = tree.get_mut(id);
        n.attrs = attrs;
        n.comment = comment;
        n.created_ms = created_ms;
        n.modified_ms = modified_ms;
        n.finfo = finfo;
        n.rsrc = rsrc.to_vec();
        ids.push(id);
    }
    if r.at != bytes.len() {
        return Err(SnapErr::TrailingBytes);
    }
    Ok(tree)
}

/// Put every volume, directory and file of `src` into `dst`, keeping what `dst` already holds (an engine's own pre-populated
/// volumes): a volume or directory that exists is reused, a file that exists has its forks and metadata replaced. A file where
/// `dst` has a directory (or the reverse) is refused with [`crate::VolErr::WrongType`] and nothing after it is merged. A stream file in `src`
/// becomes a stream of `dst` with the same length and locator (read through `dst`'s own source); a `dst` whose engine reads in
/// memory only refuses it the same way.
pub fn merge(dst: &mut Tree, src: &Tree) -> Result<(), crate::VolErr> {
    let mut stack: Vec<(NodeId, NodeId)> = Vec::new(); // (src node, dst parent; ROOT = it is a volume)
    for &v in src.children(ROOT).iter().rev() {
        stack.push((v, ROOT));
    }
    while let Some((id, parent)) = stack.pop() {
        let n = src.get(id);
        let target = if parent == ROOT {
            dst.add_volume(&n.name, n.writable)
        } else {
            match (&n.kind, dst.find_child(parent, &n.name)) {
                (Kind::Dir(_), Some(d)) if dst.is_dir(d) => d,
                (Kind::Dir(_), None) => dst.create(parent, &n.name, Kind::Dir(Vec::new()))?,
                (Kind::File(data), _) => dst.put_file(parent, &n.name, data.clone())?,
                (Kind::Stream(s), _) => dst.put_stream(parent, &n.name, s.len, src.stream_locator(id).ok_or(crate::VolErr::NotFound)?)?,
                (Kind::Dir(_), Some(_)) => return Err(crate::VolErr::WrongType),
            }
        };
        let t = dst.get_mut(target);
        t.attrs = n.attrs;
        t.comment = n.comment.clone();
        t.created_ms = n.created_ms;
        t.modified_ms = n.modified_ms;
        t.finfo = n.finfo;
        t.rsrc = n.rsrc.clone();
        for &c in src.children(id).iter().rev() {
            stack.push((c, target));
        }
    }
    Ok(())
}

/// [`merge`], consuming `src`: every file's bytes and resource fork MOVE into `dst` instead of being copied, so a staged tree the
/// size of an install (Hero of Sparta's is 76 MB) is held once, not twice, while it is merged.
pub fn merge_owned(dst: &mut Tree, mut src: Tree) -> Result<(), crate::VolErr> {
    let mut stack: Vec<(NodeId, NodeId)> = Vec::new();
    for &v in src.children(ROOT).iter().rev() {
        stack.push((v, ROOT));
    }
    while let Some((id, parent)) = stack.pop() {
        let is_dir = src.is_dir(id);
        let name: String = String::from(src.name(id));
        let target = if parent == ROOT {
            dst.add_volume(&name, src.get(id).writable)
        } else {
            match (is_dir, dst.find_child(parent, &name)) {
                (true, Some(d)) if dst.is_dir(d) => d,
                (true, None) => dst.create(parent, &name, Kind::Dir(Vec::new()))?,
                (false, _) if src.is_stream(id) => {
                    let len = src.file_len(id).unwrap_or(0);
                    dst.put_stream(parent, &name, len, src.stream_locator(id).ok_or(crate::VolErr::NotFound)?)?
                }
                (false, _) => {
                    let data = src.file_data_mut(id).map(core::mem::take).unwrap_or_default();
                    dst.put_file(parent, &name, data)?
                }
                (true, Some(_)) => return Err(crate::VolErr::WrongType),
            }
        };
        let n = src.get_mut(id);
        let (attrs, comment, created_ms, modified_ms, finfo, rsrc) =
            (n.attrs, core::mem::take(&mut n.comment), n.created_ms, n.modified_ms, n.finfo, core::mem::take(&mut n.rsrc));
        let t = dst.get_mut(target);
        t.attrs = attrs;
        t.comment = comment;
        t.created_ms = created_ms;
        t.modified_ms = modified_ms;
        t.finfo = finfo;
        t.rsrc = rsrc;
        for &c in src.children(id).iter().rev() {
            stack.push((c, target));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Tree {
        let mut t = Tree::new(31);
        let sys = t.add_volume("System", false);
        let work = t.add_volume("Work", true);
        let dir = t.mkdirs(sys, ["Fonts", "Bitmap"]).unwrap();
        let f = t.put_file(dir, "Geneva", alloc::vec![1, 2, 3]).unwrap();
        let n = t.get_mut(f);
        n.attrs = 0x80;
        n.comment = String::from("a font");
        n.created_ms = -5;
        n.modified_ms = 1_790_000_000_000;
        n.finfo = FInfo { file_type: *b"FFIL", creator: *b"DMOV", flags: 0x0100, v: -3, h: 7 };
        n.rsrc = alloc::vec![9; 300];
        t.put_file(work, "empty", Vec::new()).unwrap();
        t
    }

    /// Everything a node carries, walked by path (ids differ between two trees).
    fn census(t: &Tree) -> Vec<String> {
        let mut out = Vec::new();
        let mut stack: Vec<NodeId> = t.children(ROOT).to_vec();
        while let Some(id) = stack.pop() {
            let n = t.get(id);
            out.push(alloc::format!(
                "{:?} dir={} w={} attrs={} c={:?} {} {} {:?} data={:?} rsrc={}",
                t.path_components(id), t.is_dir(id), t.is_volume(id) && t.is_writable(id), n.attrs, n.comment, n.created_ms,
                n.modified_ms, n.finfo, t.file_data(id), n.rsrc.len()
            ));
            stack.extend_from_slice(t.children(id));
        }
        out.sort();
        out
    }

    #[test]
    fn a_tree_survives_the_round_trip_whole() {
        let t = sample();
        let back = decode(&encode(&t)).expect("decodes");
        assert_eq!(census(&back), census(&t));
        assert_eq!(back.max_name, 31);
        assert!(back.is_writable(back.volume("Work").unwrap()));
        assert!(!back.is_writable(back.volume("System").unwrap()));
    }

    #[test]
    fn a_merge_keeps_what_the_destination_had_and_adds_the_snapshot() {
        let mut dst = Tree::new(31);
        let own = dst.add_volume("Engine", false);
        dst.put_file(own, "System", alloc::vec![7]).unwrap();
        let sys = dst.add_volume("System", false);
        dst.put_file(sys, "old", alloc::vec![1]).unwrap();
        merge(&mut dst, &sample()).unwrap();
        assert_eq!(dst.file_data(dst.find_child(own, "System").unwrap()), Some(&[7u8][..]), "the engine's own volume is kept");
        assert!(dst.find_child(sys, "old").is_some(), "a file the snapshot does not name is kept");
        let mut only_src: Vec<String> = census(&dst);
        only_src.retain(|l| !l.contains("\"Engine\"") && !l.contains("\"old\""));
        assert_eq!(only_src, census(&sample()));
        let mut clash = Tree::new(31);
        let v = clash.add_volume("System", false);
        clash.put_file(v, "Fonts", alloc::vec![]).unwrap();
        assert_eq!(merge(&mut clash, &sample()), Err(crate::VolErr::WrongType));
    }

    #[test]
    fn an_owned_merge_gives_the_same_tree_as_a_copying_one() {
        let fresh = || {
            let mut dst = Tree::new(31);
            let own = dst.add_volume("Engine", false);
            dst.put_file(own, "System", alloc::vec![7]).unwrap();
            dst
        };
        let (mut copied, mut moved) = (fresh(), fresh());
        merge(&mut copied, &sample()).unwrap();
        merge_owned(&mut moved, sample()).unwrap();
        assert_eq!(census(&moved), census(&copied), "names, kinds, bytes, forks and metadata all arrive");
        let mut clash = Tree::new(31);
        let v = clash.add_volume("System", false);
        clash.put_file(v, "Fonts", alloc::vec![]).unwrap();
        assert_eq!(merge_owned(&mut clash, sample()), Err(crate::VolErr::WrongType));
    }

    #[test]
    fn every_truncation_is_refused_by_name_and_never_panics() {
        let bytes = encode(&sample());
        for cut in 0..bytes.len() {
            assert!(decode(&bytes[..cut]).is_err(), "a snapshot cut at {cut} of {} decoded", bytes.len());
        }
        let mut long = bytes.clone();
        long.push(0);
        assert_eq!(decode(&long).err(), Some(SnapErr::TrailingBytes));
        let mut bad = bytes;
        bad[0] = b'X';
        assert_eq!(decode(&bad).err(), Some(SnapErr::BadMagic));
    }

    #[test]
    fn a_forward_parent_and_a_file_parent_are_refused() {
        let mut t = Tree::new(31);
        let v = t.add_volume("V", false);
        t.put_file(v, "f", alloc::vec![1]).unwrap();
        let mut bytes = encode(&t);
        // Record 1 (the file) names itself as parent: not an earlier record.
        let rec1 = 12 + 4 + 1 + 1 + 4 + 1 + 4 + 4 + 8 + 8 + 14 + 4 + 4; // header + record 0 ("V", no data)
        bytes[rec1..rec1 + 4].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(decode(&bytes).err(), Some(SnapErr::BadParent { record: 1 }));
    }
}
