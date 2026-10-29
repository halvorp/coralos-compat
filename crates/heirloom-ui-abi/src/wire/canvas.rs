//! The shared canvas arena: where a guest window's pixels travel between an engine process and its UI host, so they never ride
//! the frame ring (docs/heirloom/23-ENGINE-SPLIT.md 2.3, the CBP discipline). The host lends one region (Control::Canvases); the
//! engine writes a window's canvas at a 16-byte-aligned offset and names it in the Lease frame as a REGION number; the host reads it
//! back when it applies that Lease, before the PresentContent that follows. ONE layout, both sides: here.
//!
//! A canvas at `offset`: `width: u32`, `height: u32`, `count: u32` (<= width * height: a present carries its DAMAGE's pixels, the
//! canvas size names the whole), `0: u32`, then `count` pixels (`u32`, the UI ABI's 0x00RRGGBB, little-endian). Region `r` is at offset `(r - 1) * 16`; region 0 means "not leased".
use alloc::vec::Vec;

/// The frame ring each way between an engine and its host (the session ring's request and reply rings).
pub const SESSION_RING_LEN: u32 = 64 * 1024;
/// The canvas arena a host lends its engine: room for several windows of a 1024x768 guest screen.
pub const CANVAS_ARENA_LEN: u32 = 8 << 20;
pub const CANVAS_HEADER: usize = 16;
pub const CANVAS_ALIGN: usize = 16;

/// Why a canvas could not be written or read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanvasError {
    /// The offset is not 16-byte aligned, or region 0.
    BadRegion,
    /// Width or height negative, or more pixels than width x height.
    BadShape,
    /// The canvas does not fit the arena at that offset.
    OutOfArena,
}

/// The region number of a canvas at `offset` (16-byte aligned), or None.
pub fn region_of(offset: usize) -> Option<u32> {
    if offset % CANVAS_ALIGN != 0 {
        return None;
    }
    u32::try_from(offset / CANVAS_ALIGN).ok()?.checked_add(1)
}

/// The byte offset region `region` names (None for region 0).
pub fn offset_of(region: u32) -> Option<usize> {
    (region as usize).checked_sub(1)?.checked_mul(CANVAS_ALIGN)
}

/// The bytes `count` pixels take in the arena, header included, rounded up to the alignment.
pub fn canvas_bytes(count: usize) -> Option<usize> {
    let raw = count.checked_mul(4)?.checked_add(CANVAS_HEADER)?;
    raw.checked_add(CANVAS_ALIGN - 1).map(|n| n / CANVAS_ALIGN * CANVAS_ALIGN)
}

/// Write a canvas into `arena` at `offset`; the region number to put in the Lease frame.
pub fn write_canvas(arena: &mut [u8], offset: usize, width: i32, height: i32, pixels: &[u32]) -> Result<u32, CanvasError> {
    let region = region_of(offset).ok_or(CanvasError::BadRegion)?;
    let whole = (u32::try_from(width).map_err(|_| CanvasError::BadShape)? as usize).checked_mul(u32::try_from(height).map_err(|_| CanvasError::BadShape)? as usize);
    if whole.map_or(true, |n| pixels.len() > n) {
        return Err(CanvasError::BadShape);
    }
    let need = canvas_bytes(pixels.len()).ok_or(CanvasError::OutOfArena)?;
    let end = offset.checked_add(need).ok_or(CanvasError::OutOfArena)?;
    let dst = arena.get_mut(offset..end).ok_or(CanvasError::OutOfArena)?;
    dst[0..4].copy_from_slice(&(width as u32).to_le_bytes());
    dst[4..8].copy_from_slice(&(height as u32).to_le_bytes());
    dst[8..12].copy_from_slice(&(pixels.len() as u32).to_le_bytes());
    dst[12..16].copy_from_slice(&0u32.to_le_bytes());
    for (i, p) in pixels.iter().enumerate() {
        let at = CANVAS_HEADER + 4 * i;
        dst[at..at + 4].copy_from_slice(&p.to_le_bytes());
    }
    Ok(region)
}

/// Read the canvas region `region` names: `(width, height, pixels)`. Every field is checked against the arena.
pub fn read_canvas(arena: &[u8], region: u32) -> Result<(i32, i32, Vec<u32>), CanvasError> {
    let offset = offset_of(region).ok_or(CanvasError::BadRegion)?;
    let head = arena.get(offset..offset.checked_add(CANVAS_HEADER).ok_or(CanvasError::OutOfArena)?).ok_or(CanvasError::OutOfArena)?;
    let word = |i: usize| u32::from_le_bytes([head[i], head[i + 1], head[i + 2], head[i + 3]]);
    let (w, h, count) = (word(0), word(4), word(8) as usize);
    let (Ok(width), Ok(height)) = (i32::try_from(w), i32::try_from(h)) else { return Err(CanvasError::BadShape) };
    if (w as usize).checked_mul(h as usize).map_or(true, |n| count > n) {
        return Err(CanvasError::BadShape);
    }
    let start = offset + CANVAS_HEADER;
    let bytes = arena.get(start..start.checked_add(count.checked_mul(4).ok_or(CanvasError::OutOfArena)?).ok_or(CanvasError::OutOfArena)?).ok_or(CanvasError::OutOfArena)?;
    let pixels = bytes.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    Ok((width, height, pixels))
}
