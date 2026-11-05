//! Direct-colour Color QuickDraw by the PowerPC C names (the shared core in heirloom-mac68k-engine `cqd/direct.rs`, `qd/copybits.rs`):
//! 16 and 32-bit offscreen worlds, drawing with RGB pens, CopyBits between every pair of depths, the Boolean modes on direct pixels,
//! the arithmetic modes with OpColor, CopyMask / CopyDeepMask. Every expected pixel is computed from the documented rule named
//! beside it (Inside Macintosh: Imaging With QuickDraw (IWQ), "Color QuickDraw" / "Arithmetic transfer modes"), never read
//! back from the engine.
use heirloom_mac68k_engine::cqd::color::{clut8, Rgb};
use heirloom_macppc_engine::symbols::Binding;
use heirloom_macppc_engine::{GlueOutcome, MacPpc, PpcHost};
use heirloom_ui_abi::RecordingGuestUi;

type M = MacPpc<RecordingGuestUi>;
const OUT: u32 = 0x4000;
const R1: u32 = 0x4100;
const R2: u32 = 0x4110;
const R3: u32 = 0x4118;
const RGB: u32 = 0x4120;

fn call(m: &mut M, name: &str, args: &[u32]) -> u32 {
    assert!(!matches!(m.bind(name), Binding::Unbound(_)), "{name} must be bound: {:?}", m.bind(name));
    let mut regs = [0u32; 32];
    regs[3..3 + args.len()].copy_from_slice(args);
    assert_eq!(m.glue_call(name, &mut regs), GlueOutcome::Return, "{name}: {:?}", m.mac.refused);
    regs[3]
}
fn refused(m: &mut M, name: &str, args: &[u32]) -> String {
    m.bind(name);
    let mut regs = [0u32; 32];
    regs[3..3 + args.len()].copy_from_slice(args);
    let before = m.mac.refused.len();
    // a Toolbox trap's refusal is recorded by name (the program goes on); a native's stops the call
    match m.glue_call(name, &mut regs) { GlueOutcome::Refused(w) => w, GlueOutcome::Return if m.mac.refused.len() > before => m.mac.refused[before..].join(" | "), o => panic!("{name}: expected a refusal, got {o:?}") }
}
fn rect(m: &mut M, at: u32, t: i16, l: i16, b: i16, r: i16) { for (i, v) in [t, l, b, r].into_iter().enumerate() { m.mac.mem.write_u16(at + 2 * i as u32, v as u16).unwrap(); } }
fn rgb(m: &mut M, r: u16, g: u16, b: u16) -> u32 { for (i, v) in [r, g, b].into_iter().enumerate() { m.mac.mem.write_u16(RGB + 2 * i as u32, v).unwrap(); } RGB }

/// A world `w` x 1 pixels deep `depth`; (port, PixMapHandle, base address, rowBytes).
struct W { port: u32, pm: u32, base: u32, row: u32, depth: u32 }
fn world(m: &mut M, depth: u32, w: i16, h: i16) -> W {
    rect(m, R1, 0, 0, h, w);
    assert_eq!(call(m, "NewGWorld", &[OUT, depth, R1, 0, 0, 0]), 0);
    let port = m.mac.mem.read_u32(OUT).unwrap();
    let pm = call(m, "GetGWorldPixMap", &[port]);
    W { port, pm, base: call(m, "GetPixBaseAddr", &[pm]), row: call(m, "GetPixRowBytes", &[pm]), depth }
}
impl W {
    fn put(&self, m: &mut M, x: u32, y: u32, v: u32) {
        let at = self.base + y * self.row;
        match self.depth { 32 => m.mac.mem.write_u32(at + 4 * x, v).unwrap(), 16 => m.mac.mem.write_u16(at + 2 * x, v as u16).unwrap(), 8 => m.mac.mem.write_u8(at + x, v as u8).unwrap(),
            1 => { let b = m.mac.mem.read_u8(at + x / 8).unwrap(); let bit = 0x80 >> (x % 8); m.mac.mem.write_u8(at + x / 8, if v != 0 { b | bit } else { b & !bit }).unwrap(); }
            d => panic!("depth {d}") }
    }
    fn get(&self, m: &M, x: u32, y: u32) -> u32 {
        let at = self.base + y * self.row;
        match self.depth { 32 => m.mac.mem.read_u32(at + 4 * x).unwrap(), 16 => u32::from(m.mac.mem.read_u16(at + 2 * x).unwrap()), 8 => u32::from(m.mac.mem.read_u8(at + x).unwrap()),
            1 => u32::from(m.mac.mem.read_u8(at + x / 8).unwrap() & (0x80 >> (x % 8)) != 0), d => panic!("depth {d}") }
    }
    fn row(&self, m: &M, n: u32) -> Vec<u32> { (0..n).map(|x| self.get(m, x, 0)).collect() }
    fn bits(&self, m: &mut M) -> u32 { call(m, "GetPortBitMapForCopyBits", &[self.port]) }
}
fn copy(m: &mut M, s: &W, d: &W, n: i16, mode: u32) {
    rect(m, R1, 0, 0, 1, n);
    let (sb, db) = (s.bits(m), d.bits(m));
    call(m, "CopyBits", &[sb, db, R1, R1, mode, 0]);
}
fn machine() -> M {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    call(&mut m, "InitGraf", &[0x1000]);
    m
}
/// The standard 8-bit table's index of a colour that is IN it.
fn idx(c: Rgb) -> u32 { clut8().iter().position(|&e| e == c).unwrap() as u32 }
const fn c(r: u16, g: u16, b: u16) -> Rgb { Rgb { r, g, b } }

