//! Direct colour (Inside Macintosh: Imaging With QuickDraw (IWQ), "Color QuickDraw": PixMap `pixelType` 16 = RGBDirect): the two
//! direct pixel formats, colour <-> pixel, the inverse table that maps colours to an indexed device's entries, the arithmetic
//! transfer modes, and the ONE depth converter every copy uses (CopyBits, CopyMask, CopyDeepMask, PICT bits, colour patterns).
//!
//! * 16 bits: `x1R5G5B5` (`cmpCount` 3, `cmpSize` 5, bit 15 unused). 32 bits: `x8R8G8B8` (`cmpSize` 8, the high byte unused).
//! * Colour -> pixel takes the top `cmpSize` bits of each 16-bit `RGBColor` component; pixel -> colour REPLICATES the component's
//!   bits to fill 16 (5 bits `abcde` -> `abcdeabcdeabcdea`, 8 bits `c` -> `c * 0x0101`) - IWQ "Direct devices", and Executor's
//!   `rgbutil.cpp` `make_component_map` (MIT; the rule, not its code).
//! * The inverse table (IWQ / IM V-137 "Color Manager": `MakeITable`, `ITab`): `2^(3 res)` one-byte cells indexed by the top `res`
//!   bits of red, green, blue (`r << 2res | g << res | b`, Executor `ITAB_LOOKUP`). A cell holds the entry NEAREST its representative
//!   colour (its `res` bits replicated to 16), squared 16-bit RGB distance, lowest index on a tie. Apple's MakeITable is a flood fill
//!   approximating the same rule; this is the exact rule, so it is deterministic and testable.
//! * Arithmetic modes 32..39 (IWQ pp. 4-38..4-40; the per-component definitions cross-checked against Executor `qIMVxfer.cpp`
//!   (MIT) and systemless `quickdraw.rs` (GPL, facts only)): computed on 16-bit components; see [`arith`].
//!
//! Every conversion is chosen ONCE per call ([`Conv::new`]) and then runs over whole rows ([`Conv::row`]): a table lookup, a
//! shift-and-replicate, or one inverse-table read per pixel, never a per-pixel decision of what kind of conversion to do.
use super::color::Rgb;
use alloc::vec::Vec;

/// The direct depths.
pub fn is_direct(depth: u8) -> bool { depth == 16 || depth == 32 }

/// The 5- and 8-bit component widths of the direct depths.
fn cmp_size(depth: u8) -> u32 { if depth == 16 { 5 } else { 8 } }

/// Replicate the top `bits` bits of `c` (already shifted down: `c < 2^bits`) to a 16-bit component.
pub fn widen(c: u32, bits: u32) -> u16 {
    let mut v = c << (16 - bits);
    let mut s = bits;
    while s < 16 { v |= v >> s; s *= 2; }
    v as u16
}

/// The direct pixel of `c` at `depth` (16 or 32): the top `cmpSize` bits of each component, the unused bits 0.
pub fn pixel_of(depth: u8, c: Rgb) -> u32 {
    let n = cmp_size(depth);
    let t = |v: u16| u32::from(v) >> (16 - n);
    (t(c.r) << (2 * n)) | (t(c.g) << n) | t(c.b)
}

/// The colour of a direct pixel at `depth` (16 or 32), components replicated to 16 bits.
pub fn rgb_of(depth: u8, px: u32) -> Rgb {
    let n = cmp_size(depth);
    let m = (1 << n) - 1;
    Rgb { r: widen((px >> (2 * n)) & m, n), g: widen((px >> n) & m, n), b: widen(px & m, n) }
}

/// An inverse table: colour -> the nearest entry of one colour table, quantised to `res` bits per component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ITab { pub res: u8, pub cells: Vec<u8> }

