//! Client pixel memory <-> the wire's tight little-endian pixels (GL 1.2 §3.6.4 / §4.3.2, "Pixel Storage Modes").
//!
//! A guest hands GL a pointer into BIG-ENDIAN memory laid out by its `GL_UNPACK_*` state (alignment, row length, skips,
//! swap bytes); `glReadPixels` writes the same way under `GL_PACK_*`. The ring carries neither the guest's state nor its
//! byte order: [`unpack`] copies exactly the `width x height` pixels out of guest memory into TIGHT rows (no padding, no
//! skips) with every multi-byte element converted to little-endian, and the worker uploads them with a tight pixel-store
//! state; [`pack`] is the inverse for a read back. Packed types (`GL_UNSIGNED_SHORT_5_6_5`, `GL_UNSIGNED_INT_8_8_8_8`,
//! ...) are one multi-byte WORD whose bit layout is defined on its value, so converting the word's byte order is the whole
//! conversion. Every length uses checked arithmetic and nothing is allocated before the range is proven to lie inside memory.
use alloc::vec::Vec;

pub const GL_COLOR_INDEX: u32 = 0x1900;
pub const GL_STENCIL_INDEX: u32 = 0x1901;
pub const GL_DEPTH_COMPONENT: u32 = 0x1902;
pub const GL_RGB: u32 = 0x1907;
pub const GL_RGBA: u32 = 0x1908;
pub const GL_BGR: u32 = 0x80E0;
pub const GL_BGRA: u32 = 0x80E1;

/// The pixel-storage state of one direction (`GL_UNPACK_*` for a download to GL, `GL_PACK_*` for a read back).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Store {
    pub alignment: u32,
    pub row_length: u32,
    pub skip_rows: u32,
    pub skip_pixels: u32,
    pub swap_bytes: bool,
}