#[test]
fn direct_worlds_hold_rgb_pixels_drawn_with_rgb_pens() {
    let mut m = machine();
    let w = world(&mut m, 32, 4, 2);
    assert_eq!(w.row, 16, "4 longs a row");
    call(&mut m, "SetGWorld", &[w.port, 0]);
    let fg = rgb(&mut m, 0x1234, 0xABCD, 0xFF00);
    call(&mut m, "RGBForeColor", &[fg]);
    rect(&mut m, R2, 0, 1, 2, 3);
    call(&mut m, "PaintRect", &[R2]);
    // 32 bits: the top byte of each component (IWQ "Direct devices")
    assert_eq!(w.row(&m, 4), [0, 0x0012_ABFF, 0x0012_ABFF, 0], "only columns 1..3 painted");
    let s = world(&mut m, 16, 2, 1);
    assert_eq!(s.row, 4, "2 words padded to a long");
    call(&mut m, "SetGWorld", &[s.port, 0]);
    let fg = rgb(&mut m, 0xFFFF, 0x0000, 0x8000);
    call(&mut m, "RGBForeColor", &[fg]);
    rect(&mut m, R2, 0, 0, 1, 2);
    call(&mut m, "PaintRect", &[R2]);
    // 16 bits x1R5G5B5: r 31, g 0, b 0x8000 >> 11 = 16
    assert_eq!(s.row(&m, 2), [(31 << 10) | 16, (31 << 10) | 16]);
    // EraseRect paints the background (white): every colour bit set, the unused bit/byte 0
    call(&mut m, "EraseRect", &[R2]);
    assert_eq!(s.row(&m, 2), [0x7FFF, 0x7FFF]);
    // InvertRect inverts colour bits only
    m.mac.mem.write_u16(s.base, 0x8000 | 0x1234).unwrap();
    rect(&mut m, R2, 0, 0, 1, 1);
    call(&mut m, "InvertRect", &[R2]);
    assert_eq!(s.get(&m, 0, 0), 0x8000 | (0x1234 ^ 0x7FFF), "bit 15 kept");
    assert!(m.mac.refused.is_empty(), "{:?}", m.mac.refused);
}

