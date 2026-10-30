//! Regions (as guest structs and as clip), the palette and pens, DrawInfo, and intuition's Image/Border/IntuiText drawing.
mod gfxcommon;
use heirloom_ami68k_engine::machine::EXEC_BASE;
use gfxcommon::*;

const NEW_REGION: i32 = -516;
const DISPOSE_REGION: i32 = -534;
const OR_RECT_REGION: i32 = -510;
const XOR_RECT_REGION: i32 = -558;
const CLEAR_RECT_REGION: i32 = -522;
const OR_REGION_REGION: i32 = -612;
const IS_POINT_IN_REGION: i32 = -1104;
const GET_RGB32: i32 = -900;
const OBTAIN_PEN: i32 = -954;
const RELEASE_PEN: i32 = -948;
const OBTAIN_BEST_PEN: i32 = -840;
const FIND_COLOR: i32 = -1008;
const GET_SCREEN_DRAW_INFO: i32 = -690;
const DRAW_IMAGE: i32 = -114;
const DRAW_BORDER: i32 = -108;
const PRINT_ITEXT: i32 = -216;
const INTUI_TEXT_LENGTH: i32 = -330;
const OPEN_LIBRARY: i32 = -552;
const INSTALL_CLIP_REGION: i32 = -174;

fn rect(g: &mut G, x0: i16, y0: i16, x1: i16, y1: i16) -> u32 { g.words(&[x0 as u16, y0 as u16, x1 as u16, y1 as u16]) }

/// The pixels a guest Region's RegionRectangle chain covers (coordinates relative to the bounds' top-left).
fn chain_area(g: &G, rg: u32) -> (i32, [i16; 4], usize) {
    let b = [0, 2, 4, 6].map(|o| g.r16(rg + o) as i16);
    let (mut p, mut area, mut n) = (g.r32(rg + 8), 0, 0);
    while p != 0 {
        let r = [8, 10, 12, 14].map(|o| g.r16(p + o) as i16 as i32);
        area += (r[2] - r[0] + 1) * (r[3] - r[1] + 1);
        p = g.r32(p); n += 1;
    }
    (area, b, n)
}

#[test]
fn regions_are_real_guest_structs_that_follow_the_set_algebra() {
    let mut g = G::new();
    let (r1, r2, r3) = (rect(&mut g, 0, 0, 9, 9), rect(&mut g, 5, 5, 14, 14), rect(&mut g, 2, 2, 3, 3));
    g.run(&[gfx(NEW_REGION, &[]), gfx(OR_RECT_REGION, &[a_res(0, 0), a(1, r1)]), gfx(OR_RECT_REGION, &[a_res(0, 0), a(1, r2)]),
        gfx(IS_POINT_IN_REGION, &[a_res(0, 0), d(0, 12), d(1, 12)]), gfx(IS_POINT_IN_REGION, &[a_res(0, 0), d(0, 12), d(1, 2)])]);
    let rg = g.result(0);
    assert_eq!((g.result(3), g.result(4)), (1, 0));
    let (area, bounds, n) = chain_area(&g, rg);
    assert_eq!((area, bounds), (100 + 100 - 25, [0, 0, 14, 14]), "union of two overlapping 10x10 squares; bounds MinX MinY MaxX MaxY");
    assert!(n >= 3, "a band chain, not one rectangle: {n}");
    g.run(&[gfx(CLEAR_RECT_REGION, &[a(0, rg), a(1, r3)])]);
    assert_eq!(chain_area(&g, rg).0, 175 - 4, "ClearRectRegion removed the 2x2 hole");
    g.run(&[gfx(XOR_RECT_REGION, &[a(0, rg), a(1, r3)])]);
    assert_eq!(chain_area(&g, rg).0, 175, "XorRectRegion over the hole restores it");
    // R2 = R2 OR R1 (the SECOND region changes)
    g.run(&[gfx(NEW_REGION, &[]), gfx(OR_REGION_REGION, &[a(0, rg), a_res(1, 0)])]);
    let rg2 = g.result(0);
    assert_eq!(chain_area(&g, rg2).0, 175, "the second region received the first's area");
    g.run(&[gfx(DISPOSE_REGION, &[a(0, rg)]), gfx(DISPOSE_REGION, &[a(0, rg)])]);
    assert!(g.a.refused[0].contains("not a Region from NewRegion"), "double dispose is refused by name: {:?}", g.a.refused);
}

