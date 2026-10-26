//! DrawPicture of stored pictures: the opcode stream is parsed once (qd/pict.rs) and every bitmap opcode is drawn by the engine's own CopyBits
//! (qd/pictplay.rs), so scaling, clipping, colour mapping and masks are CopyBits'. Pictures here are built by hand to the format in Inside
//! Macintosh V ("QuickDraw: Pictures"); the corpus' own pictures (Glider PRO's 150) are exercised by the census.
mod common;
use common::*;

fn w(v: &mut Vec<u8>, xs: &[u16]) { for x in xs { v.extend(x.to_be_bytes()); } }

/// Version 2, frame (10,20)-(12,28): a PackBitsRect of an 8-bit 8x2 PixMap (table: 0 white, 1 black); row 0 all 1, row 1 = 0,1,0,1,...
/// `extra` is spliced in before the end opcode.
fn v2_picture(extra: &[u8]) -> Vec<u8> {
    let mut b = Vec::new();
    w(&mut b, &[0, 10, 20, 12, 28, 0x0011, 0x02FF, 0x0C00]);
    b.extend([0u8; 24]);
    w(&mut b, &[0x0098, 0x8008, 0, 0, 2, 8]);
    w(&mut b, &[0, 0, 0, 0, 0, 0, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 0, 0]);
    w(&mut b, &[0, 0, 0, 1]);
    w(&mut b, &[0, 0xFFFF, 0xFFFF, 0xFFFF, 1, 0, 0, 0]);
    w(&mut b, &[0, 0, 2, 8, 10, 20, 12, 28, 0]);
    b.extend([2, 0xF9, 1]);
    b.extend([9, 7, 0, 1, 0, 1, 0, 1, 0, 1]);
    b.push(0);
    b.extend(extra);
    w(&mut b, &[0x00FF]);
    b
}

fn handle_of(g: &mut Rig, bytes: &[u8]) -> u32 {
    let h = g.m.mm.new_handle(&mut g.m.mem, &mut g.m.heap, bytes.len() as u32, false).unwrap();
    let p = g.m.mm.block(h).unwrap().ptr;
    g.m.mem.write_bytes(p, bytes).unwrap();
    h
}

fn draw(g: &mut Rig, pic: &[u8], dst: (i16, i16, i16, i16)) {
    let h = handle_of(g, pic);
    set_rect(&mut g.m, 0x3100, dst.0, dst.1, dst.2, dst.3);
    let mut p = Prog::new();
    p.set_port(g.win).l(h).l(0x3100).trap(0xA8F6);
    g.run(p.exit());
}

#[test]
fn a_v2_packbitsrect_is_drawn_at_its_rectangle_row_by_row_through_the_colour_table() {
    let mut g = Rig::new();
    draw(&mut g, &v2_picture(&[]), (10, 20, 12, 28));
    g.no_refusals();
    assert!((20..28).all(|x| g.black(x, 10)), "row 0 is all index 1 = black\n{}", g.pic(18, 8, 30, 14));
    assert_eq!((20..28).map(|x| g.black(x, 11)).collect::<Vec<_>>(), [false, true, false, true, false, true, false, true], "row 1 alternates white / black");
    assert_eq!(g.count_black(), 8 + 4, "nothing else is drawn");
    // negative control: the same picture drawn into a rectangle elsewhere lands there, not at the frame
    let mut g2 = Rig::new();
    draw(&mut g2, &v2_picture(&[]), (40, 60, 42, 68));
    assert!((60..68).all(|x| g2.black(x, 40)) && !g2.black(20, 10), "the frame maps to dstRect");
}

#[test]
fn a_picture_is_scaled_from_its_frame_to_the_destination_rectangle() {
    let mut g = Rig::new();
    draw(&mut g, &v2_picture(&[]), (10, 20, 14, 36)); // 2x each way
    g.no_refusals();
    assert!((20..36).all(|x| g.black(x, 10) && g.black(x, 11)), "row 0 doubled");
    assert_eq!((20..36).map(|x| g.black(x, 12)).collect::<Vec<_>>(), (0..16).map(|i| (i / 2) % 2 == 1).collect::<Vec<_>>(), "row 1 stretched to double width");
    assert_eq!(g.count_black(), 16 * 2 + 8 * 2, "and double height");
}