#[test]
fn copybits_converts_between_every_pair_of_depths() {
    let mut m = machine();
    let src = world(&mut m, 32, 4, 1);
    for (x, v) in [0x00FF_0000u32, 0x00FF_FFFF, 0, 0x0012_3456].into_iter().enumerate() { src.put(&mut m, x as u32, 0, v); }
    // 32 -> 8 through the inverse table (4 bits a component; a cell holds the entry nearest its replicated colour)
    let d8 = world(&mut m, 8, 4, 1);
    copy(&mut m, &src, &d8, 4, 0);
    // 0x123456: cell (1,3,5) = (0x1111,0x3333,0x5555); nearest standard entry: cube (00,33,66) at 0x1111 per red and blue
    let cube = c(0, 0x3333, 0x6666);
    assert_eq!(d8.row(&m, 4), [idx(c(0xFFFF, 0, 0)), 0, 255, idx(cube)]);
    assert_eq!(idx(cube), 5 * 36 + 4 * 6 + 3, "levels FF CC 99 66 33 00: red 00 = 5, green 33 = 4, blue 66 = 3");
    // 8 -> 32: the table's colours as direct pixels
    let back = world(&mut m, 32, 4, 1);
    copy(&mut m, &d8, &back, 4, 0);
    assert_eq!(back.row(&m, 4), [0x00FF_0000, 0x00FF_FFFF, 0, 0x0000_3366]);
    // 32 -> 16: the top 5 bits; 0x123456 -> r 2, g 6, b 10
    let d16 = world(&mut m, 16, 4, 1);
    copy(&mut m, &src, &d16, 4, 0);
    assert_eq!(d16.row(&m, 4), [31 << 10, 0x7FFF, 0, (2 << 10) | (6 << 5) | 10]);
    // 16 -> 32: replicate 5 -> 8 bits (c << 3 | c >> 2): 2 -> 0x10, 6 -> 0x31, 10 -> 0x52
    let up = world(&mut m, 32, 4, 1);
    copy(&mut m, &d16, &up, 4, 0);
    assert_eq!(up.row(&m, 4), [0x00FF_0000, 0x00FF_FFFF, 0, 0x0010_3152]);
    // 32 -> 1: black below half luminance (Rec. 601): red 0.299 < 0.5 is black, white white, black black, 0x123456 black
    let d1 = world(&mut m, 1, 4, 1);
    copy(&mut m, &src, &d1, 4, 0);
    assert_eq!(d1.row(&m, 4), [1, 0, 1, 1]);
    // 1 -> 32 is colourised with thePort's pens: black -> the foreground, white -> the background
    call(&mut m, "SetGWorld", &[up.port, 0]);
    let fg = rgb(&mut m, 0xFFFF, 0, 0);
    call(&mut m, "RGBForeColor", &[fg]);
    let bg = rgb(&mut m, 0, 0, 0xFFFF);
    call(&mut m, "RGBBackColor", &[bg]);
    copy(&mut m, &d1, &up, 4, 0);
    assert_eq!(up.row(&m, 4), [0x00FF_0000, 0x0000_00FF, 0x00FF_0000, 0x00FF_0000]);
    assert!(m.mac.refused.is_empty(), "{:?}", m.mac.refused);
}

#[test]
fn boolean_modes_act_on_direct_pixel_values() {
    let mut m = machine();
    let s = world(&mut m, 32, 1, 1);
    s.put(&mut m, 0, 0, 0x000F_0FF0);
    // IWQ "Boolean transfer modes with color pixels" on a direct destination, thePort's pens black on white: the source 0x0F0FF0
    // is INK (Rec. 601 luminance 0x28CF < half): srcOr writes the foreground (black), srcXor inverts the destination's colour
    // bits, srcBic writes the background (white), notSrcCopy is the source colour inverted, srcCopy the source
    for (mode, want) in [(1u32, 0u32), (2, 0x000F_0F0F), (3, 0x00FF_FFFF), (4, 0x00F0_F00F), (0, 0x000F_0FF0), (5, 0x00F0_F0F0), (6, 0x00F0_F0F0), (7, 0x00F0_F0F0)] {
        let d = world(&mut m, 32, 1, 1);
        d.put(&mut m, 0, 0, 0x00F0_F0F0);
        copy(&mut m, &s, &d, 1, mode);
        assert_eq!(d.get(&m, 0, 0), want, "mode {mode}");
    }
    // NEGATIVE control: a white (paper) source leaves the destination under srcOr / srcXor / srcBic
    let white = world(&mut m, 32, 1, 1);
    white.put(&mut m, 0, 0, 0x00FF_FFFF);
    for mode in [1u32, 2, 3] {
        let d = world(&mut m, 32, 1, 1);
        d.put(&mut m, 0, 0, 0x0012_3456);
        copy(&mut m, &white, &d, 1, mode);
        assert_eq!(d.get(&m, 0, 0), 0x0012_3456, "mode {mode} over paper");
    }
    // a 1-bit source into a direct destination with srcOr: its black bits take the foreground colour (text and icons)
    let (one, d) = (world(&mut m, 1, 2, 1), world(&mut m, 32, 2, 1));
    one.put(&mut m, 0, 0, 1);
    for x in 0..2 { d.put(&mut m, x, 0, 0x0055_5555); }
    call(&mut m, "SetGWorld", &[d.port, 0]);
    let red = rgb(&mut m, 0xFFFF, 0, 0);
    call(&mut m, "RGBForeColor", &[red]);
    copy(&mut m, &one, &d, 2, 1);
    assert_eq!(d.row(&m, 2), [0x00FF_0000, 0x0055_5555]);
    // ditherCopy (64) is accepted and does no dithering
    let d = world(&mut m, 32, 1, 1);
    copy(&mut m, &s, &d, 1, 64);
    assert_eq!(d.get(&m, 0, 0), 0x000F_0FF0);
}

