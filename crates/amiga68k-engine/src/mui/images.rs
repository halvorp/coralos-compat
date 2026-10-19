//! The image classes draw: Bitmap.mui, ChunkyImage.mui, Bodychunk.mui and Imagedisplay.mui become a content PLANE in the rectangle layout
//! gave them (`GuestUi::present_plane`), as `Pixels` for what they hold as pixels and as a theme `Fill`, `Line` or `Icon` where the image is a
//! named thing (an image spec that is a standard background, pen, arrow, check mark or disk glyph). Facts: Zune's bitmap.c (draw clipped
//! to the object's box from its top-left; `MUIA_Bitmap_Transparent` masks one pen; `MUIA_Bitmap_SourceColors` are the pens' RGB32 triples, the
//! remap to the screen is `ObtainBestPenA` and here the host just uses the colour), chunkyimage.c (`width * height` pen bytes, row stride
//! `MUIA_ChunkyImage_Modulo`, 3-byte RGB palette of `NumColors` <= 256), bodychunk.c (an ILBM BODY: `depth` planes per row, rows padded to 16 bits,
//! optional ByteRun1 per row, a mask plane is skipped) and imagedisplay.c + imspec.c (`<type>:<data>`; sizes: Imagedisplay min 3, default 16,
//! Bitmap min = default 1; both stretch). The class sources are APL; behaviour only, nothing copied.
//!
//! NOT drawn, each a NAMED refusal when asked: image specs of type 3..7 (an image file through a picture datatype, gradients, vectors) and the
//! standard images with no glyph here (cycle, popup, tape, knob); a Bitmap whose BitMap the engine does not know.
use alloc::format;
use alloc::vec::Vec;
use super::classes::Kind;
use super::ids_gen::*;
use crate::{machine::Amiga, Refusal};
use guestui_abi::{DrawOp, GuestPoint, GuestRect, GuestUi, IconRole, PenRole, Plane};

/// Is `k` one of the classes this module draws?
pub fn is_image(k: Kind) -> bool { matches!(k, Kind::Bitmap | Kind::ChunkyImage | Kind::Bodychunk | Kind::Imagedisplay) }

/// MUI's size answer for the class (min, default): Bitmap family 1x1, Imagedisplay 3x3 / 16x16. Both stretch.
pub fn size(k: Kind) -> (i32, i32) { let d = if k == Kind::Imagedisplay { 16 } else { 1 }; (d, d) }

/// A decoded picture: `w x h` colours and which pixels are drawn (a transparent pen is not).
struct Pic { w: i32, h: i32, rgb: Vec<u32>, solid: Vec<bool> }

