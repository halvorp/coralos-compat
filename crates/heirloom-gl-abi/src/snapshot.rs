//! Draw-time snapshots of big-endian guest memory. GL reads client arrays when the draw is issued; this module copies
//! exactly the referenced ranges at that moment and converts them to the wire's little-endian order, so nothing the
//! guest writes afterwards can change a submitted draw. All offsets/lengths use checked arithmetic, and no allocation is
//! sized before the range has been proven to lie inside guest memory.
use alloc::vec::Vec;

/// Component types the fixed-function arrays use (GL enum values).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ElemType {
    I8 = 0x1400,
    U8 = 0x1401,
    I16 = 0x1402,
    U16 = 0x1403,
    I32 = 0x1404,
    U32 = 0x1405,
    F32 = 0x1406,
    /// `GL_DOUBLE`: the array types of GL 1.1 include doubles for vertices, normals, colours and texture coordinates.
    F64 = 0x140A,
}

impl ElemType {
    pub fn from_gl(v: u32) -> Option<ElemType> {
        Some(match v {
            0x1400 => ElemType::I8,
            0x1401 => ElemType::U8,
            0x1402 => ElemType::I16,
            0x1403 => ElemType::U16,
            0x1404 => ElemType::I32,
            0x1405 => ElemType::U32,
            0x1406 => ElemType::F32,
            0x140A => ElemType::F64,
            _ => return None,
        })
    }
    pub const fn bytes(self) -> usize {
        match self {
            ElemType::I8 | ElemType::U8 => 1,
            ElemType::I16 | ElemType::U16 => 2,
            ElemType::I32 | ElemType::U32 | ElemType::F32 => 4,
            ElemType::F64 => 8,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SnapshotError {
    /// A size or offset computation overflowed.
    Overflow,
    /// The referenced range is not inside guest memory.
    OutOfRange,
    /// Components per vertex must be 1..=4.
    BadSize(u8),
    /// Indices must be U8, U16 or U32.
    BadIndexType,
}

/// The role of an array (which `gl*Pointer` set it).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Role {
    Vertex = 0,
    Normal = 1,
    Color = 2,
    TexCoord = 3,
    /// `glEdgeFlagPointer`: one GLboolean (U8) per vertex, the edge flag of polygon mode GL_LINE/GL_POINT (GL 1.2 §2.6.2).
    EdgeFlag = 4,
}

impl Role {
    pub fn from_u8(v: u8) -> Option<Role> {
        Some(match v {
            0 => Role::Vertex,
            1 => Role::Normal,
            2 => Role::Color,
            3 => Role::TexCoord,
            4 => Role::EdgeFlag,
            _ => return None,
        })
    }
}

/// What `gl*Pointer` registered: a descriptor only. The bytes are NOT read until a draw (GL semantics).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Descriptor {
    pub role: Role,
    /// Texture unit (TexCoord) or 0.
    pub unit: u8,
    /// Offset of element 0 in guest memory.
    pub offset: usize,
    pub size: u8,
    pub ty: ElemType,
    /// Guest stride in bytes; 0 means tightly packed (GL).
    pub stride: u32,
}

/// RESIDENT ARRAYS (wire v4): an array's bytes may be KEPT by the worker under an id the engine chose, and a later draw whose
/// array the engine proved unchanged (guest RAM's write watch: no byte of its source written since it was kept) names the id
/// instead of carrying the bytes. The worker's [`Resident`] store puts them back, so every consumer after it sees whole data.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum Keep {
    /// The bytes travel and are not kept (today's path).
    #[default]
    Copy,
    /// The bytes travel and the worker keeps them under this id (replacing what it held there).
    Store(u32),
    /// The bytes do not travel: they are the ones kept under this id.
    Ref(u32),
}

/// Vertices `first .. first+count` of one array, packed (stride removed), little-endian.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArraySnapshot {
    pub role: Role,
    pub unit: u8,
    pub size: u8,
    pub ty: ElemType,
    pub first: u32,
    pub count: u32,
    pub data: Vec<u8>,
    /// How the bytes travelled: a decoded `Ref` has empty `data` until [`Resident::resolve`] fills it.
    pub keep: Keep,
}

/// The worker's resident arrays: the bytes each id was last stored with. Ids are the engine's (dense, reused once it dropped
/// one), so the store is a growable table indexed by id; it holds at most the engine's working set.
#[derive(Default)]
pub struct Resident {
    slots: Vec<Option<Vec<u8>>>,
    /// Bytes put back from the store (the wire bytes saved), and stores taken.
    pub refs: u64,
    pub ref_bytes: u64,
    pub stores: u64,
}