#[test]
fn arithmetic_modes_combine_colours_with_opcolor() {
    let mut m = machine();
    let pair = |m: &mut M, sv: u32, dv: u32, mode: u32| -> u32 {
        let (s, d) = (world(m, 32, 1, 1), world(m, 32, 1, 1));
        s.put(m, 0, 0, sv);
        d.put(m, 0, 0, dv);
        copy(m, &s, &d, 1, mode);
        d.get(m, 0, 0)
    };
    // OpColor lives in thePort's grafVars (a colour port: a world's)
    let host = world(&mut m, 32, 1, 1);
    call(&mut m, "SetGWorld", &[host.port, 0]);
    let op = rgb(&mut m, 0xC000, 0x8000, 0x4000);
    call(&mut m, "OpColor", &[op]);
    // addPin: min(s+d, op) on 16-bit components: 0x4040 + 0x8080 = 0xC0C0 -> min with (C000, 8000, 4000)
    assert_eq!(pair(&mut m, 0x0040_4040, 0x0080_8080, 33), 0x00C0_8040);
    // addOver: (s+d) mod 65536: 0x0101 + 0xFFFF = 0x0100 -> top byte 0x01
    assert_eq!(pair(&mut m, 0x0001_0101, 0x00FF_FFFF, 34), 0x0001_0101);
    // subPin: max(s-d, op): 0x8080 - 0x4040 = 0x4040; max with (C000, 8000, 4000) -> C0 80 40
    assert_eq!(pair(&mut m, 0x0080_8080, 0x0040_4040, 35), 0x00C0_8040);
    // subOver: (s-d) mod 65536: 0x4040 - 0x8080 = 0xBFC0 -> 0xBF
    assert_eq!(pair(&mut m, 0x0040_4040, 0x0080_8080, 38), 0x00BF_BFBF);
    assert_eq!(pair(&mut m, 0x0010_8030, 0x0080_1040, 37), 0x0080_8040, "addMax: per component max");
    assert_eq!(pair(&mut m, 0x0010_8030, 0x0080_1040, 39), 0x0010_1030, "adMin: per component min");
    // blend: (s*w + d*(65535-w)) / 65535, w = OpColor 0x8000 per component: s 0xFFFF, d 0 -> 0x8000 -> 0x80
    let half = rgb(&mut m, 0x8000, 0x8000, 0x8000);
    call(&mut m, "OpColor", &[half]);
    assert_eq!(pair(&mut m, 0x00FF_FFFF, 0, 32), 0x0080_8080);
    // transparent: the source except where it is the background colour (white)
    let (s, d) = (world(&mut m, 32, 2, 1), world(&mut m, 32, 2, 1));
    for (x, v) in [0x00FF_FFFFu32, 0x00FF_0000].into_iter().enumerate() { s.put(&mut m, x as u32, 0, v); d.put(&mut m, x as u32, 0, 0x0000_FF00); }
    copy(&mut m, &s, &d, 2, 36);
    assert_eq!(d.row(&m, 2), [0x0000_FF00, 0x00FF_0000]);
    // an indexed destination: blend in RGB, then the inverse table. White (0) under black (255) at weight 0x8000:
    // (0*0x8000 + 0xFFFF*0x7FFF)/0xFFFF = 0x7FFF -> cell 7 -> (0x7777 x3), the gray ramp's 0x77 entry
    let (s8, d8) = (world(&mut m, 8, 1, 1), world(&mut m, 8, 1, 1));
    s8.put(&mut m, 0, 0, 255);
    copy(&mut m, &s8, &d8, 1, 32);
    assert_eq!(d8.get(&m, 0, 0), idx(c(0x7777, 0x7777, 0x7777)));
    // a 1-bit destination takes the Boolean equivalent: addOver -> srcXor
    let (s1, d1) = (world(&mut m, 1, 1, 1), world(&mut m, 1, 1, 1));
    s1.put(&mut m, 0, 0, 1);
    d1.put(&mut m, 0, 0, 1);
    copy(&mut m, &s1, &d1, 1, 34);
    assert_eq!(d1.get(&m, 0, 0), 0, "1 xor 1");
    assert!(m.mac.refused.is_empty(), "{:?}", m.mac.refused);
    // NEGATIVE controls: hilite (50) and a pattern mode (8) are refused by name; an arithmetic mode with scaling too
    let (sb, db) = (s.bits(&mut m), d.bits(&mut m));
    rect(&mut m, R1, 0, 0, 1, 2);
    assert!(refused(&mut m, "CopyBits", &[sb, db, R1, R1, 50, 0]).contains("transfer mode 50 is not implemented"));
    assert!(refused(&mut m, "CopyBits", &[sb, db, R1, R1, 8, 0]).contains("transfer mode 8 is not implemented"));
    rect(&mut m, R2, 0, 0, 1, 1);
    assert!(refused(&mut m, "CopyBits", &[sb, db, R1, R2, 33, 0]).contains("with scaling"));
}

