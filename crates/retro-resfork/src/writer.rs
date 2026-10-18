//! Build forks and containers (the inverse of [`crate::fork`] / [`crate::container`]): test fixtures and tools.
use crate::container::{crc16, AD_MAGIC, AD_VERSION};
use crate::OsType;
use alloc::vec::Vec;

struct Res { ty: OsType, id: i16, name: Option<Vec<u8>>, attrs: u8, data: Vec<u8> }

/// Accumulates resources and serialises them in the Resource Manager format.
#[derive(Default)]
pub struct ForkBuilder { res: Vec<Res> }

impl ForkBuilder {
    pub fn new() -> Self { Self::default() }
    pub fn add(mut self, ty: &[u8; 4], id: i16, name: Option<&str>, data: &[u8]) -> Self {
        self.res.push(Res { ty: *ty, id, name: name.map(|n| n.bytes().collect()), attrs: 0, data: data.to_vec() });
        self
    }
    /// Set the attribute byte of the resource added last.
    pub fn attrs(mut self, attrs: u8) -> Self {
        if let Some(r) = self.res.last_mut() { r.attrs = attrs; }
        self
    }

    pub fn build(&self) -> Vec<u8> {
        let mut types: Vec<OsType> = Vec::new();
        for r in &self.res { if !types.contains(&r.ty) { types.push(r.ty); } }
        // data area
        let mut data = Vec::new();
        let mut offs = Vec::new();
        for r in &self.res {
            offs.push(data.len() as u32);
            data.extend_from_slice(&(r.data.len() as u32).to_be_bytes());
            data.extend_from_slice(&r.data);
        }
        // names
        let mut names = Vec::new();
        let mut name_offs: Vec<u16> = Vec::new();
        for r in &self.res {
            match &r.name {
                Some(n) => { name_offs.push(names.len() as u16); names.push(n.len() as u8); names.extend_from_slice(n); }
                None => name_offs.push(0xFFFF),
            }
        }
        // map
        let tl_len = 2 + types.len() * 8;
        let mut map = alloc::vec![0u8; 28];
        let mut tl = Vec::new();
        tl.extend_from_slice(&(types.len() as u16).wrapping_sub(1).to_be_bytes());
        let mut refs = Vec::new();
        for ty in &types {
            let idx: Vec<usize> = (0..self.res.len()).filter(|&i| self.res[i].ty == *ty).collect();
            tl.extend_from_slice(ty);
            tl.extend_from_slice(&(idx.len() as u16 - 1).to_be_bytes());
            tl.extend_from_slice(&((tl_len + refs.len()) as u16).to_be_bytes());
            for i in idx {
                refs.extend_from_slice(&self.res[i].id.to_be_bytes());
                refs.extend_from_slice(&name_offs[i].to_be_bytes());
                refs.push(self.res[i].attrs);
                refs.extend_from_slice(&offs[i].to_be_bytes()[1..]);
                refs.extend_from_slice(&[0; 4]);
            }
        }
        let name_list_off = 28 + tl.len() + refs.len();
        map[24..26].copy_from_slice(&28u16.to_be_bytes());
        map[26..28].copy_from_slice(&(name_list_off as u16).to_be_bytes());
        map.extend_from_slice(&tl);
        map.extend_from_slice(&refs);
        map.extend_from_slice(&names);
        let mut out = Vec::new();
        for v in [16u32, 16 + data.len() as u32, data.len() as u32, map.len() as u32] { out.extend_from_slice(&v.to_be_bytes()); }
        out.extend_from_slice(&data);
        map[..16].copy_from_slice(&out[..16]);
        out.extend_from_slice(&map);
        out
    }
}

/// Wrap forks in a MacBinary II file (version bytes 129/129, CRC over the first 124 bytes).
pub fn macbinary(name: &str, file_type: &[u8; 4], creator: &[u8; 4], data: &[u8], rsrc: &[u8]) -> Vec<u8> {
    let mut h = alloc::vec![0u8; 128];
    let n = name.len().min(63);
    h[1] = n as u8;
    h[2..2 + n].copy_from_slice(&name.as_bytes()[..n]);
    h[65..69].copy_from_slice(file_type);
    h[69..73].copy_from_slice(creator);
    h[83..87].copy_from_slice(&(data.len() as u32).to_be_bytes());
    h[87..91].copy_from_slice(&(rsrc.len() as u32).to_be_bytes());
    h[122] = 129;
    h[123] = 129;
    let crc = crc16(&h[..124]);
    h[124..126].copy_from_slice(&crc.to_be_bytes());
    let mut out = h;
    out.extend_from_slice(data);
    out.resize(128 + data.len().div_ceil(128) * 128, 0);
    out.extend_from_slice(rsrc);
    out.resize(out.len().div_ceil(128) * 128, 0);
    out
}

/// An AppleDouble header file: a real-name entry (3) and a resource fork entry (2).
pub fn appledouble(name: &str, rsrc: &[u8]) -> Vec<u8> {
    let table_end = 26 + 2 * 12;
    let mut out = Vec::new();
    out.extend_from_slice(&AD_MAGIC.to_be_bytes());
    out.extend_from_slice(&AD_VERSION.to_be_bytes());
    out.extend_from_slice(&[0; 16]);
    out.extend_from_slice(&2u16.to_be_bytes());
    for (id, off, len) in [(3u32, table_end, name.len()), (2, table_end + name.len(), rsrc.len())] {
        out.extend_from_slice(&id.to_be_bytes());
        out.extend_from_slice(&(off as u32).to_be_bytes());
        out.extend_from_slice(&(len as u32).to_be_bytes());
    }
    out.extend_from_slice(name.as_bytes());
    out.extend_from_slice(rsrc);
    out
}
