//! The Resource Manager file format.
//!
//! ```text
//! header (16): u32 dataOffset, u32 mapOffset, u32 dataLength, u32 mapLength
//! data area:   per resource  u32 length, bytes
//! map:         16-byte copy of the header, u32 next-map handle, u16 file ref, u16 attributes,
//!              u16 typeListOffset (from map start), u16 nameListOffset (from map start)
//! type list:   u16 (count-1), then per type  OSType, u16 (resources-1), u16 refListOffset (from type list start)
//! ref list:    per resource  i16 id, u16 nameOffset (0xFFFF = none; from name list start),
//!              u8 attributes, u24 dataOffset (from data area start), u32 reserved (handle)
//! name list:   Pascal strings
//! ```
use crate::{be16, be32, OsType};
use alloc::vec::Vec;
use core::fmt;
use core::ops::Range;

/// Why a fork was refused. The text names the part of the format that is wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForkError {
    /// The 16-byte header, the map or a table runs past the end of the fork.
    Truncated(&'static str),
    /// An offset or length points outside the part it must lie in.
    BadOffset(&'static str),
}
impl fmt::Display for ForkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ForkError::Truncated(w) => write!(f, "resource fork truncated: {w}"),
            ForkError::BadOffset(w) => write!(f, "resource fork has a bad offset: {w}"),
        }
    }
}
impl core::error::Error for ForkError {}

/// One resource: its identity, attributes and where its bytes (and name) are in the fork.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResEntry {
    pub ty: OsType,
    pub id: i16,
    /// resSysHeap 0x40, resPurgeable 0x20, resLocked 0x10, resProtected 0x08, resPreload 0x04, resChanged 0x02.
    pub attrs: u8,
    name: Option<Range<usize>>,
    data: Range<usize>,
    /// The bytes and name of a resource added or rewritten since the fork was parsed (they replace the ranges above).
    own: Option<Own>,
    removed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Own { data: Vec<u8>, name: Option<Vec<u8>> }

/// A validated resource fork. Resources are numbered `0..len()` in file order (type list order, then reference list
/// order); that index is the stable identity the engines key their handles on. Changing the fork never renumbers: a
/// resource added later takes the next index, a removed one keeps its index (and answers nothing), so a handle keyed on
/// an index stays right. [`ResFork::to_bytes`] writes the Resource Manager file format back out.
#[derive(Debug, Clone)]
pub struct ResFork { bytes: Vec<u8>, entries: Vec<ResEntry> }

impl ResFork {
    pub fn parse(bytes: Vec<u8>) -> Result<ResFork, ForkError> {
        let entries = index(&bytes)?;
        Ok(ResFork { bytes, entries })
    }
    /// The number of indices, removed resources included (an index never moves).
    pub fn len(&self) -> usize { self.entries.len() }
    pub fn is_empty(&self) -> bool { self.entries.iter().all(|e| e.removed) }
    pub fn entry(&self, i: usize) -> Option<&ResEntry> { self.entries.get(i).filter(|e| !e.removed) }
    /// Every index's entry, removed ones included (see [`ResFork::entry`] for one that is live).
    pub fn entries(&self) -> &[ResEntry] { &self.entries }
    /// The resource's bytes (without the 4-byte length prefix).
    pub fn data(&self, i: usize) -> Option<&[u8]> {
        let e = self.entry(i)?;
        Some(e.own.as_ref().map_or(&self.bytes[e.data.clone()], |o| &o.data))
    }
    /// The resource's name, if it has one.
    pub fn name(&self, i: usize) -> Option<&[u8]> {
        let e = self.entry(i)?;
        match &e.own { Some(o) => o.name.as_deref(), None => e.name.clone().map(|r| &self.bytes[r]) }
    }
    fn live(&self) -> impl Iterator<Item = usize> + '_ { (0..self.entries.len()).filter(|&i| !self.entries[i].removed) }
    /// Index of resource (`ty`, `id`).
    pub fn find(&self, ty: OsType, id: i16) -> Option<usize> {
        self.live().find(|&i| self.entries[i].ty == ty && self.entries[i].id == id)
    }
    /// Index of the first resource of `ty` whose name equals `name` byte for byte (GetNamedResource is
    /// case-insensitive on a real Mac: [`Self::find_named`] folds ASCII case).
    pub fn find_named(&self, ty: OsType, name: &[u8]) -> Option<usize> {
        self.live().find(|&i| self.entries[i].ty == ty && self.name(i).is_some_and(|n| n.eq_ignore_ascii_case(name)))
    }
    /// Number of resources of `ty`.
    pub fn count(&self, ty: OsType) -> usize { self.live().filter(|&i| self.entries[i].ty == ty).count() }
    /// The `n`th (1-based, like GetIndResource) resource of `ty`.
    pub fn nth(&self, ty: OsType, n: usize) -> Option<usize> {
        if n == 0 { return None; }
        self.live().filter(|&i| self.entries[i].ty == ty).nth(n - 1)
    }
    /// Distinct resource types, in first-appearance order.
    pub fn types(&self) -> Vec<OsType> {
        let mut t: Vec<OsType> = Vec::new();
        for i in self.live() { let ty = self.entries[i].ty; if !t.contains(&ty) { t.push(ty); } }
        t
    }