#[test]
fn an_opcode_that_draws_is_refused_by_name_and_what_came_before_it_stays_drawn() {
    let mut g = Rig::new();
    let mut paint = Vec::new();
    w(&mut paint, &[0x0031, 0, 0, 0, 0, 0, 5, 0, 5]); // PaintRect
    let pic = v2_picture(&paint);
    let h = handle_of(&mut g, &pic);
    set_rect(&mut g.m, 0x3100, 10, 20, 12, 28);
    let mut p = Prog::new();
    p.set_port(g.win).l(h).l(0x3100).trap(0xA8F6).exit();
    g.m.start(&p.0, 0).unwrap();
    let _ = g.m.run_loop(1_000_000);
    assert!(g.m.refused.iter().any(|r| r.contains("DrawPicture") && r.contains("PaintRect") && r.contains("1 drawn before it")), "{:?}", g.m.refused);
    assert!(g.black(21, 10), "the bitmap before it was drawn");
}

#[test]
fn a_mask_region_limits_an_unscaled_bitmap_and_is_refused_scaled() {
    // PackBitsRgn ($0099) = the same bitmap with a rectangular mask region (rgnSize 10 + bbox) after the mode: only columns 20..24 draw
    let mut pic = v2_picture(&[]);
    // turn the opcode into $0099 and insert the region after the mode word (offset 14 + 2 + 24 + ... find it: the opcode word is at 14 + 2 + 24)
    let at_op = 14 + 2 + 24;
    pic[at_op + 1] = 0x99;
    let at_mode = at_op + 2 + 2 + 8 + 36 + 8 + 16 + 8 + 8; // opcode, rowBytes, bounds, pixmap rest, ctab head, 2 entries, src, dst
    let mut rgn = Vec::new();
    w(&mut rgn, &[10, 10, 20, 12, 24]); // rgnSize 10, bbox top 10 left 20 bottom 12 right 24
    for (i, b) in rgn.iter().enumerate() { pic.insert(at_mode + 2 + i, *b); }
    let mut g = Rig::new();
    draw(&mut g, &pic, (10, 20, 12, 28));
    g.no_refusals();
    assert!((20..24).all(|x| g.black(x, 10)) && !(24..28).any(|x| g.black(x, 10)), "row 0 draws only inside the mask\n{}", g.pic(18, 8, 30, 14));
    // scaled: refused by name
    let mut g = Rig::new();
    let h = handle_of(&mut g, &pic);
    set_rect(&mut g.m, 0x3100, 10, 20, 14, 36);
    let mut p = Prog::new();
    p.set_port(g.win).l(h).l(0x3100).trap(0xA8F6).exit();
    g.m.start(&p.0, 0).unwrap();
    let _ = g.m.run_loop(1_000_000);
    assert!(g.m.refused.iter().any(|r| r.contains("mask region is drawn only unscaled")), "{:?}", g.m.refused);
}

#[test]
fn a_version_1_bitsrect_of_a_one_bit_bitmap_is_drawn_and_a_pattern_mode_is_refused() {
    let mk = |mode: u16| {
        let mut b = vec![0, 0, 0, 5, 0, 5, 0, 7, 0, 13, 0x11, 0x01, 0x90]; // frame (5,5)-(7,13), version 1, BitsRect
        w(&mut b, &[2, 0, 0, 1, 8, 0, 0, 1, 8, 5, 5, 6, 13, mode]); // rowBytes 2 (unpacked), bounds 0,0,1,8, src, dst 5,5,6,13, mode
        b.extend([0xA5, 0x00, 0xFF]);
        b
    };
    let mut g = Rig::new();
    draw(&mut g, &mk(0), (5, 5, 7, 13));
    g.no_refusals();
    assert_eq!((5..13).map(|x| g.black(x, 5)).collect::<Vec<_>>(), [true, false, true, false, false, true, false, true], "0xA5 = 10100101, black bits set");
    let mut g = Rig::new();
    let h = handle_of(&mut g, &mk(8));
    set_rect(&mut g.m, 0x3100, 5, 5, 7, 13);
    let mut p = Prog::new();
    p.set_port(g.win).l(h).l(0x3100).trap(0xA8F6).exit();
    g.m.start(&p.0, 0).unwrap();
    let _ = g.m.run_loop(1_000_000);
    assert!(g.m.refused.iter().any(|r| r.contains("transfer mode 8")), "{:?}", g.m.refused);
}