impl ITab {
    /// Build the table of `table` at resolution `res` (3, 4 or 5 bits; `None` outside that, as MakeITable's range).
    pub fn build(table: &[Rgb], res: u8) -> Option<ITab> {
        if !(3..=5).contains(&res) || table.is_empty() || table.len() > 256 { return None; }
        let n = 1u32 << res;
        let mut cells = Vec::with_capacity((n * n * n) as usize);
        for r in 0..n { for g in 0..n { for b in 0..n {
            let c = Rgb { r: widen(r, u32::from(res)), g: widen(g, u32::from(res)), b: widen(b, u32::from(res)) };
            cells.push(super::color::nearest(table, c));
        } } }
        Some(ITab { res, cells })
    }
    /// The entry for `c`.
    pub fn lookup(&self, c: Rgb) -> u8 {
        let s = 16 - u32::from(self.res);
        let (r, g, b) = (u32::from(c.r) >> s, u32::from(c.g) >> s, u32::from(c.b) >> s);
        self.cells[((r << (2 * u32::from(self.res))) | (g << u32::from(self.res)) | b) as usize]
    }
}

/// What a pixel value of one target means: a 1-bit map (0 white, 1 black), an indexed one (its colour table), a direct one.
#[derive(Debug, Clone, Copy)]
pub enum Space<'a> { Mono, Indexed(&'a [Rgb]), Direct(u8) }

impl Space<'_> {
    /// The colour of pixel `p`.
    pub fn rgb(&self, p: u32) -> Rgb {
        match self {
            Space::Mono => if p == 0 { Rgb::WHITE } else { Rgb::BLACK },
            Space::Indexed(t) => t.get(p as usize).copied().unwrap_or(Rgb::BLACK),
            Space::Direct(d) => rgb_of(*d, p),
        }
    }
}

/// Rec. 601 luma of a colour, 16 bits (what decides black or white when a deep image goes to a 1-bit map).
pub fn luminance(c: Rgb) -> u32 { (u32::from(c.r) * 299 + u32::from(c.g) * 587 + u32::from(c.b) * 114) / 1000 }

/// A depth conversion, chosen once per copy.
pub enum Conv<'a> {
    /// Source pixel value -> destination pixel value through a table (any indexed or 1-bit source; at most 256 entries).
    Lut(Vec<u32>),
    /// 16-bit -> 32-bit direct: replicate each 5-bit component to 8 bits.
    D16To32,
    /// 32-bit -> 16-bit direct: the top 5 bits of each component.
    D32To16,
    /// Direct -> an indexed destination through its inverse table.
    ToIndexed(u8, &'a ITab),
    /// Direct -> 1 bit: black below half luminance.
    ToMono(u8),
    /// Same values both sides.
    Same,
}

