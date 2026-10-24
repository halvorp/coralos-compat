//! TextEdit drawing: a text edit record's lines, selection highlight and caret drawn through the engine's own QuickDraw into the
//! record's `inPort`, so the program's content plane carries them exactly as it carries any other drawing of the program.
//!
//! One routine ([`paint`]) does what Executor's `te_draw` does: set the port's text characteristics from the record, clip to the
//! view rectangle (and the port's own clip), erase the target rectangle with the background pattern, draw each visible line at its
//! baseline (`destRect.top + line * lineHeight + fontAscent`, flush by `just`), then invert the selection. A carriage return is
//! never drawn: it ends a line and has no width. The port's pen position, text characteristics and clip are put back after.
use super::layout::{self, CR};
use crate::pascal::Rect;
use crate::qd::draw::paint as paint_region;
use crate::qd::port::{raster, read_port_pat, set_pen_loc, read_pen, PatSlot, PAT_COPY, P_CLIP, P_TX_FACE, P_TX_FONT, P_TX_MODE, P_TX_SIZE};
use crate::machine::Mac;
use crate::Refusal;
use guestui_abi::GuestUi;
use retro_raster::Region;

/// The patXor transfer mode, with a black pattern: QuickDraw's `InvertRect`.
const PAT_XOR: u16 = 10;
const BLACK: [u8; 8] = [0xFF; 8];

/// What a drawing pass needs from a record (or, for `TETextBox`, from the port).
#[derive(Debug, Clone, Copy)]
pub struct Geo { pub dest: Rect, pub view: Rect, pub line_height: i16, pub ascent: i16, pub just: i16, pub font: u16, pub face: u8, pub mode: u16, pub size: u16 }

/// What to show on top of the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark { None, Range(usize, usize), Caret(usize), /** a deactivated record's selection with `teFOutlineHilite` set */ Outline(usize, usize) }

fn ir(r: Rect) -> retro_raster::Rect { raster(r) }

/// The insertion point's one-pixel-wide rectangle (LOCAL coordinates).
pub fn caret_rect(g: &Geo, text: &[u8], starts: &[u16], sel: usize, w: &dyn Fn(u8) -> i32) -> Rect {
    let (v, h) = layout::char_to_point(text, starts, sel, i32::from(g.line_height), i32::from(g.dest.right) - i32::from(g.dest.left), g.just, w);
    let (top, left) = (i32::from(g.dest.top) + v, i32::from(g.dest.left) + h);
    Rect { top: top as i16, left: left as i16, bottom: (top + i32::from(g.line_height)) as i16, right: (left + 1) as i16 }
}

/// The rectangles a selection range covers, one per line. A selection that runs through a line's end (a CR, or into the next
/// line) is highlighted to the destination's right edge; one that stops at the text's end stops there.
fn range_rects(g: &Geo, text: &[u8], starts: &[u16], a: usize, b: usize, w: &dyn Fn(u8) -> i32) -> Region {
    let mut rg = Region::new();
    let width = i32::from(g.dest.right) - i32::from(g.dest.left);
    for line in 0..starts.len().saturating_sub(1) {
        let (ls, le) = (usize::from(starts[line]), usize::from(starts[line + 1]));
        if b <= ls || a >= le { continue; }
        let off = layout::line_offset(text, starts, line, width, g.just, w);
        let from = off + layout::span_width(text, ls, a.max(ls), w);
        let through_end = b > le || (b == le && text.get(le.wrapping_sub(1)) == Some(&CR));
        let to = if through_end { width } else { off + layout::span_width(text, ls, b.min(le), w) };
        let top = i32::from(g.dest.top) + line as i32 * i32::from(g.line_height);
        rg.or_rect(retro_raster::Rect::new(i32::from(g.dest.left) + from, top, i32::from(g.dest.left) + to.max(from), top + i32::from(g.line_height)));
    }
    rg
}

/// The one-pixel boundary of `r`: its pixels that have a neighbour (left, right, above, below) outside it.
fn outline_of(r: &Region) -> Region {
    let mut inner = r.clone();
    for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] { let mut s = r.clone(); s.translate(dx, dy); inner.and_region(&s); }
    let mut out = r.clone();
    out.clear_region(&inner);
    out
}

/// The width of every byte in the font that is drawn (the layout's `w` function).
pub fn width_table(font: &retro_raster::font::Font) -> [i32; 256] {
    let mut t = [0i32; 256];
    for (c, w) in t.iter_mut().enumerate() { *w = font.text_length(&[c as u8]); }
    t
}