impl Default for Store {
    /// GL's initial state: alignment 4, everything else off.
    fn default() -> Self {
        Store { alignment: 4, row_length: 0, skip_rows: 0, skip_pixels: 0, swap_bytes: false }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PixelError {
    /// The format enum is not one of the GL 1.2 pixel formats.
    BadFormat(u32),
    /// The type enum is not a GL 1.2 pixel type (`GL_BITMAP` is a separate path, not served here).
    BadType(u32),
    /// `GL_*_ALIGNMENT` must be 1, 2, 4 or 8.
    BadAlignment(u32),
    /// A packed type was paired with a format it does not describe.
    Mismatch { format: u32, ty: u32 },
    /// A size or offset computation overflowed.
    Overflow,
    /// The range is not inside memory.
    OutOfRange,
}

/// Bytes per element and per pixel for a (format, type) pair.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    /// Size of one component, or of the whole word for a packed type: the unit that changes byte order.
    pub elem: usize,
    /// Size of one pixel.
    pub pixel: usize,
}

fn components(format: u32) -> Option<usize> {
    Some(match format {
        GL_COLOR_INDEX | GL_STENCIL_INDEX | GL_DEPTH_COMPONENT | 0x1903..=0x1906 | 0x1909 => 1,
        0x190A => 2,
        GL_RGB | GL_BGR => 3,
        GL_RGBA | GL_BGRA => 4,
        _ => return None,
    })
}

/// The layout of `format` x `ty`, refusing a type/format pairing GL 1.2 does not define.
pub fn layout(format: u32, ty: u32) -> Result<Layout, PixelError> {
    let n = components(format).ok_or(PixelError::BadFormat(format))?;
    let one = |elem: usize| Layout { elem, pixel: elem * n };
    let packed = |elem: usize, ok: bool| if ok { Ok(Layout { elem, pixel: elem }) } else { Err(PixelError::Mismatch { format, ty }) };
    let rgb = format == GL_RGB;
    let rgba = format == GL_RGBA || format == GL_BGRA;
    match ty {
        0x1400 | 0x1401 => Ok(one(1)),
        0x1402 | 0x1403 => Ok(one(2)),
        0x1404..=0x1406 => Ok(one(4)),
        0x8032 | 0x8362 => packed(1, rgb),
        0x8363 | 0x8364 => packed(2, rgb),
        0x8033 | 0x8034 | 0x8365 | 0x8366 => packed(2, rgba),
        0x8035 | 0x8036 | 0x8367 | 0x8368 => packed(4, rgba),
        other => Err(PixelError::BadType(other)),
    }
}

/// Where the rows are: the byte offset of the first pixel from the pointer, and the distance between rows.
fn geometry(l: Layout, width: u32, s: &Store) -> Result<(usize, usize), PixelError> {
    let row = if s.row_length > 0 { s.row_length } else { width } as usize;
    let a = s.alignment as usize;
    if !matches!(a, 1 | 2 | 4 | 8) {
        return Err(PixelError::BadAlignment(s.alignment)); // the engine validates alignment before it records it; this is the backstop
    }
    let raw = row.checked_mul(l.pixel).ok_or(PixelError::Overflow)?;
    // rows start on an `alignment` boundary once the element is smaller than the alignment (GL 1.2 §3.6.4)
    let stride = if l.elem >= a { raw } else { raw.checked_add(a - 1).ok_or(PixelError::Overflow)? / a * a };
    let first = (s.skip_rows as usize).checked_mul(stride).and_then(|r| r.checked_add((s.skip_pixels as usize).checked_mul(l.pixel)?)).ok_or(PixelError::Overflow)?;
    Ok((first, stride))
}

fn convert(chunk: &mut [u8], elem: usize, guest_is_big_endian: bool) {
    if elem > 1 && guest_is_big_endian {
        chunk.reverse();
    }
}

/// `width x height` pixels at `mem[start..]` (guest, big-endian unless `store.swap_bytes`) as tight little-endian rows.
pub fn unpack(mem: &[u8], start: usize, width: u32, height: u32, format: u32, ty: u32, store: &Store) -> Result<Vec<u8>, PixelError> {
    let l = layout(format, ty)?;
    let (first, stride) = geometry(l, width, store)?;
    let row_bytes = (width as usize).checked_mul(l.pixel).ok_or(PixelError::Overflow)?;
    let total = row_bytes.checked_mul(height as usize).ok_or(PixelError::Overflow)?;
    if total == 0 {
        return Ok(Vec::new());
    }
    // the last row's end must be inside memory before anything is allocated
    let last = (height as usize - 1).checked_mul(stride).and_then(|r| r.checked_add(first)).and_then(|r| r.checked_add(start)).ok_or(PixelError::Overflow)?;
    if last.checked_add(row_bytes).ok_or(PixelError::Overflow)? > mem.len() {
        return Err(PixelError::OutOfRange);
    }
    let mut out = Vec::with_capacity(total);
    for r in 0..height as usize {
        let at = start + first + r * stride;
        out.extend_from_slice(&mem[at..at + row_bytes]);
    }
    for c in out.chunks_exact_mut(l.elem) {
        convert(c, l.elem, !store.swap_bytes);
    }
    Ok(out)
}

/// The inverse of [`unpack`] for a read back: tight little-endian `data` written, row by row, through `put(offset, bytes)`
/// at the guest layout `store` describes (offsets are from the pointer). `put` returns false when the guest range is invalid.
pub fn pack(data: &[u8], width: u32, height: u32, format: u32, ty: u32, store: &Store, mut put: impl FnMut(usize, &[u8]) -> bool) -> Result<(), PixelError> {
    let l = layout(format, ty)?;
    let (first, stride) = geometry(l, width, store)?;
    let row_bytes = (width as usize).checked_mul(l.pixel).ok_or(PixelError::Overflow)?;
    if row_bytes.checked_mul(height as usize).ok_or(PixelError::Overflow)? != data.len() {
        return Err(PixelError::OutOfRange);
    }
    if row_bytes == 0 {
        return Ok(());
    }
    for r in 0..height as usize {
        let mut row = data[r * row_bytes..(r + 1) * row_bytes].to_vec();
        for c in row.chunks_exact_mut(l.elem) {
            convert(c, l.elem, !store.swap_bytes);
        }
        let at = first.checked_add(r.checked_mul(stride).ok_or(PixelError::Overflow)?).ok_or(PixelError::Overflow)?;
        if !put(at, &row) {
            return Err(PixelError::OutOfRange);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgba_bytes_are_untouched_and_rows_are_tight() {
        // 3x2 RGB UNSIGNED_BYTE, alignment 4: each 9-byte row is padded to 12
        let mut mem = vec![0u8; 24];
        for (i, b) in mem.iter_mut().enumerate() {
            *b = i as u8;
        }
        let t = unpack(&mem, 0, 3, 2, GL_RGB, 0x1401, &Store::default()).unwrap();
        assert_eq!(t, [0, 1, 2, 3, 4, 5, 6, 7, 8, 12, 13, 14, 15, 16, 17, 18, 19, 20]);
    }

    #[test]
    fn alignment_one_has_no_padding_and_row_length_and_skips_move_the_window() {
        let mem: Vec<u8> = (0..64).collect();
        let s = Store { alignment: 1, row_length: 8, skip_rows: 1, skip_pixels: 2, swap_bytes: false };
        // one component, 3x2 window starting at row 1, pixel 2 of an 8-wide image
        let t = unpack(&mem, 0, 3, 2, 0x1903, 0x1401, &s).unwrap();
        assert_eq!(t, [10, 11, 12, 18, 19, 20]);
    }

    #[test]
    fn multi_byte_elements_become_little_endian_and_swap_bytes_cancels_it() {
        let mem = [0x12, 0x34, 0x56, 0x78];
        // two big-endian UNSIGNED_SHORT luminance pixels -> little-endian
        let t = unpack(&mem, 0, 2, 1, 0x1909, 0x1403, &Store { alignment: 1, ..Store::default() }).unwrap();
        assert_eq!(t, [0x34, 0x12, 0x78, 0x56]);
        // SWAP_BYTES: the guest data is already little-endian
        let t = unpack(&mem, 0, 2, 1, 0x1909, 0x1403, &Store { alignment: 1, swap_bytes: true, ..Store::default() }).unwrap();
        assert_eq!(t, mem);
    }

    #[test]
    fn a_packed_word_is_converted_as_one_unit() {
        // big-endian UNSIGNED_INT_8_8_8_8 word 0xAABBCCDD (R=AA G=BB B=CC A=DD) -> the LE word the host reads as the same value
        let t = unpack(&[0xAA, 0xBB, 0xCC, 0xDD], 0, 1, 1, GL_RGBA, 0x8035, &Store::default()).unwrap();
        assert_eq!(u32::from_le_bytes([t[0], t[1], t[2], t[3]]), 0xAABB_CCDD);
        // and 5_6_5 is a 2-byte word
        let t = unpack(&[0xF8, 0x1F], 0, 1, 1, GL_RGB, 0x8363, &Store::default()).unwrap();
        assert_eq!(u16::from_le_bytes([t[0], t[1]]), 0xF81F);
    }

    #[test]
    fn types_and_formats_outside_the_spec_pairing_are_refused() {
        assert_eq!(layout(GL_RGBA, 0x8363), Err(PixelError::Mismatch { format: GL_RGBA, ty: 0x8363 }));
        assert_eq!(layout(GL_RGB, 0x8035), Err(PixelError::Mismatch { format: GL_RGB, ty: 0x8035 }));
        assert_eq!(layout(0x1234, 0x1401), Err(PixelError::BadFormat(0x1234)));
        assert_eq!(layout(GL_RGBA, 0x1A00), Err(PixelError::BadType(0x1A00)));
        assert_eq!(layout(GL_BGRA, 0x8367).unwrap(), Layout { elem: 4, pixel: 4 });
    }

    #[test]
    fn a_range_outside_memory_is_refused_before_any_allocation() {
        let mem = [0u8; 16];
        assert_eq!(unpack(&mem, 0, 4, 4, GL_RGBA, 0x1401, &Store::default()), Err(PixelError::OutOfRange));
        assert_eq!(unpack(&mem, 0, u32::MAX, u32::MAX, GL_RGBA, 0x1406, &Store::default()), Err(PixelError::Overflow));
    }

    #[test]
    fn pack_is_the_inverse_of_unpack() {
        let mem: Vec<u8> = (0..48).collect();
        for (fmt, ty) in [(GL_RGBA, 0x1401u32), (GL_RGB, 0x8363), (GL_BGRA, 0x8367), (0x1909, 0x1403)] {
            let s = Store { alignment: 4, row_length: 5, skip_rows: 0, skip_pixels: 1, swap_bytes: false };
            let t = unpack(&mem, 0, 2, 2, fmt, ty, &s).unwrap();
            let mut out = vec![0u8; 48];
            pack(&t, 2, 2, fmt, ty, &s, |at, b| { out[at..at + b.len()].copy_from_slice(b); true }).unwrap();
            assert_eq!(unpack(&out, 0, 2, 2, fmt, ty, &s).unwrap(), t, "{fmt:#x} {ty:#x}");
        }
    }
}