#[test]
fn an_installed_clip_region_limits_window_drawing_and_removing_it_restores_full_drawing() {
    let mut g = G::new();
    let nw = g.zeroed(48);
    for (o, v) in [(0u32, 0u16), (2, 0), (4, 40), (6, 20)] { g.a.mem.write_u16(nw + o, v).unwrap(); }
    g.a.mem.write_u32(nw + 14, 0x800).unwrap();
    let hole = rect(&mut g, 10, 0, 14, 19);
    let all = rect(&mut g, 0, 0, 39, 19);
    g.run(&[intu(OPEN_WINDOW, &[a(0, nw)]), gfx(NEW_REGION, &[]), gfx(OR_RECT_REGION, &[a_res(0, 1), a(1, all)]), gfx(CLEAR_RECT_REGION, &[a_res(0, 1), a(1, hole)])]);
    let (win, rg) = (g.result(0), g.result(1));
    let (rp, layer) = (g.r32(win + 50), g.r32(win + 124));
    let lname = g.s("layers.library");
    g.run(&[(EXEC_BASE, OPEN_LIBRARY, vec![a(1, lname), d(0, 0)])]);
    let layers = g.result(0);
    assert_ne!(layers, 0);
    g.run(&[(layers, INSTALL_CLIP_REGION, vec![a(0, layer), a(1, rg)]), gfx(SET_APEN, &[a(1, rp), d(0, 2)]), gfx(RECT_FILL, &[a(1, rp), d(0, 0), d(1, 0), d(2, 39), d(3, 0)])]);
    assert_eq!(g.result(0), 0, "no previous clip region");
    let id = *g.a.ui.canvases.keys().next().unwrap();
    const P: [u32; 8] = heirloom_theme::amiga::system_palette();
    let row: Vec<u32> = g.a.ui.canvases[&id].2[..40].to_vec();
    assert_eq!((row[9], row[10], row[14], row[15]), (P[2], P[0], P[0], P[2]), "the hole x=10..14 stays unpainted");
    g.run(&[(layers, INSTALL_CLIP_REGION, vec![a(0, layer), a(1, 0)]), gfx(RECT_FILL, &[a(1, rp), d(0, 0), d(1, 1), d(2, 39), d(3, 1)])]);
    assert_eq!(g.result(0), rg, "InstallClipRegion returns the previously installed region");
    assert_eq!(g.a.ui.canvases[&id].2[40 + 12], 0xFFFFFF, "control: with the region removed the same x is painted");
}

#[test]
fn the_palette_obtain_release_best_pen_and_find_color() {
    let mut g = G::new();
    let tab = g.zeroed(12);
    let none = g.tags(&[]);
    g.run(&[gfx(GET_RGB32, &[a(0, 0), d(0, 2), d(1, 1), a(1, tab)]),
        gfx(OBTAIN_PEN, &[a(0, 0), di(0, -1), d(1, 0xFF00_0000), d(2, 0), d(3, 0), d(4, 0)]),
        gfx(OBTAIN_BEST_PEN, &[a(0, 0), d(1, 0xFF00_0000), d(2, 0), d(3, 0), a(1, none)]),
        gfx(OBTAIN_BEST_PEN, &[a(0, 0), d(1, 0), d(2, 0xFF00_0000), d(3, 0), a(1, none)]),
        gfx(FIND_COLOR, &[a(3, 0), d(1, 0xFE00_0000), d(2, 0x01000000), d(3, 0), di(4, 255)]),
        gfx(RELEASE_PEN, &[a(0, 0), d_res(0, 1)]), gfx(OBTAIN_PEN, &[a(0, 0), di(0, -1), d(1, 0), d(2, 0), d(3, 0xFF00_0000), d(4, 0)])]);
    assert_eq!([g.r32(tab), g.r32(tab + 4), g.r32(tab + 8)], [0xFFFF_FFFF; 3], "GetRGB32 pen 2 (white), 8-bit replicated to 32");
    let (red, red2, green) = (g.result(1), g.result(2), g.result(3));
    assert!(red >= 8, "pens 0..8 belong to the system: {red}");
    assert_eq!(red2, red, "ObtainBestPen shares the exact colour");
    assert!(green >= 8 && green != red);
    assert_eq!(g.result(4), red, "FindColor finds the nearest pen to a (254,1,0) colour: the red one");
    assert_eq!(g.result(6), green + 1, "red was released once but shared twice: still held, so the next free pen is after green");
    assert_eq!(g.px_pal(red), 0xFF0000);
}
impl G { fn px_pal(&mut self, pen: u32) -> u32 { self.a.gfx.palette().rgb[pen as usize] } }