impl Resident {
    /// Keep every `Store` of `arrays` and fill every `Ref` from what is kept. A `Ref` to an id never stored, or whose kept bytes
    /// are not the length its record declares, is refused (the engine and the worker disagree: the draw cannot be trusted).
    pub fn resolve(&mut self, arrays: &mut [ArraySnapshot]) -> Result<(), crate::wire::WireError> {
        for a in arrays.iter_mut() {
            match a.keep {
                Keep::Copy => {}
                Keep::Store(id) => {
                    let i = id as usize;
                    if self.slots.len() <= i {
                        self.slots.resize_with(i.checked_add(1).ok_or(crate::wire::WireError::Overflow)?, || None);
                    }
                    match &mut self.slots[i] {
                        Some(v) => { v.clear(); v.extend_from_slice(&a.data); }
                        slot => *slot = Some(a.data.clone()),
                    }
                    self.stores += 1;
                }
                Keep::Ref(id) => {
                    let want = (a.count as usize).checked_mul(a.size as usize).and_then(|n| n.checked_mul(a.ty.bytes())).ok_or(crate::wire::WireError::Overflow)?;
                    let kept = self.slots.get(id as usize).and_then(|s| s.as_ref()).ok_or(crate::wire::WireError::BadValue("resident array id never stored"))?;
                    if kept.len() != want {
                        return Err(crate::wire::WireError::LengthMismatch);
                    }
                    a.data.clear();
                    a.data.extend_from_slice(kept);
                    self.refs += 1;
                    self.ref_bytes += want as u64;
                }
            }
        }
        Ok(())
    }
}

/// Snapshot vertices `first .. first+count` of `d` out of guest memory `mem`.
pub fn snapshot_array(mem: &[u8], d: &Descriptor, first: u32, count: u32) -> Result<ArraySnapshot, SnapshotError> {
    let mut out = ArraySnapshot { role: d.role, unit: d.unit, size: d.size, ty: d.ty, first, count, data: Vec::new(), keep: Keep::Copy };
    snapshot_array_into(mem, d, first, count, &mut out)?;
    Ok(out)
}

/// [`snapshot_array`] into `out`, reusing its buffer (a draw path that snapshots every frame allocates nothing once warm). On an
/// error `out` is unspecified.
pub fn snapshot_array_into(mem: &[u8], d: &Descriptor, first: u32, count: u32, out: &mut ArraySnapshot) -> Result<(), SnapshotError> {
    if !(1..=4).contains(&d.size) {
        return Err(SnapshotError::BadSize(d.size));
    }
    let vbytes = (d.size as usize).checked_mul(d.ty.bytes()).ok_or(SnapshotError::Overflow)?;
    let stride = if d.stride == 0 { vbytes } else { d.stride as usize };
    let total = (count as usize).checked_mul(vbytes).ok_or(SnapshotError::Overflow)?;
    (out.role, out.unit, out.size, out.ty, out.first, out.count, out.keep) = (d.role, d.unit, d.size, d.ty, first, count, Keep::Copy);
    let data = &mut out.data;
    data.clear();
    if count > 0 {
        // Prove the whole range (first vertex .. end of the last vertex) lies inside guest memory BEFORE allocating.
        let start = (first as usize).checked_mul(stride).and_then(|o| o.checked_add(d.offset)).ok_or(SnapshotError::Overflow)?;
        let last = (count as usize - 1).checked_mul(stride).and_then(|o| o.checked_add(start)).ok_or(SnapshotError::Overflow)?;
        let end = last.checked_add(vbytes).ok_or(SnapshotError::Overflow)?;
        if end > mem.len() {
            return Err(SnapshotError::OutOfRange);
        }
        data.reserve_exact(total);
        // copied and converted in one pass, no `memcpy` per vertex (wire_copy)
        crate::wire_copy::append_swapped(data, mem, start, stride, vbytes, count as usize, d.ty.bytes())?;
    }
    Ok(())
}

/// Indices of one draw: little-endian bytes plus the range they reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexSnapshot {
    pub ty: ElemType,
    pub count: u32,
    pub data: Vec<u8>,
    /// `(min, max)` of the indices; `None` for an empty draw.
    pub range: Option<(u32, u32)>,
}

/// Snapshot `count` indices of type `ty` at `offset` and compute the vertex range they reference (from the converted
/// values, never from the raw guest bytes).
pub fn snapshot_indices(mem: &[u8], offset: usize, count: u32, ty: ElemType) -> Result<IndexSnapshot, SnapshotError> {
    let mut out = IndexSnapshot { ty, count, data: Vec::new(), range: None };
    snapshot_indices_into(mem, offset, count, ty, &mut out)?;
    Ok(out)
}

/// [`snapshot_indices`] into `out`, reusing its buffer. On an error `out` is unspecified.
pub fn snapshot_indices_into(mem: &[u8], offset: usize, count: u32, ty: ElemType, out: &mut IndexSnapshot) -> Result<(), SnapshotError> {
    if !matches!(ty, ElemType::U8 | ElemType::U16 | ElemType::U32) {
        return Err(SnapshotError::BadIndexType);
    }
    let total = (count as usize).checked_mul(ty.bytes()).ok_or(SnapshotError::Overflow)?;
    let end = offset.checked_add(total).ok_or(SnapshotError::Overflow)?;
    if end > mem.len() {
        return Err(SnapshotError::OutOfRange);
    }
    let data = &mut out.data;
    data.clear();
    crate::wire_copy::append_swapped(data, mem, offset, total, total, 1, ty.bytes())?;
    let mut range: Option<(u32, u32)> = None;
    for c in data.chunks_exact(ty.bytes()) {
        let v = match ty {
            ElemType::U8 => u32::from(c[0]),
            ElemType::U16 => u32::from(u16::from_le_bytes([c[0], c[1]])),
            _ => u32::from_le_bytes([c[0], c[1], c[2], c[3]]),
        };
        range = Some(range.map_or((v, v), |(lo, hi)| (lo.min(v), hi.max(v))));
    }
    (out.ty, out.count, out.range) = (ty, count, range);
    Ok(())
}