/// Draw `text` (lines `starts`) into `port` inside `area` (LOCAL), then the mark. See the module note.
pub fn paint<U: GuestUi>(m: &mut Mac<U>, port: u32, g: &Geo, text: &[u8], starts: &[u16], area: Rect, mark: Mark, style: Option<&super::styles::StyleTab>) -> Result<(), Refusal> {
    let clip_box = ir(area).intersect(&ir(g.view));
    if clip_box.is_empty() { return Ok(()); }
    // the clip: the target rectangle, sectioned with the port's own clip (the ROM's `ROMlib_tesave` does the same)
    let saved = m.mem.read_u32(port.wrapping_add(P_CLIP)).map_err(Refusal::mem("clipRgn"))?;
    let mut rg = Region::from_rect(clip_box);
    if saved != 0 { rg.and_region(&m.read_region(saved)?); }
    let tmp = m.new_region(&rg)?;
    m.mem.write_u32(port.wrapping_add(P_CLIP), tmp).map_err(Refusal::mem("clipRgn"))?;
    let saved_tx = (m.mem.read_u16(port.wrapping_add(P_TX_FONT)), m.mem.read_u8(port.wrapping_add(P_TX_FACE)), m.mem.read_u16(port.wrapping_add(P_TX_MODE)), m.mem.read_u16(port.wrapping_add(P_TX_SIZE)));
    let pen = read_pen(&m.mem, port)?;
    let r = lines(m, port, g, text, starts, clip_box, mark, style);
    // put everything back, whatever happened
    m.mem.write_u32(port.wrapping_add(P_CLIP), saved).map_err(Refusal::mem("clipRgn"))?;
    m.dispose_handle(tmp).map_err(|e| Refusal(alloc::format!("TextEdit: DisposeRgn of the temporary clip failed ({e})")))?;
    if let (Ok(f), Ok(fa), Ok(mo), Ok(sz)) = saved_tx {
        for (off, v) in [(P_TX_FONT, f), (P_TX_MODE, mo), (P_TX_SIZE, sz)] { m.mem.write_u16(port.wrapping_add(off), v).map_err(Refusal::mem("text characteristics"))?; }
        m.mem.write_u8(port.wrapping_add(P_TX_FACE), fa).map_err(Refusal::mem("txFace"))?;
    }
    set_pen_loc(&mut m.mem, port, pen.v, pen.h)?;
    r
}

fn lines<U: GuestUi>(m: &mut Mac<U>, port: u32, g: &Geo, text: &[u8], starts: &[u16], area: retro_raster::Rect, mark: Mark, style: Option<&super::styles::StyleTab>) -> Result<(), Refusal> {
    let bk = read_port_pat(&m.mem, port, PatSlot::Bk)?;
    paint_region(m, port, &Region::from_rect(area), PAT_COPY, &bk)?;
    for (off, v) in [(P_TX_FONT, g.font), (P_TX_MODE, g.mode), (P_TX_SIZE, g.size)] { m.mem.write_u16(port.wrapping_add(off), v).map_err(Refusal::mem("text characteristics"))?; }
    m.mem.write_u8(port.wrapping_add(P_TX_FACE), g.face).map_err(Refusal::mem("txFace"))?;
    let tab = width_table(&m.qd.font);
    let w = |c: u8| tab[usize::from(c)];
    let width = i32::from(g.dest.right) - i32::from(g.dest.left);
    let lh = i32::from(g.line_height).max(1);
    for line in 0..starts.len().saturating_sub(1) {
        let top = i32::from(g.dest.top) + line as i32 * lh;
        if top + lh <= area.y0 || top >= area.y1 { continue; }
        let (ls, le) = (usize::from(starts[line]), usize::from(starts[line + 1]));
        let mut end = le.min(text.len());
        while end > ls.min(end) && text[end - 1] == CR { end -= 1; }
        let ls = ls.min(end);
        if ls == end { continue; }
        let h = i32::from(g.dest.left) + layout::line_offset(text, starts, line, width, g.just, &w);
        set_pen_loc(&mut m.mem, port, (top + i32::from(g.ascent)) as i16, h as i16)?;
        match style {
            // a styled record: one piece per run, each drawn with its run's font, face and size (the pen carries on)
            Some(tab) => {
                let mut at = ls;
                while at < end {
                    let run = tab.runs.iter().rposition(|r| usize::from(r.0) <= at).unwrap_or(0);
                    let stop = tab.runs.get(run + 1).map_or(end, |r| usize::from(r.0)).min(end).max(at + 1);
                    let a = tab.styles.get(tab.runs[run].1 as usize).map(|s| s.attrs).unwrap_or_default();
                    for (off, v) in [(P_TX_FONT, a.font as u16), (P_TX_SIZE, a.size as u16)] { m.mem.write_u16(port.wrapping_add(off), v).map_err(Refusal::mem("text characteristics"))?; }
                    m.mem.write_u8(port.wrapping_add(P_TX_FACE), a.face).map_err(Refusal::mem("txFace"))?;
                    crate::qd::text::draw_on(m, port, &text[at..stop], "TEUpdate")?;
                    at = stop;
                }
            }
            None => { crate::qd::text::draw_on(m, port, &text[ls..end], "TEUpdate")?; }
        }
    }
    let inverted = match mark {
        Mark::None => return Ok(()),
        Mark::Range(a, b) => range_rects(g, text, starts, a, b, &w),
        Mark::Caret(at) => Region::from_rect(ir(caret_rect(g, text, starts, at, &w))),
        Mark::Outline(a, b) => return paint_region(m, port, &outline_of(&range_rects(g, text, starts, a, b, &w)), PAT_COPY, &BLACK),
    };
    paint_region(m, port, &inverted, PAT_XOR, &BLACK)
}

/// Invert just the caret (TEIdle's blink): the same pixels [`paint`] puts down for [`Mark::Caret`].
pub fn toggle_caret<U: GuestUi>(m: &mut Mac<U>, port: u32, g: &Geo, text: &[u8], starts: &[u16], sel: usize) -> Result<(), Refusal> {
    let tab = width_table(&m.qd.font);
    let w = |c: u8| tab[usize::from(c)];
    let r = ir(caret_rect(g, text, starts, sel, &w)).intersect(&ir(g.view));
    if r.is_empty() { return Ok(()); }
    paint_region(m, port, &Region::from_rect(r), PAT_XOR, &BLACK)
}