#[test]
fn the_screen_draw_info_is_a_real_struct_with_pens_and_the_default_font() {
    let mut g = G::new();
    g.run(&[intu(GET_SCREEN_DRAW_INFO, &[a(0, heirloom_ami68k_engine::screen::SCREEN)])]);
    let dri = g.result(0);
    assert_ne!(dri, 0);
    assert_eq!((g.r16(dri), g.r16(dri + 2), g.r16(dri + 12)), (3, 12, 8), "DRI_VERSION 3, NUMDRIPENS 12, depth 8");
    let pens = g.r32(dri + 4);
    let p: Vec<u16> = (0..12).map(|i| g.r16(pens + 2 * i)).collect();
    use heirloom_theme::{amiga as am, GUTE_FORM as T};
    assert_eq!(p, am::DRI_PENS, "dri_Pens is the Gute Form map");
    assert_ne!(p, am::AROS_DRI_PENS, "control: not the AROS default pens");
    // each pen resolves, through the screen palette the guest reads (ViewPort ColorMap -> RGB4 table), to its Gute Form token (RGB4 = the top nibble)
    let rgb4 = |c: heirloom_theme::Rgb| ((c.0 as u32 >> 4) << 8 | (c.1 as u32 >> 4) << 4 | c.2 as u32 >> 4) as u16;
    let table = 0x8C00; // SCREEN_COLOR_TABLE (graphics/layout.rs)
    for (pen, want) in [(am::TEXTPEN, T.text), (am::BACKGROUNDPEN, T.surface), (am::FILLPEN, T.accent_press), (am::FILLTEXTPEN, T.text_on_accent), (am::SHINEPEN, T.border), (am::SHADOWPEN, T.border), (am::BARBLOCKPEN, T.surface)] {
        assert_eq!(g.r16(table + 2 * p[pen] as u32), rgb4(want), "{} -> {:?}", am::DRI_PEN_NAMES[pen], want);
    }
    assert_ne!(g.r16(table + 2 * p[am::FILLPEN] as u32), 0x68B, "control: the AROS FILLPEN blue 6688BB is not what the guest reads");
    assert_eq!(g.r32(dri + 8), 0x9000, "dri_Font is the Gute Form UI face (topaz stays at 0x8000 for explicit requests)");
    assert_ne!(g.r32(dri + 22), 0, "dri_CheckMark is an Image");
    g.run(&[intu(GET_SCREEN_DRAW_INFO, &[a(0, 0x1234)])]);
    assert!(g.a.refused[0].contains("not the public screen"), "{:?}", g.a.refused);
}

