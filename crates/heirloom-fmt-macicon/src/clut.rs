//! The Mac's standard colour tables - THE one home of them in this repository (the 68k engine's Color QuickDraw builds its
//! `'clut'` 4 and 8 from here; the icon decoders index them directly).
//!
//! The 8-bit table (`'clut'` id 8) is generated, not stored: a 6x6x6 cube of the levels FF CC 99 66 33 00 (red slowest), its
//! last cell (black) moved to the end, then ten-step ramps of red, green, blue and gray, then black. The 4-bit table (`'clut'`
//! id 4) is the sixteen colours Apple chose. Both are checked against Pomme's `SystemPalettes.cpp` (MIT; the facts only, no code
//! copied) by the 68k engine's color test over every entry.

/// The cube's component levels, and the ramps'.
const CUBE: [u32; 6] = [0xFF, 0xCC, 0x99, 0x66, 0x33, 0x00];
const RAMP: [u32; 10] = [0xEE, 0xDD, 0xBB, 0xAA, 0x88, 0x77, 0x55, 0x44, 0x22, 0x11];

const fn clut8() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 215 {
        t[i] = CUBE[i / 36] << 16 | CUBE[i / 6 % 6] << 8 | CUBE[i % 6];
        i += 1;
    }
    let mut ch = 0;
    while ch < 4 {
        let mut k = 0;
        while k < 10 {
            let l = RAMP[k];
            t[215 + ch * 10 + k] = match ch { 0 => l << 16, 1 => l << 8, 2 => l, _ => l << 16 | l << 8 | l };
            k += 1;
        }
        ch += 1;
    }
    t[255] = 0;
    t
}

/// The standard 8-bit system colour table as 0xRRGGBB, 256 entries (index 0 white, 255 black).
pub const CLUT8: [u32; 256] = clut8();

/// The standard 4-bit table as 0xRRGGBB: white, yellow, orange, red, magenta, purple, blue, cyan, green, dark green, brown,
/// tan, light gray, medium gray, dark gray, black.
pub const CLUT4: [u32; 16] = [
    0xFFFFFF, 0xFCF305, 0xFF6402, 0xDD0806, 0xF20884, 0x4600A5, 0x0000D4, 0x02ABEA,
    0x1FB714, 0x006411, 0x562C05, 0x90713A, 0xC0C0C0, 0x808080, 0x404040, 0x000000,
];
