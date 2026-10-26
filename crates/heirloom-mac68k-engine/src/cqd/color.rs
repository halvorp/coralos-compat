//! RGB colours and the standard colour lookup tables of Color QuickDraw.
//!
//! A colour is an `RGBColor` (three 16-bit components, 6 bytes in guest memory). The standard 8-bit system table (the one
//! every 256-colour Macintosh screen starts with, `'clut'` id 8) is generated, not stored: Apple's layout is the
//! 6 x 6 x 6 colour cube of component levels `FF CC 99 66 33 00` (red slowest, blue fastest, white first) in entries
//! 0..=214, then four 10-step ramps (red, green, blue, gray; levels `EE DD BB AA 88 77 55 44 22 11`) in 215..=254, and
//! black in 255. The layout is checked against Pomme's `SystemPalettes.cpp` `clut8` (MIT; the facts only, no code copied)
//! by a unit test over every entry. The 4-bit table (`'clut'` id 4, [`CLUT4`]) is Pomme's `clut4` likewise.
use alloc::vec::Vec;
use heirloom_common::Refusal;
use heirloom_mem_flat32::GuestMem;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rgb { pub r: u16, pub g: u16, pub b: u16 }

impl Rgb {
    pub const WHITE: Rgb = Rgb { r: 0xFFFF, g: 0xFFFF, b: 0xFFFF };
    pub const BLACK: Rgb = Rgb { r: 0, g: 0, b: 0 };
    /// `0x00RRGGBB`, the host's pixel (the high byte of each 16-bit component).
    pub fn to_host(self) -> u32 { u32::from(self.r >> 8) << 16 | u32::from(self.g >> 8) << 8 | u32::from(self.b >> 8) }
    /// The component-wise complement (`InvertColor`).
    pub fn inverted(self) -> Rgb { Rgb { r: !self.r, g: !self.g, b: !self.b } }
    fn distance(self, o: Rgb) -> u64 {
        let d = |a: u16, b: u16| (i64::from(a) - i64::from(b)).unsigned_abs();
        d(self.r, o.r).pow(2) + d(self.g, o.g).pow(2) + d(self.b, o.b).pow(2)
    }
}

pub fn read_rgb(mem: &GuestMem, at: u32) -> Result<Rgb, Refusal> {
    let e = Refusal::mem("RGBColor");
    Ok(Rgb { r: mem.read_u16(at).map_err(&e)?, g: mem.read_u16(at.wrapping_add(2)).map_err(&e)?, b: mem.read_u16(at.wrapping_add(4)).map_err(&e)? })
}

pub fn write_rgb(mem: &mut GuestMem, at: u32, c: Rgb) -> Result<(), Refusal> {
    let e = Refusal::mem("RGBColor");
    mem.write_u16(at, c.r).map_err(&e)?;
    mem.write_u16(at.wrapping_add(2), c.g).map_err(&e)?;
    mem.write_u16(at.wrapping_add(4), c.b).map_err(&e)
}

/// The cube's component levels, and the ramps' (Pomme `clut8`).
const CUBE: [u8; 6] = [0xFF, 0xCC, 0x99, 0x66, 0x33, 0x00];
const RAMP: [u8; 10] = [0xEE, 0xDD, 0xBB, 0xAA, 0x88, 0x77, 0x55, 0x44, 0x22, 0x11];
fn wide(b: u8) -> u16 { u16::from(b) * 0x0101 }

/// The standard 8-bit system colour table, 256 entries.
pub fn clut8() -> Vec<Rgb> {
    let mut t = Vec::with_capacity(256);
    for i in 0..215usize { t.push(Rgb { r: wide(CUBE[i / 36]), g: wide(CUBE[i / 6 % 6]), b: wide(CUBE[i % 6]) }); }
    for ch in 0..4 { for &l in &RAMP {
        let l = wide(l);
        t.push(match ch { 0 => Rgb { r: l, g: 0, b: 0 }, 1 => Rgb { r: 0, g: l, b: 0 }, 2 => Rgb { r: 0, g: 0, b: l }, _ => Rgb { r: l, g: l, b: l } });
    } }
    t.push(Rgb::BLACK);
    t
}

