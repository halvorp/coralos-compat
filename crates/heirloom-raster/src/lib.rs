//! Raster SEMANTICS for the heirloom engines, not display hardware: a chunky pen-index bitmap, rectangles and regions,
//! drawing modes, minterm blits and a bitmap-font renderer. No guest memory is touched here; the engines read guest
//! structs, call these, and publish the result. `no_std` + `alloc` so it runs in CoralOS user mode.
//!
//! Layout qualified: [`Bitmap`] is CHUNKY, one byte per pixel holding a pen index (depth 1..=8; bits above the depth
//! are always zero). Planar guest `struct BitMap`s are modelled by the engine as host-backed chunky bitmaps.
#![cfg_attr(not(test), no_std)]
extern crate alloc;

pub mod bitmap;
pub mod blit;
pub mod font;
pub mod paint;
pub mod rect;
pub mod region;
pub mod topaz8;

pub use bitmap::Bitmap;
pub use rect::Rect;
pub use region::Region;
