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
}

/// Reverse every `ty`-sized component in place: guest (big-endian) to wire (little-endian).
fn to_wire_order(ty: ElemType, bytes: &mut [u8]) {
    let n = ty.bytes();
    if n > 1 {
        for c in bytes.chunks_exact_mut(n) {
            c.reverse();
        }
    }
}

/// Snapshot vertices `first .. first+count` of `d` out of guest memory `mem`.
pub fn snapshot_array(mem: &[u8], d: &Descriptor, first: u32, count: u32) -> Result<ArraySnapshot, SnapshotError> {
    if !(1..=4).contains(&d.size) {
        return Err(SnapshotError::BadSize(d.size));
    }
    let vbytes = (d.size as usize).checked_mul(d.ty.bytes()).ok_or(SnapshotError::Overflow)?;
    let stride = if d.stride == 0 { vbytes } else { d.stride as usize };
    let total = (count as usize).checked_mul(vbytes).ok_or(SnapshotError::Overflow)?;
    let mut data = Vec::new();
    if count > 0 {
        // Prove the whole range (first vertex .. end of the last vertex) lies inside guest memory BEFORE allocating.
        let start = (first as usize).checked_mul(stride).and_then(|o| o.checked_add(d.offset)).ok_or(SnapshotError::Overflow)?;
        let last = (count as usize - 1).checked_mul(stride).and_then(|o| o.checked_add(start)).ok_or(SnapshotError::Overflow)?;
        let end = last.checked_add(vbytes).ok_or(SnapshotError::Overflow)?;
        if end > mem.len() {
            return Err(SnapshotError::OutOfRange);
        }
        data.reserve_exact(total);
        for i in 0..count as usize {
            let at = start + i * stride; // bounded by the proof above
            data.extend_from_slice(&mem[at..at + vbytes]);
        }
        to_wire_order(d.ty, &mut data);
    }
    Ok(ArraySnapshot { role: d.role, unit: d.unit, size: d.size, ty: d.ty, first, count, data })
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
    if !matches!(ty, ElemType::U8 | ElemType::U16 | ElemType::U32) {
        return Err(SnapshotError::BadIndexType);
    }
    let total = (count as usize).checked_mul(ty.bytes()).ok_or(SnapshotError::Overflow)?;
    let end = offset.checked_add(total).ok_or(SnapshotError::Overflow)?;
    if end > mem.len() {
        return Err(SnapshotError::OutOfRange);
    }
    let mut data = mem[offset..end].to_vec();
    to_wire_order(ty, &mut data);
    let mut range: Option<(u32, u32)> = None;
    for c in data.chunks_exact(ty.bytes()) {
        let v = match ty {
            ElemType::U8 => u32::from(c[0]),
            ElemType::U16 => u32::from(u16::from_le_bytes([c[0], c[1]])),
            _ => u32::from_le_bytes([c[0], c[1], c[2], c[3]]),
        };
        range = Some(range.map_or((v, v), |(lo, hi)| (lo.min(v), hi.max(v))));
    }
    Ok(IndexSnapshot { ty, count, data, range })
}