/// The standard 4-bit table (Pomme `clut4`): white, yellow, orange, red, magenta, purple, blue, cyan, green, dark green,
/// brown, tan, light gray, medium gray, dark gray, black.
pub const CLUT4: [u32; 16] = [
    0xFFFFFF, 0xFCF305, 0xFF6402, 0xDD0806, 0xF20884, 0x4600A5, 0x0000D4, 0x02ABEA,
    0x1FB714, 0x006411, 0x562C05, 0x90713A, 0xC0C0C0, 0x808080, 0x404040, 0x000000,
];

pub fn clut4() -> Vec<Rgb> {
    CLUT4.iter().map(|&c| Rgb { r: wide((c >> 16) as u8), g: wide((c >> 8) as u8), b: wide(c as u8) }).collect()
}

/// The 1-bit table: white 0, black 1.
pub fn clut1() -> Vec<Rgb> { alloc::vec![Rgb::WHITE, Rgb::BLACK] }

/// The gray table of `depth` bits (`'clut'` ids 33, 34, 36, 40): `2^depth` evenly spaced grays, white first.
pub fn gray(depth: u8) -> Vec<Rgb> {
    let n = 1u32 << depth;
    (0..n).map(|i| { let v = (0xFFFF * (n - 1 - i) / (n - 1)) as u16; Rgb { r: v, g: v, b: v } }).collect()
}

/// The eight QuickDraw colour constants of `ForeColor`/`BackColor` (Inside Macintosh I-168): the code, its name, and the
/// entry of the standard 4-bit table it shows as (white is entry 0, black entry 15). The one home of these codes.
pub const STANDARD_COLORS: [(u32, &str, usize); 8] = [(33, "black", 15), (30, "white", 0), (205, "red", 3), (341, "green", 9), (409, "blue", 6), (273, "cyan", 7), (137, "magenta", 4), (69, "yellow", 1)];

/// A standard colour constant as RGB, or None for any other code.
pub fn standard_color(code: u32) -> Option<Rgb> {
    STANDARD_COLORS.iter().find(|(k, ..)| *k == code).map(|&(.., i)| clut4()[i])
}

/// The standard colour constant nearest `c` (what a plain, non-color `GrafPort` stores for an `RGBForeColor`).
pub fn nearest_standard_code(c: Rgb) -> u32 {
    STANDARD_COLORS.iter().map(|&(k, ..)| k).min_by_key(|&k| standard_color(k).map_or(u64::MAX, |s| s.distance(c))).unwrap_or(33)
}