#[test]
fn images_borders_and_intuitext_draw_with_their_own_pens() {
    let mut g = G::new();
    let (bm, rp) = g.bitmap_rp(40, 24, 3);
    // a 1-plane 8x2 image: PlanePick 1 = the plane feeds destination plane 0; PlaneOnOff 2 = plane 1 constant on
    let data = g.words(&[0b1010_1010_0000_0000, 0b1111_0000_0000_0000]);
    let im = g.zeroed(20);
    for (o, v) in [(0u32, 3u16), (2, 1), (4, 8), (6, 2), (8, 1)] { g.a.mem.write_u16(im + o, v).unwrap(); }
    g.a.mem.write_u32(im + 10, data).unwrap();
    g.a.mem.write_u8(im + 14, 1).unwrap();
    g.a.mem.write_u8(im + 15, 2).unwrap();
    g.run(&[intu(DRAW_IMAGE, &[a(0, rp), a(1, im), d(0, 10), d(1, 5)])]);
    assert_eq!(g.rows(bm, 13, 6, 8, 2), vec![vec![3, 2, 3, 2, 3, 2, 3, 2], vec![3, 3, 3, 3, 2, 2, 2, 2]], "plane 0 from the image, plane 1 from PlaneOnOff; at (3+10, 1+5)");
    // a Border: 3 points, pens 5/0, JAM1
    let xy = g.words(&[0, 0, 6, 0, 6, 4]);
    let bd = g.zeroed(16);
    g.a.mem.write_u8(bd + 4, 5).unwrap(); g.a.mem.write_u8(bd + 7, 3).unwrap(); g.a.mem.write_u32(bd + 8, xy).unwrap();
    g.run(&[intu(DRAW_BORDER, &[a(0, rp), a(1, bd), d(0, 20), d(1, 10)])]);
    assert_eq!((g.px(bm, 20, 10), g.px(bm, 26, 10), g.px(bm, 26, 14), g.px(bm, 20, 14)), (Some(5), Some(5), Some(5), Some(0)), "an L: right then down; control: the far corner");
    // IntuiText "Hi" at (2,1)+(4,12): top-left of the text box
    let hi = g.s("Hi");
    let it = g.zeroed(20);
    g.a.mem.write_u8(it, 6).unwrap(); g.a.mem.write_u8(it + 1, 1).unwrap(); g.a.mem.write_u8(it + 2, 0).unwrap();
    g.a.mem.write_u16(it + 4, 2).unwrap(); g.a.mem.write_u16(it + 6, 1).unwrap(); g.a.mem.write_u32(it + 12, hi).unwrap();
    g.run(&[intu(PRINT_ITEXT, &[a(0, rp), a(1, it), d(0, 4), d(1, 12)]), intu(INTUI_TEXT_LENGTH, &[a(0, it)])]);
    let f = heirloom_raster::font::Font::gute_form();
    assert_eq!(g.result(1) as i32, f.text_length(b"Hi"), "IntuiTextLength in the default (Gute Form) face");
    // 'H': box top = 12+1 = 13, left = 4+2 = 6; the first inked pixel of the glyph, shifted by its CharKern
    let (ix, iy) = (0..f.y_size as i32).flat_map(|y| (0..9).map(move |x| (x, y))).find(|&(x, y)| f.glyph_bit(f.slot(b'H'), x, y)).unwrap();
    let (px, py) = (6 + f.kern.as_ref().unwrap()[f.slot(b'H')] as i32 + ix, 13 + iy);
    assert_eq!((g.px(bm, px, py), g.px(bm, px, py - 1 - iy), g.px(bm, 30, py)), (Some(6), Some(0), Some(0)), "ink in FrontPen 6, JAM1 leaves the paper (and nothing above the box)");
}