impl<'a> Conv<'a> {
    /// The conversion from `src` to `dst`. `fg`/`bg` are the destination pixels a 1-bit source's black/white become (IWQ
    /// "colorizing"); `itab` is the destination's inverse table (needed only for a direct source into an indexed destination).
    pub fn new(src: Space, src_depth: u8, dst: Space, fg: u32, bg: u32, itab: Option<&'a ITab>) -> Option<Conv<'a>> {
        Some(match (src, dst) {
            (Space::Direct(s), Space::Direct(d)) => if s == d { Conv::Same } else if s == 16 { Conv::D16To32 } else { Conv::D32To16 },
            (Space::Direct(s), Space::Mono) => Conv::ToMono(s),
            (Space::Direct(s), Space::Indexed(_)) => Conv::ToIndexed(s, itab?),
            (Space::Mono, Space::Mono) => Conv::Same,
            (Space::Mono, _) => Conv::Lut(alloc::vec![bg, fg]),
            (Space::Indexed(st), _) => {
                let n = 1usize << src_depth.min(8);
                Conv::Lut((0..n).map(|p| {
                    let c = st.get(p).copied().unwrap_or(Rgb::BLACK);
                    match dst {
                        Space::Mono => u32::from(luminance(c) < 0x8000),
                        Space::Indexed(dt) => u32::from(super::color::nearest(dt, c)),
                        Space::Direct(d) => pixel_of(d, c),
                    }
                }).collect())
            }
        })
    }
    /// Convert a row of source pixel values in place.
    pub fn row(&self, v: &mut [u32]) {
        match self {
            Conv::Same => {}
            Conv::Lut(t) => for p in v.iter_mut() { *p = t.get(*p as usize).copied().unwrap_or(0); },
            Conv::D16To32 => for p in v.iter_mut() {
                let w = |c: u32| (c << 3) | (c >> 2);
                *p = (w((*p >> 10) & 31) << 16) | (w((*p >> 5) & 31) << 8) | w(*p & 31);
            },
            Conv::D32To16 => for p in v.iter_mut() { *p = (((*p >> 19) & 31) << 10) | (((*p >> 11) & 31) << 5) | ((*p >> 3) & 31); },
            Conv::ToIndexed(d, it) => for p in v.iter_mut() { *p = u32::from(it.lookup(rgb_of(*d, *p))); },
            Conv::ToMono(d) => for p in v.iter_mut() { *p = u32::from(luminance(rgb_of(*d, *p)) < 0x8000); },
        }
    }
}

/// Arithmetic transfer modes (IWQ pp. 4-38..4-40), on 16-bit components: `s` source, `d` destination, `op` the port's OpColor.
pub mod mode {
    pub const BLEND: u16 = 32;
    pub const ADD_PIN: u16 = 33;
    pub const ADD_OVER: u16 = 34;
    pub const SUB_PIN: u16 = 35;
    pub const TRANSPARENT: u16 = 36;
    pub const ADD_MAX: u16 = 37;
    pub const SUB_OVER: u16 = 38;
    pub const AD_MIN: u16 = 39;
}

/// One component of an arithmetic mode: blend `(s*w + d*(65535-w)) / 65535` with `w` the OpColor component; addPin
/// `min(s+d, op)`; addOver `(s+d) mod 65536`; subPin `max(s-d, op)` with `s-d` stopping at 0; addMax `max`; subOver
/// `(s-d) mod 65536`; adMin `min`. `None` for a mode that is not one of these (transparent is a pixel rule, [`transparent`]).
pub fn arith_component(m: u16, s: u16, d: u16, op: u16) -> Option<u16> {
    let (s32, d32, o32) = (u32::from(s), u32::from(d), u32::from(op));
    Some(match m {
        mode::BLEND => ((s32 * o32 + d32 * (65535 - o32)) / 65535) as u16,
        mode::ADD_PIN => (s32 + d32).min(o32) as u16,
        mode::ADD_OVER => s.wrapping_add(d),
        mode::SUB_PIN => s.saturating_sub(d).max(op),
        mode::ADD_MAX => s.max(d),
        mode::SUB_OVER => s.wrapping_sub(d),
        mode::AD_MIN => s.min(d),
        _ => return None,
    })
}

/// [`arith_component`] on each component.
pub fn arith(m: u16, s: Rgb, d: Rgb, op: Rgb) -> Option<Rgb> {
    Some(Rgb { r: arith_component(m, s.r, d.r, op.r)?, g: arith_component(m, s.g, d.g, op.g)?, b: arith_component(m, s.b, d.b, op.b)? })
}

/// `transparent` (36): the source pixel, except where it is the background pixel (then the destination stays).
pub fn transparent(s: u32, d: u32, bg: u32) -> u32 { if s == bg { d } else { s } }

/// On a 1-bit destination the arithmetic modes are the Boolean ones IWQ names (blend -> srcCopy, addPin -> srcBic, addOver ->
/// srcXor, subPin -> srcOr, transparent -> srcOr, addMax -> srcBic, subOver -> srcXor, adMin -> srcOr; Executor
/// `qStdRgn.cpp` `funny_mode_lookup`).
pub fn mono_equivalent(m: u16) -> Option<u16> {
    Some(match m { 32 => 0, 33 => 3, 34 => 2, 35 => 1, 36 => 1, 37 => 3, 38 => 2, 39 => 1, _ => return None })
}

/// `CopyDeepMask`'s weighted average for one component: `(s*w + d*(65535-w)) / 65535`, `w` the mask pixel's component (white =
/// 65535 = all source, black = 0... the mask is read as WEIGHTS of the source).
pub fn weigh(s: u16, d: u16, w: u16) -> u16 { ((u32::from(s) * u32::from(w) + u32::from(d) * (65535 - u32::from(w))) / 65535) as u16 }

/// Big-endian guest bytes of a direct row -> pixel values (`depth` 16 or 32; `out` has `width` entries).
pub fn unpack_row(bytes: &[u8], depth: u8, out: &mut [u32]) {
    if depth == 16 { for (o, c) in out.iter_mut().zip(bytes.chunks_exact(2)) { *o = u32::from(u16::from_be_bytes([c[0], c[1]])); } }
    else { for (o, c) in out.iter_mut().zip(bytes.chunks_exact(4)) { *o = u32::from_be_bytes([c[0], c[1], c[2], c[3]]); } }
}

/// Pixel values -> big-endian guest bytes of a direct row.
pub fn pack_row(vals: &[u32], depth: u8, out: &mut [u8]) {
    if depth == 16 { for (c, &v) in out.chunks_exact_mut(2).zip(vals) { c.copy_from_slice(&(v as u16).to_be_bytes()); } }
    else { for (c, &v) in out.chunks_exact_mut(4).zip(vals) { c.copy_from_slice(&v.to_be_bytes()); } }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cqd::color::clut8;
    const fn c(r: u16, g: u16, b: u16) -> Rgb { Rgb { r, g, b } }

    #[test]
    fn colour_to_pixel_takes_the_top_bits_and_back_replicates() {
        // 32 bits: top byte of each component
        assert_eq!(pixel_of(32, c(0x1234, 0xABCD, 0xFF00)), 0x0012_ABFF);
        // 16 bits: top 5 bits, x1R5G5B5
        assert_eq!(pixel_of(16, c(0xFFFF, 0x0000, 0x8000)), (31 << 10) | 16);
        // replicate: 8-bit 0x12 -> 0x1212; 5-bit 0x1F -> 0xFFFF; 5-bit 0x10 (10000) -> 1000010000100001 = 0x8421
        assert_eq!(rgb_of(32, 0x0012_ABFF), c(0x1212, 0xABAB, 0xFFFF));
        assert_eq!(rgb_of(16, (31 << 10) | 16), c(0xFFFF, 0, 0x8421));
        assert_eq!(widen(1, 5), 0x0842, "00001 -> 0000100001000010");
        assert_eq!(rgb_of(32, 0xAB00_0000), Rgb::BLACK, "the unused byte is not colour");
        for p in [0u32, 0x7FFF, 0x1234, 0x4210] { assert_eq!(pixel_of(16, rgb_of(16, p)), p, "16-bit round trip {p:#x}"); }
        for p in [0u32, 0x00FF_FFFF, 0x0012_3456] { assert_eq!(pixel_of(32, rgb_of(32, p)), p, "32-bit round trip {p:#x}"); }
    }

    #[test]
    fn inverse_table_cells_hold_the_nearest_entry() {
        let t = clut8();
        let it = ITab::build(&t, 4).unwrap();
        assert_eq!(it.cells.len(), 4096);
        assert_eq!(it.lookup(Rgb::WHITE), 0, "white is entry 0 of the standard table");
        assert_eq!(it.lookup(Rgb::BLACK), 255, "black is entry 255");
        // pure red FFFF,0,0 is the cube entry r=FF g=00 b=00: index 0*36 + 5*6 + 5 = 35
        assert_eq!(it.lookup(c(0xFFFF, 0, 0)), 35);
        assert_eq!(t[35], c(0xFFFF, 0, 0));
        // a cell's colour is its 4 bits replicated: 0x7xxx red -> cell r=7 -> 0x7777; nearest of the 0x66/0x77 red ramp etc. is the
        // table's own nearest to (0x7777,0,0): computed by the same rule, not by Apple's flood fill
        let cell = c(0x7777, 0, 0);
        assert_eq!(it.lookup(c(0x7000, 0x0FFF, 0x0123)), crate::cqd::color::nearest(&t, cell));
        assert!(ITab::build(&t, 2).is_none() && ITab::build(&t, 6).is_none() && ITab::build(&[], 4).is_none(), "negative: res 3..5, a table");
    }

    #[test]
    fn the_converter_covers_every_depth_pair() {
        let t = clut8();
        let it = ITab::build(&t, 4).unwrap();
        let mut v = alloc::vec![0u32, 35, 255];
        Conv::new(Space::Indexed(&t), 8, Space::Direct(32), 0, 0, None).unwrap().row(&mut v);
        assert_eq!(v, [0x00FF_FFFF, 0x00FF_0000, 0]);
        let mut w = alloc::vec![(31 << 10) | (1 << 5) | 16];
        Conv::new(Space::Direct(16), 16, Space::Direct(32), 0, 0, None).unwrap().row(&mut w);
        assert_eq!(w, [(0xFF << 16) | (0x08 << 8) | 0x84], "5->8 replicate: 1 -> 00001000, 16 -> 10000100");
        Conv::new(Space::Direct(32), 32, Space::Direct(16), 0, 0, None).unwrap().row(&mut w);
        assert_eq!(w, [(31 << 10) | (1 << 5) | 16], "8->5 truncates back");
        let mut x = alloc::vec![0x00FF_0000, 0x00FF_FFFF, 0x0000_0001];
        Conv::new(Space::Direct(32), 32, Space::Indexed(&t), 0, 0, Some(&it)).unwrap().row(&mut x);
        assert_eq!(x, [35, 0, 255]);
        let mut y = alloc::vec![0, 1];
        Conv::new(Space::Mono, 1, Space::Direct(32), 0x0011_2233, 0x00AA_BBCC, None).unwrap().row(&mut y);
        assert_eq!(y, [0x00AA_BBCC, 0x0011_2233], "1-bit is colourised: white -> bg, black -> fg");
        let mut z = alloc::vec![0x0080_8080, 0x007F_7F7F];
        Conv::new(Space::Direct(32), 32, Space::Mono, 0, 0, None).unwrap().row(&mut z);
        assert_eq!(z, [0, 1], "luminance 0x8080 is white, 0x7F7F black");
        assert!(Conv::new(Space::Direct(32), 32, Space::Indexed(&t), 0, 0, None).is_none(), "negative: an indexed destination needs its inverse table");
    }

    #[test]
    fn arithmetic_modes_follow_their_definitions() {
        let (s, d) = (0xFFFFu16, 0x0101u16);
        assert_eq!(arith_component(mode::ADD_OVER, s, d, 0), Some(0x0100), "wraps at 16 bits");
        assert_eq!(arith_component(mode::ADD_PIN, 0x8000, 0x8000, 0xC000), Some(0xC000), "pinned at OpColor");
        assert_eq!(arith_component(mode::ADD_PIN, 0x1000, 0x2000, 0xC000), Some(0x3000));
        assert_eq!(arith_component(mode::SUB_PIN, 0x1000, 0x3000, 0x0800), Some(0x0800), "s-d stops at 0, then max with OpColor");
        assert_eq!(arith_component(mode::SUB_PIN, 0x5000, 0x1000, 0x0800), Some(0x4000));
        assert_eq!(arith_component(mode::SUB_OVER, 0x1000, 0x3000, 0), Some(0xE000), "wraps");
        assert_eq!(arith_component(mode::ADD_MAX, 0x1000, 0x3000, 0), Some(0x3000));
        assert_eq!(arith_component(mode::AD_MIN, 0x1000, 0x3000, 0), Some(0x1000));
        // blend with weight 0x8000: (0xFFFF*0x8000 + 0*0x7FFF)/0xFFFF = 0x8000
        assert_eq!(arith_component(mode::BLEND, 0xFFFF, 0, 0x8000), Some(0x8000));
        assert_eq!(arith_component(mode::BLEND, 0x1234, 0x5678, 0xFFFF), Some(0x1234), "full weight is the source");
        assert_eq!(arith_component(mode::BLEND, 0x1234, 0x5678, 0), Some(0x5678), "zero weight is the destination");
        assert_eq!(transparent(5, 9, 5), 9);
        assert_eq!(transparent(6, 9, 5), 6);
        assert_eq!(arith_component(mode::TRANSPARENT, 1, 2, 3), None, "negative: transparent is a pixel rule");
        assert_eq!(arith_component(50, 1, 2, 3), None, "negative: hilite is not arithmetic");
        assert_eq!(mono_equivalent(34), Some(2));
        assert_eq!(weigh(0xFFFF, 0, 0x8000), 0x8000);
    }

    #[test]
    fn direct_rows_are_big_endian_words_and_longs() {
        let mut v = [0u32; 2];
        unpack_row(&[0x12, 0x34, 0xAB, 0xCD], 16, &mut v);
        assert_eq!(v, [0x1234, 0xABCD]);
        let mut o = [0u8; 8];
        pack_row(&[0x0011_2233, 0xFF00_00FF], 32, &mut o);
        assert_eq!(o, [0, 0x11, 0x22, 0x33, 0xFF, 0, 0, 0xFF]);
        let mut b = [0u32; 2];
        unpack_row(&o, 32, &mut b);
        assert_eq!(b, [0x0011_2233, 0xFF00_00FF]);
    }
}