/// The index of the entry of `table` that best matches `c`: an exact match (the lowest such index), else the nearest in
/// RGB space (squared 16-bit distance, lowest index on a tie). The real Color Manager answers through the device's inverse
/// table, whose 5-bit-per-channel quantisation can pick a different near colour for a colour not in the table;
/// for a colour that IS in the table both give the same index.
pub fn nearest(table: &[Rgb], c: Rgb) -> u8 {
    if let Some(i) = table.iter().position(|&e| e == c) { return i as u8; }
    table.iter().enumerate().min_by_key(|(_, e)| e.distance(c)).map_or(0, |(i, _)| i as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pomme SystemPalettes.cpp `clut8` as 0xRRGGBB: every one of the 256 entries.
    const POMME_CLUT8: [u32; 256] = [
        0xFFFFFF, 0xFFFFCC, 0xFFFF99, 0xFFFF66, 0xFFFF33, 0xFFFF00, 0xFFCCFF, 0xFFCCCC,
        0xFFCC99, 0xFFCC66, 0xFFCC33, 0xFFCC00, 0xFF99FF, 0xFF99CC, 0xFF9999, 0xFF9966,
        0xFF9933, 0xFF9900, 0xFF66FF, 0xFF66CC, 0xFF6699, 0xFF6666, 0xFF6633, 0xFF6600,
        0xFF33FF, 0xFF33CC, 0xFF3399, 0xFF3366, 0xFF3333, 0xFF3300, 0xFF00FF, 0xFF00CC,
        0xFF0099, 0xFF0066, 0xFF0033, 0xFF0000, 0xCCFFFF, 0xCCFFCC, 0xCCFF99, 0xCCFF66,
        0xCCFF33, 0xCCFF00, 0xCCCCFF, 0xCCCCCC, 0xCCCC99, 0xCCCC66, 0xCCCC33, 0xCCCC00,
        0xCC99FF, 0xCC99CC, 0xCC9999, 0xCC9966, 0xCC9933, 0xCC9900, 0xCC66FF, 0xCC66CC,
        0xCC6699, 0xCC6666, 0xCC6633, 0xCC6600, 0xCC33FF, 0xCC33CC, 0xCC3399, 0xCC3366,
        0xCC3333, 0xCC3300, 0xCC00FF, 0xCC00CC, 0xCC0099, 0xCC0066, 0xCC0033, 0xCC0000,
        0x99FFFF, 0x99FFCC, 0x99FF99, 0x99FF66, 0x99FF33, 0x99FF00, 0x99CCFF, 0x99CCCC,
        0x99CC99, 0x99CC66, 0x99CC33, 0x99CC00, 0x9999FF, 0x9999CC, 0x999999, 0x999966,
        0x999933, 0x999900, 0x9966FF, 0x9966CC, 0x996699, 0x996666, 0x996633, 0x996600,
        0x9933FF, 0x9933CC, 0x993399, 0x993366, 0x993333, 0x993300, 0x9900FF, 0x9900CC,
        0x990099, 0x990066, 0x990033, 0x990000, 0x66FFFF, 0x66FFCC, 0x66FF99, 0x66FF66,
        0x66FF33, 0x66FF00, 0x66CCFF, 0x66CCCC, 0x66CC99, 0x66CC66, 0x66CC33, 0x66CC00,
        0x6699FF, 0x6699CC, 0x669999, 0x669966, 0x669933, 0x669900, 0x6666FF, 0x6666CC,
        0x666699, 0x666666, 0x666633, 0x666600, 0x6633FF, 0x6633CC, 0x663399, 0x663366,
        0x663333, 0x663300, 0x6600FF, 0x6600CC, 0x660099, 0x660066, 0x660033, 0x660000,
        0x33FFFF, 0x33FFCC, 0x33FF99, 0x33FF66, 0x33FF33, 0x33FF00, 0x33CCFF, 0x33CCCC,
        0x33CC99, 0x33CC66, 0x33CC33, 0x33CC00, 0x3399FF, 0x3399CC, 0x339999, 0x339966,
        0x339933, 0x339900, 0x3366FF, 0x3366CC, 0x336699, 0x336666, 0x336633, 0x336600,
        0x3333FF, 0x3333CC, 0x333399, 0x333366, 0x333333, 0x333300, 0x3300FF, 0x3300CC,
        0x330099, 0x330066, 0x330033, 0x330000, 0x00FFFF, 0x00FFCC, 0x00FF99, 0x00FF66,
        0x00FF33, 0x00FF00, 0x00CCFF, 0x00CCCC, 0x00CC99, 0x00CC66, 0x00CC33, 0x00CC00,
        0x0099FF, 0x0099CC, 0x009999, 0x009966, 0x009933, 0x009900, 0x0066FF, 0x0066CC,
        0x006699, 0x006666, 0x006633, 0x006600, 0x0033FF, 0x0033CC, 0x003399, 0x003366,
        0x003333, 0x003300, 0x0000FF, 0x0000CC, 0x000099, 0x000066, 0x000033, 0xEE0000,
        0xDD0000, 0xBB0000, 0xAA0000, 0x880000, 0x770000, 0x550000, 0x440000, 0x220000,
        0x110000, 0x00EE00, 0x00DD00, 0x00BB00, 0x00AA00, 0x008800, 0x007700, 0x005500,
        0x004400, 0x002200, 0x001100, 0x0000EE, 0x0000DD, 0x0000BB, 0x0000AA, 0x000088,
        0x000077, 0x000055, 0x000044, 0x000022, 0x000011, 0xEEEEEE, 0xDDDDDD, 0xBBBBBB,
        0xAAAAAA, 0x888888, 0x777777, 0x555555, 0x444444, 0x222222, 0x111111, 0x000000,
    ];

    #[test]
    fn the_standard_8_bit_table_has_apples_layout() {
        let t = clut8();
        assert_eq!(t.len(), 256);
        for (i, (c, want)) in t.iter().zip(POMME_CLUT8).enumerate() { assert_eq!(c.to_host(), want, "entry {i}"); }
        assert_eq!(t[0], Rgb::WHITE);
        assert_eq!(t[1], Rgb { r: 0xFFFF, g: 0xFFFF, b: 0xCCCC });
        assert_eq!(t[6], Rgb { r: 0xFFFF, g: 0xCCCC, b: 0xFFFF });
        assert_eq!(t[36], Rgb { r: 0xCCCC, g: 0xFFFF, b: 0xFFFF });
        assert_eq!(t[214], Rgb { r: 0, g: 0, b: 0x3333 }, "the last cube entry before the ramps");
        assert_eq!((t[215].to_host(), t[224].to_host()), (0xEE0000, 0x110000), "red ramp");
        assert_eq!((t[225].to_host(), t[234].to_host()), (0x00EE00, 0x001100), "green ramp");
        assert_eq!((t[235].to_host(), t[244].to_host()), (0x0000EE, 0x000011), "blue ramp");
        assert_eq!((t[245].to_host(), t[254].to_host()), (0xEEEEEE, 0x111111), "gray ramp");
        assert_eq!(t[255], Rgb::BLACK);
        // every entry is distinct: a colour cube minus black plus ramps plus black
        let mut s: Vec<_> = t.iter().map(|c| c.to_host()).collect();
        s.sort_unstable();
        s.dedup();
        assert_eq!(s.len(), 256);
    }

    #[test]
    fn nearest_prefers_an_exact_entry_then_the_closest() {
        let t = clut8();
        assert_eq!(nearest(&t, Rgb::WHITE), 0);
        assert_eq!(nearest(&t, Rgb::BLACK), 255);
        assert_eq!(nearest(&t, Rgb { r: 0x9999, g: 0x6666, b: 0x3333 }), 36 * 2 + 6 * 3 + 4);
        assert_eq!(nearest(&t, Rgb { r: 0xFE00, g: 0xFE00, b: 0xFE00 }), 0, "near white is white (negative control: not black)");
        assert_eq!(nearest(&clut1(), Rgb { r: 0x4000, g: 0x4000, b: 0x4000 }), 1);
    }

    #[test]
    fn the_eight_colour_constants_and_inversion() {
        assert_eq!(standard_color(205).unwrap().to_host(), 0xDD0806);
        assert_eq!(standard_color(409).unwrap().to_host(), 0x0000D4);
        assert!(standard_color(7).is_none());
        assert_eq!(nearest_standard_code(Rgb { r: 0xDD00, g: 0x0900, b: 0x0600 }), 205);
        assert_eq!(Rgb::WHITE.inverted(), Rgb::BLACK);
        assert_eq!(gray(2).iter().map(|c| c.r).collect::<Vec<_>>(), [0xFFFF, 0xAAAA, 0x5555, 0]);
    }
}