/// The plane route under an installed clip region: a fill is named over each of the region's rectangles (what it really painted), a
/// text run inside one rectangle is named as drawn, and only a run the region cuts is a hole, counted under that reason.
#[test]
fn a_fill_under_a_clip_region_is_named_per_rectangle_and_only_a_cut_op_is_a_hole() {
    use heirloom_present::plane::Rec;
    use heirloom_ui_abi::DrawOp;
    let mut g = G::new_plane();
    let nw = g.zeroed(48);
    for (o, v) in [(0u32, 0u16), (2, 0), (4, 120), (6, 40)] { g.a.mem.write_u16(nw + o, v).unwrap(); }
    g.a.mem.write_u32(nw + 14, 0x800).unwrap();
    let all = rect(&mut g, 0, 0, 119, 39);
    let gap = rect(&mut g, 50, 0, 59, 39); // the region is the window minus a 10 px column: two rectangles side by side
    g.run(&[intu(OPEN_WINDOW, &[a(0, nw)]), gfx(NEW_REGION, &[]), gfx(OR_RECT_REGION, &[a_res(0, 1), a(1, all)]), gfx(CLEAR_RECT_REGION, &[a_res(0, 1), a(1, gap)])]);
    let (win, rg) = (g.result(0), g.result(1));
    let (rp, layer) = (g.r32(win + 50), g.r32(win + 124));
    let lname = g.s("layers.library");
    g.run(&[(EXEC_BASE, OPEN_LIBRARY, vec![a(1, lname), d(0, 0)])]);
    let layers = g.result(0);
    let (inside, cut) = (g.s("ab"), g.s("abcdefghijkl"));
    g.run(&[(layers, INSTALL_CLIP_REGION, vec![a(0, layer), a(1, rg)]), gfx(SET_APEN, &[a(1, rp), d(0, 2)]),
        gfx(RECT_FILL, &[a(1, rp), d(0, 0), d(1, 0), d(2, 119), d(3, 39)])]);
    let fills: Vec<(i32, i32, i32, i32)> = g.a.gfx.present.recs(win).iter().filter_map(|r| match r { Rec::Op { op: DrawOp::Fill { rect, .. }, .. } => Some((rect.left, rect.top, rect.width, rect.height)), _ => None }).collect();
    assert!(g.a.gfx.present.recs(win).iter().all(|r| !matches!(r, Rec::Hole(_))), "a fill under a region is no hole: {:?}", g.a.gfx.present.recs(win));
    let painted: i32 = fills.iter().map(|f| f.2 * f.3).sum();
    assert!(fills.iter().all(|f| f.0 + f.2 <= 50 || f.0 >= 60), "no fill reaches into the excluded column: {fills:?}");
    assert_eq!(painted, (120 - 10) * 40, "the fills cover exactly the region's part of the rectangle: {fills:?}");
    // A run inside one rectangle is named as drawn; one the region cuts (it crosses the excluded column) is a hole, by name.
    g.run(&[gfx(MOVE, &[a(1, rp), d(0, 4), d(1, 20)]), gfx(TEXT, &[a(1, rp), a(0, inside), d(0, 2)]),
        gfx(MOVE, &[a(1, rp), d(0, 30), d(1, 30)]), gfx(TEXT, &[a(1, rp), a(0, cut), d(0, 12)])]);
    let recs = g.a.gfx.present.recs(win);
    assert!(recs.iter().any(|r| matches!(r, Rec::Op { op: DrawOp::Text { string, .. }, .. } if string == "ab")), "{recs:?}");
    assert!(!recs.iter().any(|r| matches!(r, Rec::Op { op: DrawOp::Text { string, .. }, .. } if string == "abcdefghijkl")), "a cut run is not named as if whole");
    assert_eq!(recs.iter().filter(|r| matches!(r, Rec::Hole(_))).count(), 1, "the cut run is the one hole: {recs:?}");
    assert!(g.a.gfx.present.hole_why.keys().any(|k| k == "Text: a clip region cuts the op"), "{:?}", g.a.gfx.present.hole_why);
    // NEGATIVE CONTROL: with the region removed the same text is named, and no new hole appears.
    g.run(&[(layers, INSTALL_CLIP_REGION, vec![a(0, layer), a(1, 0)]), gfx(MOVE, &[a(1, rp), d(0, 30), d(1, 10)]), gfx(TEXT, &[a(1, rp), a(0, cut), d(0, 12)])]);
    let recs = g.a.gfx.present.recs(win);
    assert!(recs.iter().any(|r| matches!(r, Rec::Op { op: DrawOp::Text { string, .. }, .. } if string == "abcdefghijkl")));
    assert_eq!(recs.iter().filter(|r| matches!(r, Rec::Hole(_))).count(), 1);
}