#[test]
fn copymask_and_copydeepmask_on_direct_worlds() {
    let mut m = machine();
    let (s, d) = (world(&mut m, 32, 3, 1), world(&mut m, 32, 3, 1));
    for x in 0..3 { s.put(&mut m, x, 0, 0x00FF_FFFF); }
    // a 1-bit mask: only its black pixels copy
    let k = world(&mut m, 1, 3, 1);
    k.put(&mut m, 1, 0, 1);
    let (sb, kb, db) = (s.bits(&mut m), k.bits(&mut m), d.bits(&mut m));
    rect(&mut m, R1, 0, 0, 1, 3);
    call(&mut m, "CopyMask", &[sb, kb, db, R1, R1, R1]);
    assert_eq!(d.row(&m, 3), [0, 0x00FF_FFFF, 0]);
    // a deep (8-bit) mask: per component (s*w + d*(65535-w))/65535 with w the mask colour; white = all source, black = none
    let dd = world(&mut m, 32, 3, 1);
    let km = world(&mut m, 8, 3, 1);
    for (x, v) in [0u32, idx(c(0x7777, 0x7777, 0x7777)), 255].into_iter().enumerate() { km.put(&mut m, x as u32, 0, v); }
    let (kmb, ddb) = (km.bits(&mut m), dd.bits(&mut m));
    call(&mut m, "CopyDeepMask", &[sb, kmb, ddb, R1, R1, R1, 0, 0]);
    // 0xFFFF * 0x7777 / 0xFFFF = 0x7777 -> 0x77
    assert_eq!(dd.row(&m, 3), [0x00FF_FFFF, 0x0077_7777, 0]);
    // the 1-bit mask path of CopyDeepMask is CopyMask's
    let d2 = world(&mut m, 32, 3, 1);
    let d2b = d2.bits(&mut m);
    call(&mut m, "CopyDeepMask", &[sb, kb, d2b, R1, R1, R1, 0, 0]);
    assert_eq!(d2.row(&m, 3), [0, 0x00FF_FFFF, 0]);
    assert!(m.mac.refused.is_empty(), "{:?}", m.mac.refused);
    // NEGATIVE controls: a scaling deep mask, and a mode other than srcCopy with a deep mask, are refused by name
    rect(&mut m, R3, 0, 0, 1, 2);
    assert!(refused(&mut m, "CopyDeepMask", &[sb, kmb, ddb, R1, R1, R3, 0, 0]).contains("a scaling mask is not implemented"));
    assert!(refused(&mut m, "CopyDeepMask", &[sb, kmb, ddb, R1, R1, R1, 1, 0]).contains("srcCopy only"));
}