fn rd8<U: GuestUi>(a: &Amiga<U>, at: u32, what: &'static str) -> Result<u8, Refusal> { a.mem.read_u8(at).map_err(Refusal::mem(what)) }
fn rgb8(r: u32, g: u32, b: u32) -> u32 { (r & 0xFF) << 16 | (g & 0xFF) << 8 | (b & 0xFF) }

/// The colour of pen `pen`: the object's `MUIA_Bitmap_SourceColors` table (ULONG red, green, blue per pen, the top byte of each used) when
/// it has one, else the screen palette (through `MUIA_Bitmap_MappingTable` when the program gave one).
fn colour<U: GuestUi>(a: &mut Amiga<U>, obj: u32, pen: u32, pens: u32) -> Result<u32, Refusal> {
    let src = a.objects.num_or(obj, MUIA_Bitmap_SourceColors, 0);
    if src != 0 {
        if pen >= pens.max(1) { return Ok(0); }
        let g = |i: u32| a.mem.read_u32(src + 4 * (pen * 3 + i)).map(|v| v >> 24).map_err(Refusal::mem("MUIA_Bitmap_SourceColors"));
        return Ok(rgb8(g(0)?, g(1)?, g(2)?));
    }
    let table = a.objects.num_or(obj, MUIA_Bitmap_MappingTable, 0);
    let screen = if table != 0 { rd8(a, table + pen, "MUIA_Bitmap_MappingTable")? as usize } else { pen as usize & 0xFF };
    Ok(a.gfx.palette().rgb[screen])
}

fn transparent<U: GuestUi>(a: &Amiga<U>, obj: u32) -> Option<u32> { match a.objects.num(obj, MUIA_Bitmap_Transparent) { Some(v) if v != u32::MAX => Some(v), _ => None } }

fn dims<U: GuestUi>(a: &Amiga<U>, obj: u32) -> (i32, i32) { (a.objects.num_or(obj, MUIA_Bitmap_Width, 0) as i32, a.objects.num_or(obj, MUIA_Bitmap_Height, 0) as i32) }

/// Bitmap.mui: the program's `struct BitMap`.
fn pic_bitmap<U: GuestUi>(a: &mut Amiga<U>, obj: u32) -> Result<Option<Pic>, Refusal> {
    let bm = a.objects.num_or(obj, MUIA_Bitmap_Bitmap, 0);
    let (w, h) = dims(a, obj);
    if bm == 0 || w < 1 || h < 1 { return Ok(None); } // bitmap.c Draw: nothing without a bitmap and a size
    crate::graphics::bitmaps::ensure_source(a, bm)?;
    let (pens, bw, bh, depth) = {
        let g = a.gfx.bitmaps.get(&bm).ok_or_else(|| Refusal(format!("MUIA_Bitmap_Bitmap {bm:#x}: not a BitMap the engine knows")))?;
        (g.bm.pixels().to_vec(), g.bm.width(), g.bm.height(), g.bm.depth())
    };
    let (w, h) = (w.min(bw), h.min(bh));
    let tr = transparent(a, obj);
    let mut cache = alloc::collections::BTreeMap::new();
    let (mut rgb, mut solid) = (Vec::with_capacity((w * h) as usize), Vec::with_capacity((w * h) as usize));
    for y in 0..h { for x in 0..w {
        let pen = pens[(y * bw + x) as usize] as u32;
        let c = match cache.get(&pen) { Some(&c) => c, None => { let c = colour(a, obj, pen, 1 << depth)?; cache.insert(pen, c); c } };
        rgb.push(c); solid.push(tr != Some(pen));
    } }
    Ok(Some(Pic { w, h, rgb, solid }))
}

/// ChunkyImage.mui: `width * height` pen bytes with a 3-byte-per-colour palette.
fn pic_chunky<U: GuestUi>(a: &mut Amiga<U>, obj: u32) -> Result<Option<Pic>, Refusal> {
    let (w, h) = dims(a, obj);
    let (px, pal, n) = (a.objects.num_or(obj, MUIA_ChunkyImage_Pixels, 0), a.objects.num_or(obj, MUIA_ChunkyImage_Palette, 0), a.objects.num_or(obj, MUIA_ChunkyImage_NumColors, 0));
    if px == 0 || w < 1 || h < 1 || n < 1 || n > 256 {
        return Err(Refusal(format!("ChunkyImage.mui {obj:#x}: pixels {px:#x}, {w}x{h}, {n} colours: chunkyimage.c make_bitmap fails, so MUIM_Setup is FALSE and Zune would not open the window")));
    }
    let modulo = match a.objects.num(obj, MUIA_ChunkyImage_Modulo) { Some(m) if m != 0xFFFF => m, _ => w as u32 };
    let tr = transparent(a, obj);
    let (mut rgb, mut solid) = (Vec::new(), Vec::new());
    for y in 0..h as u32 { for x in 0..w as u32 {
        let pen = rd8(a, px + y * modulo + x, "MUIA_ChunkyImage_Pixels")? as u32;
        let c = if pal != 0 && pen < n { rgb8(rd8(a, pal + pen * 3, "ChunkyImage palette")? as u32, rd8(a, pal + pen * 3 + 1, "ChunkyImage palette")? as u32, rd8(a, pal + pen * 3 + 2, "ChunkyImage palette")? as u32) }
            else { colour(a, obj, pen, n)? };
        rgb.push(c); solid.push(tr != Some(pen));
    } }
    Ok(Some(Pic { w, h, rgb, solid }))
}

/// ByteRun1 (bodychunk.c unpack_byterun1): `want` bytes unpacked from `at`; returns them and the address after the packed row.
fn unpack<U: GuestUi>(a: &Amiga<U>, mut at: u32, want: usize) -> Result<(Vec<u8>, u32), Refusal> {
    let mut out = Vec::with_capacity(want);
    while out.len() < want {
        let c = rd8(a, at, "MUIA_Bodychunk_Body")? as i8; at += 1;
        if c >= 0 { for _ in 0..=c as usize { if out.len() < want { out.push(rd8(a, at, "MUIA_Bodychunk_Body")?); } at += 1; } }
        else if c != -128 { let r = rd8(a, at, "MUIA_Bodychunk_Body")?; at += 1; for _ in 0..=(-(c as i32)) as usize { if out.len() < want { out.push(r); } } }
    }
    Ok((out, at))
}

/// Bodychunk.mui: an ILBM BODY (planar rows, `depth` planes, optionally a mask plane after them, rows 16-bit padded, optionally ByteRun1).
fn pic_body<U: GuestUi>(a: &mut Amiga<U>, obj: u32) -> Result<Option<Pic>, Refusal> {
    let (w, h) = dims(a, obj);
    let (body, depth, comp, mask) = (a.objects.num_or(obj, MUIA_Bodychunk_Body, 0), a.objects.num_or(obj, MUIA_Bodychunk_Depth, 0), a.objects.num_or(obj, MUIA_Bodychunk_Compression, 0) & 0xFF, a.objects.num_or(obj, MUIA_Bodychunk_Masking, 0) & 0xFF);
    if body == 0 || w < 1 || h < 1 || depth < 1 || depth > 8 || comp > 1 {
        return Err(Refusal(format!("Bodychunk.mui {obj:#x}: body {body:#x}, {w}x{h}x{depth}, compression {comp}: bodychunk.c make_bitmap fails, so MUIM_Setup is FALSE and Zune would not open the window")));
    }
    let bpr = (((w + 15) & !15) / 8) as usize;
    let bpl = bpr * (depth as usize + (mask == 1) as usize);
    let (mut at, tr) = (body, transparent(a, obj));
    let (mut rgb, mut solid) = (Vec::new(), Vec::new());
    for _ in 0..h {
        let row: Vec<u8> = if comp == 1 { let (r, next) = unpack(a, at, bpl)?; at = next; r } else { let r = (0..bpl as u32).map(|i| rd8(a, at + i, "MUIA_Bodychunk_Body")).collect::<Result<Vec<_>, _>>()?; at += bpl as u32; r };
        for x in 0..w as usize {
            let pen = (0..depth as usize).fold(0u32, |p, pl| p | (((row[pl * bpr + x / 8] >> (7 - x % 8)) & 1) as u32) << pl);
            rgb.push(colour(a, obj, pen, 1 << depth)?); solid.push(tr != Some(pen));
        }
    }
    Ok(Some(Pic { w, h, rgb, solid }))
}

/// The ops that show `pic` from `(left, top)`, clipped to `max` pixels: one `Pixels` op when everything is drawn, one per opaque run otherwise.
fn pic_ops(pic: &Pic, left: i32, top: i32, max: (i32, i32)) -> Vec<DrawOp> {
    let (w, h) = (pic.w.min(max.0), pic.h.min(max.1));
    if w < 1 || h < 1 { return Vec::new(); }
    let at = |x: i32, y: i32| (y * pic.w + x) as usize;
    if (0..h).all(|y| (0..w).all(|x| pic.solid[at(x, y)])) {
        return alloc::vec![DrawOp::Pixels { rect: GuestRect { left, top, width: w, height: h }, rgb: (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| pic.rgb[at(x, y)]).collect() }];
    }
    let mut ops = Vec::new();
    for y in 0..h {
        let mut x = 0;
        while x < w {
            if !pic.solid[at(x, y)] { x += 1; continue; }
            let start = x;
            while x < w && pic.solid[at(x, y)] { x += 1; }
            ops.push(DrawOp::Pixels { rect: GuestRect { left: left + start, top: top + y, width: x - start, height: 1 }, rgb: (start..x).map(|i| pic.rgb[at(i, y)]).collect() });
        }
    }
    ops
}

// ---- Imagedisplay: image specs ------------------------------------------------------------------------------------------------------------

/// MUI pen (`MPEN_*`, classes/window.h) -> theme role.
fn mui_pen_role(n: i32) -> Option<PenRole> {
    Some(match n { 0 | 1 => PenRole::Shine, 2 => PenRole::Background, 3 | 4 => PenRole::Shadow, 5 => PenRole::Text, 6 => PenRole::Fill, 7 => PenRole::Highlight, _ => return None })
}

/// `MUII_*` background (type 0) -> theme role. The two-tone patterns (128..145) are drawn in their first pen.
fn background_role(n: u32) -> Option<PenRole> {
    Some(match n {
        MUII_WindowBack | MUII_RequesterBack | MUII_ButtonBack | MUII_ListBack | MUII_TextBack | MUII_PropBack | MUII_PopupBack | MUII_RegisterBack
        | MUII_GroupBack | MUII_SliderBack | MUII_SliderKnob | MUII_PageBack | MUII_ReadListBack | MUII_BACKGROUND => PenRole::Background,
        MUII_SelectedBack | MUII_FILL | MUII_FILLBACK | MUII_FILLSHINE | MUII_FILLBACK2 => PenRole::Fill,
        MUII_ListCursor | MUII_ListSelect | MUII_ListSelCur => PenRole::Selection,
        MUII_SHADOW | MUII_SHADOWBACK | MUII_SHADOWFILL | MUII_SHADOWSHINE | MUII_HSHADOWBACK | MUII_HSHADOWSHADOW => PenRole::Shadow,
        MUII_SHINE | MUII_SHINEBACK | MUII_HSHINEBACK | MUII_HSHINESHINE => PenRole::Shine,
        MUII_MARKSHINE | MUII_MARKHALFSHINE | MUII_MARKBACKGROUND => PenRole::Highlight,
        _ => return None,
    })
}

fn pt(x: i32, y: i32) -> GuestPoint { GuestPoint { x, y } }

/// A filled triangle pointing `dir` (0 up, 1 down, 2 left, 3 right) in the square centred in `r`, as scanline `Line`s in the text pen.
fn arrow(r: GuestRect, dir: u32) -> Vec<DrawOp> {
    let s = r.width.min(r.height).max(3) - 2;
    let (x0, y0) = (r.left + (r.width - s) / 2, r.top + (r.height - s) / 2);
    let mut ops = Vec::new();
    for i in 0..s {
        let half = (i * (s - 1) / 2) / (s - 1).max(1);
        let (a, b) = match dir {
            0 => (pt(x0 + s / 2 - half, y0 + i), pt(x0 + s / 2 + half, y0 + i)),
            1 => (pt(x0 + s / 2 - half, y0 + s - 1 - i), pt(x0 + s / 2 + half, y0 + s - 1 - i)),
            2 => (pt(x0 + i, y0 + s / 2 - half), pt(x0 + i, y0 + s / 2 + half)),
            _ => (pt(x0 + s - 1 - i, y0 + s / 2 - half), pt(x0 + s - 1 - i, y0 + s / 2 + half)),
        };
        ops.push(DrawOp::Line { a, b, pen_role: PenRole::Text });
    }
    ops
}

fn check_mark(r: GuestRect) -> Vec<DrawOp> {
    let (w, h) = (r.width.max(4), r.height.max(4));
    let p = |fx: i32, fy: i32| pt(r.left + w * fx / 10, r.top + h * fy / 10);
    let mut ops = Vec::new();
    for d in 0..2 {
        ops.push(DrawOp::Line { a: pt(p(2, 5).x, p(2, 5).y + d), b: pt(p(4, 8).x, p(4, 8).y + d), pen_role: PenRole::Text });
        ops.push(DrawOp::Line { a: pt(p(4, 8).x, p(4, 8).y + d), b: pt(p(8, 2).x, p(8, 2).y + d), pen_role: PenRole::Text });
    }
    ops
}

/// The ops of the image spec `spec` in rectangle `r`.
fn spec_ops<U: GuestUi>(a: &mut Amiga<U>, obj: u32, spec: &str, r: GuestRect) -> Result<Vec<DrawOp>, Refusal> {
    let (ty, data) = spec.split_once(':').ok_or_else(|| Refusal(format!("Imagedisplay.mui {obj:#x}: image spec \"{spec}\" has no type prefix")))?;
    let num = || data.trim().parse::<i64>().ok();
    let fill = |role| alloc::vec![DrawOp::Fill { rect: r, pen_role: role }];
    let nope = |what: &str| Refusal(format!("Imagedisplay.mui {obj:#x}: image spec \"{spec}\": {what} is not drawn"));
    Ok(match ty {
        "0" => fill(num().and_then(|n| background_role(n as u32)).ok_or_else(|| nope("this standard background"))?),
        "1" => match num().map(|n| n as u32) {
            Some(MUII_ArrowUp) => arrow(r, 0), Some(MUII_ArrowDown) => arrow(r, 1), Some(MUII_ArrowLeft) => arrow(r, 2), Some(MUII_ArrowRight) => arrow(r, 3),
            Some(MUII_CheckMark) => check_mark(r),
            Some(MUII_Disk | MUII_HardDisk) => alloc::vec![DrawOp::Icon { rect: r, role: IconRole::Disk }],
            Some(MUII_Volume) => alloc::vec![DrawOp::Icon { rect: r, role: IconRole::Volume }],
            Some(MUII_Drawer) => alloc::vec![DrawOp::Icon { rect: r, role: IconRole::Drawer }],
            Some(MUII_Chip | MUII_Network | MUII_Assign) => alloc::vec![DrawOp::Icon { rect: r, role: IconRole::Device }],
            _ => return Err(nope("this standard image (no glyph for it)")),
        },
        "2" => {
            let (kind, n) = (data.chars().next().unwrap_or(' '), data.get(1..).and_then(|s| s.trim().parse::<i32>().ok()));
            match (kind, n) {
                ('m', Some(n)) => fill(mui_pen_role(n).ok_or_else(|| nope("a MUI pen outside 0..8"))?),
                ('p', Some(n)) if (0..256).contains(&n) => alloc::vec![DrawOp::Pixels { rect: r, rgb: alloc::vec![a.gfx.palette().rgb[n as usize]; (r.width * r.height).max(0) as usize] }],
                _ => return Err(nope("this pen spec (system and RGB pens)")),
            }
        }
        other => return Err(nope(&format!("an image spec of type {other} (an image file through a datatype, a gradient or a vector image)"))),
    })
}

/// The display list of image object `obj` in `r` (window coordinates), or none when it has nothing to show.
pub fn ops<U: GuestUi>(a: &mut Amiga<U>, obj: u32, r: GuestRect) -> Result<Vec<DrawOp>, Refusal> {
    let Some(kind) = a.objects.kind(obj) else { return Ok(Vec::new()) };
    let pic = match kind {
        Kind::Bitmap => pic_bitmap(a, obj)?,
        Kind::ChunkyImage => pic_chunky(a, obj)?,
        Kind::Bodychunk => pic_body(a, obj)?,
        Kind::Imagedisplay => {
            let at = a.objects.num_or(obj, MUIA_Imagedisplay_Spec, 0);
            if at == 0 { return Ok(Vec::new()); }
            let spec = a.mem.read_cstr(at).map_err(Refusal::mem("MUIA_Imagedisplay_Spec"))?;
            return spec_ops(a, obj, &spec, r);
        }
        _ => None,
    };
    Ok(pic.map(|p| pic_ops(&p, r.left, r.top, (r.width, r.height))).unwrap_or_default())
}

/// Show `obj` in `r` as its content plane (id = the object), unless the host already shows exactly this.
pub fn present<U: GuestUi>(a: &mut Amiga<U>, win: guestui_abi::UiObjectRef, obj: u32, r: GuestRect) -> Result<(), Refusal> {
    if r.width <= 0 || r.height <= 0 { return Ok(()); }
    let ops = ops(a, obj, r)?;
    if ops.is_empty() && !a.objects.images.contains_key(&obj) { return Ok(()); } // nothing to show and nothing shown
    if a.objects.images.get(&obj).is_some_and(|(rr, oo)| *rr == r && *oo == ops) { return Ok(()); }
    a.objects.images.insert(obj, (r, ops.clone()));
    a.ui.present_plane(win, Plane { id: obj, rect: r, ops }).map_err(Refusal::ui)
}

/// Window `w` closed: what was shown for its images is forgotten.
pub fn forget_window<U: GuestUi>(a: &mut Amiga<U>, w: u32) {
    let mine: Vec<u32> = a.objects.images.keys().copied().filter(|&o| super::object::ancestor(&a.objects, o, Kind::Window) == Some(w)).collect();
    for o in mine { a.objects.images.remove(&o); }
}