    /// Add a resource; its index is the next one. Nothing is checked about `(ty, id)` being new (the Resource Manager does not).
    pub fn add(&mut self, ty: OsType, id: i16, name: Option<&[u8]>, attrs: u8, data: Vec<u8>) -> usize {
        self.entries.push(ResEntry { ty, id, attrs, name: None, data: 0..0, own: Some(Own { data, name: name.map(<[u8]>::to_vec) }), removed: false });
        self.entries.len() - 1
    }
    /// Replace a resource's bytes. False when `i` is not a live resource.
    pub fn set_data(&mut self, i: usize, data: Vec<u8>) -> bool { self.own_mut(i).map(|o| o.data = data).is_some() }
    /// Set a resource's attribute byte.
    pub fn set_attrs(&mut self, i: usize, attrs: u8) -> bool { match self.entries.get_mut(i).filter(|e| !e.removed) { Some(e) => { e.attrs = attrs; true } None => false } }
    /// Change a resource's ID and, when `name` is given, its name (`Some(&[])` clears it).
    pub fn set_info(&mut self, i: usize, id: i16, name: Option<&[u8]>) -> bool {
        let Some(o) = self.own_mut(i) else { return false };
        if let Some(n) = name { o.name = if n.is_empty() { None } else { Some(n.to_vec()) }; }
        self.entries[i].id = id;
        true
    }
    /// Remove a resource (its index stays, empty). False when it was not live.
    pub fn remove(&mut self, i: usize) -> bool { match self.entries.get_mut(i).filter(|e| !e.removed) { Some(e) => { e.removed = true; true } None => false } }

    /// The entry's owned copy, made from the parsed bytes the first time it is needed.
    fn own_mut(&mut self, i: usize) -> Option<&mut Own> {
        if self.entries.get(i)?.removed { return None; }
        if self.entries[i].own.is_none() {
            let (data, name) = (self.data(i)?.to_vec(), self.name(i).map(<[u8]>::to_vec));
            self.entries[i].own = Some(Own { data, name });
        }
        self.entries[i].own.as_mut()
    }

    /// The fork in the Resource Manager file format: one type list in first-appearance order, each type's references sorted by ID
    /// (the order the Resource Manager keeps them in), the data area in that order.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = crate::writer::ForkBuilder::new();
        for ty in self.types() {
            let mut of_type: Vec<usize> = self.live().filter(|&i| self.entries[i].ty == ty).collect();
            of_type.sort_by_key(|&i| self.entries[i].id);
            for i in of_type { b = b.add_raw(&ty, self.entries[i].id, self.name(i), self.entries[i].attrs, self.data(i).unwrap_or(&[])); }
        }
        b.build()
    }
}

fn index(b: &[u8]) -> Result<Vec<ResEntry>, ForkError> {
    let t = ForkError::Truncated;
    let o = ForkError::BadOffset;
    let (data_off, map_off, data_len, map_len) = (
        be32(b, 0).ok_or(t("header"))? as usize, be32(b, 4).ok_or(t("header"))? as usize,
        be32(b, 8).ok_or(t("header"))? as usize, be32(b, 12).ok_or(t("header"))? as usize);
    let in_file = |off: usize, len: usize| off.checked_add(len).is_some_and(|e| e <= b.len());
    if !in_file(data_off, data_len) { return Err(o("data area outside the fork")); }
    if !in_file(map_off, map_len) { return Err(o("map outside the fork")); }
    if map_len < 28 { return Err(t("map shorter than its 28-byte fixed part")); }
    let map = &b[map_off..map_off + map_len];
    let tl_off = be16(map, 24).ok_or(t("map"))? as usize;
    let nl_off = be16(map, 26).ok_or(t("map"))? as usize;
    if tl_off < 28 || tl_off + 2 > map_len { return Err(o("type list offset")); }
    if nl_off > map_len { return Err(o("name list offset")); }
    let n_types = be16(map, tl_off).ok_or(t("type list"))?.wrapping_add(1) as usize; // 0xFFFF = no types
    let mut out = Vec::new();
    for ti in 0..n_types {
        let at = tl_off + 2 + ti * 8;
        if at + 8 > map_len { return Err(t("type list entry")); }
        let ty: OsType = map[at..at + 4].try_into().unwrap();
        let n_res = be16(map, at + 4).unwrap().wrapping_add(1) as usize;
        let rl_off = tl_off + be16(map, at + 6).unwrap() as usize;
        for ri in 0..n_res {
            let r = rl_off + ri * 12;
            if r + 12 > map_len { return Err(o("reference list runs past the map")); }
            let id = be16(map, r).unwrap() as i16;
            let name_off = be16(map, r + 2).unwrap();
            let attrs = map[r + 4];
            let d = u32::from_be_bytes([0, map[r + 5], map[r + 6], map[r + 7]]) as usize;
            // data: u32 length at data_off + d
            let len = be32(b, data_off.checked_add(d).ok_or(o("data offset"))?).ok_or(t("resource length"))? as usize;
            let start = data_off + d + 4;
            if d as u64 + 4 + len as u64 > data_len as u64 { return Err(o("resource data past the data area")); }
            let name = if name_off == 0xFFFF { None } else {
                let n = nl_off + name_off as usize;
                let l = *map.get(n).ok_or(o("name offset"))? as usize;
                if n + 1 + l > map_len { return Err(o("name runs past the map")); }
                Some(map_off + n + 1..map_off + n + 1 + l)
            };
            out.push(ResEntry { ty, id, attrs, name, data: start..start + len, own: None, removed: false });
        }
    }
    Ok(out)
}