/// A program that draws a default icon ITSELF (a BitMap whose planes are the icon's ImageData, blitted whole, as Directory Opus's
/// backdrop does) has drawn that icon: the plane records the host's Disk icon, not the pixels. Only the whole image from its data counts.
#[test]
fn a_blit_of_an_icons_own_image_data_is_that_icon_and_anything_else_stays_pixels() {
    use heirloom_present::plane::Rec;
    use heirloom_ui_abi::{DrawOp, IconRole};
    const GET_DEF_DISK_OBJECT: i32 = -120;
    let mut g = G::new_plane();
    let art: Vec<u32> = (0..32 * 32u32).map(|i| if i % 5 == 0 { 0xFF00_0000 } else { 0xFF44_88DD }).collect();
    g.a.ui.icon_art.insert(IconRole::Disk, (32, 32, art));
    let nw = g.zeroed(48);
    for (o, v) in [(0u32, 0u16), (2, 0), (4, 200), (6, 80)] { g.a.mem.write_u16(nw + o, v).unwrap(); }
    g.a.mem.write_u32(nw + 14, 0x800).unwrap();
    let iname = g.s("icon.library");
    g.run(&[intu(OPEN_WINDOW, &[a(0, nw)]), (EXEC_BASE, OPEN_LIBRARY, vec![a(1, iname), d(0, 0)])]);
    let (win, icon) = (g.result(0), g.result(1));
    g.run(&[(icon, GET_DEF_DISK_OBJECT, vec![d(0, 1)])]);
    let dobj = g.result(0);
    let img = g.r32(dobj + 22);
    let (w, h, depth, data) = (g.a.mem.read_u16(img + 4).unwrap(), g.a.mem.read_u16(img + 6).unwrap(), g.a.mem.read_u16(img + 8).unwrap(), g.r32(img + 10));
    // A struct BitMap over `planes` (BytesPerRow, Rows, Flags, Depth, pad, Planes[8]).
    let bitmap_over = |g: &mut G, base: u32| {
        let bpr = (u32::from(w) + 15) / 16 * 2;
        let bm = g.zeroed(40);
        g.a.mem.write_u16(bm, bpr as u16).unwrap();
        g.a.mem.write_u16(bm + 2, h).unwrap();
        g.a.mem.write_u8(bm + 5, depth as u8).unwrap();
        for p in 0..u32::from(depth) { g.a.mem.write_u32(bm + 8 + 4 * p, base + p * bpr * u32::from(h)).unwrap(); }
        bm
    };
    let rp = g.r32(win + 50);
    let own = bitmap_over(&mut g, data);
    let blit = |src: u32, sx: u32, x: u32, y: u32, bw: u16| gfx(BLT_BITMAP_RASTPORT, &[a(0, src), d(0, sx), d(1, 0), a(1, rp), d(2, x), d(3, y), d(4, u32::from(bw)), d(5, u32::from(h)), d(6, 0xC0)]);
    g.run(&[blit(own, 0, 10, 20, w)]);
    let icons = |g: &G| g.a.gfx.present.recs(win).iter().filter_map(|r| match r { Rec::Op { op: DrawOp::Icon { rect, role }, .. } => Some((*role, rect.width, rect.height)), _ => None }).collect::<Vec<_>>();
    let holes = |g: &G| g.a.gfx.present.recs(win).iter().filter(|r| matches!(r, Rec::Hole(_))).count();
    assert_eq!(icons(&g), [(IconRole::Disk, i32::from(w), i32::from(h))], "{:?}", g.a.gfx.present.recs(win));
    assert_eq!(holes(&g), 0);
    // NEGATIVE CONTROLS: part of the image (from x = 1) is pixels, and the same pixels copied from other memory are pixels too.
    g.run(&[blit(own, 1, 100, 20, w - 1)]);
    assert_eq!((icons(&g).len(), holes(&g)), (1, 1), "a partial copy is not the icon: {:?}", g.a.gfx.present.recs(win));
    let n = usize::from(depth) * ((usize::from(w) + 15) / 16 * 2) * usize::from(h);
    let copy: Vec<u8> = (0..n as u32).map(|i| g.a.mem.read_u8(data + i).unwrap()).collect();
    let elsewhere = g.bytes(&copy);
    let other = bitmap_over(&mut g, elsewhere);
    g.run(&[blit(other, 0, 150, 20, w)]);
    assert_eq!((icons(&g).len(), holes(&g)), (1, 2), "identical pixels from other memory are not the icon: provenance, not looks");
}
