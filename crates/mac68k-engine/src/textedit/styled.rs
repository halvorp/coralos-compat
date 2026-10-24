//! Styled TextEdit in guest memory: `TEStyleNew` ($A83E), `TEGetOffset` ($A83C) and `_TEDispatch` ($A83D). The data model and its
//! layout are in [`super::styles`]; this file reads and writes it through the Memory Manager and implements the routines.
//!
//! `_TEDispatch` is a stack-selector trap: the caller pushes the arguments, then the selector WORD last (`move.w #sel,-(sp)`), so
//! the frame below the selector depends on the routine, like the List Manager's `_Pack0`. The selectors and frame sizes are Inside
//! Macintosh VI's, cross-read against systemless' `trap/dialog.rs` and Executor's `trapinfo` / `teIMV.cpp`:
//!
//! | sel | routine | params after the selector (bytes) | result |
//! |---|---|---|---|
//! | 0 | `TEStylePaste(hTE)` | 4 | |
//! | 1 | `TESetStyle(mode, newStyle^, redraw, hTE)` | 12 | |
//! | 2 | `TEReplaceStyle(mode, old^, new^, redraw, hTE)` | 16 | |
//! | 3 | `TEGetStyle(offset, VAR style, VAR lineHeight, VAR ascent, hTE)` | 18 | |
//! | 4 | `TEGetStyleHandle(hTE)` | 4 | TEStyleHandle |
//! | 5 | `TESetStyleHandle(h, hTE)` | 8 | |
//! | 6 | `TEGetStyleScrapHandle(hTE)` | 4 | StScrpHandle |
//! | 7 | `TEStyleInsert(text, length, hST, hTE)` | 16 | |
//! | 8 | `TEGetPoint(offset, hTE)` | 6 | Point |
//! | 9 | `TEGetHeight(endLine, startLine, hTE)` | 12 | LONGINT |
//! | 10 | `TEContinuousStyle(VAR mode, VAR style, hTE)` | 12 | BOOLEAN |
//! | 11 | `TEUseStyleScrap(start, stop, hST, redraw, hTE)` | 18 | |
//! | 12 | `TECustomHook(which, VAR addr, hTE)` | 10 | |
//! | 13 | `TENumStyles(start, stop, hTE)` | 12 | LONGINT |
//! | 14 | `TEFeatureFlag(feature, action, hTE)` | 8 | INTEGER |
//!
//! Implemented: `TESetStyle` (1), `TEGetStyle` (3), `TEGetStyleHandle` (4), `TESetStyleHandle` (5), `TEGetPoint` (8), `TEGetHeight` (9),
//! `TEContinuousStyle` (10), `TENumStyles` (13), `TEFeatureFlag` (14: auto-scroll is real, other features read as clear) and `TEStyleNew` /
//! `TEGetOffset`. Selectors 0 (`TEStylePaste`), 2, 6, 7, 11 and 12 pop their frame and are refused BY NAME (they need the style scrap
//! formats or hook procedures run as guest code, which the engine does not model yet).
//! NAMED LIMIT: one stand-in font draws everything, so a style's font id and size are stored and reported but do not change glyphs
//! or line heights (a face -- bold, italic, underline -- does).
use super::styles::*;
use alloc::string::String;
use crate::machine::Regs;
use super::*;

pub const SEL_FRAMES: [(u16, &str, u32); 15] = [
    (0, "TEStylePaste", 4), (1, "TESetStyle", 12), (2, "TEReplaceStyle", 16), (3, "TEGetStyle", 18), (4, "TEGetStyleHandle", 4),
    (5, "TESetStyleHandle", 8), (6, "TEGetStyleScrapHandle", 4), (7, "TEStyleInsert", 16), (8, "TEGetPoint", 6), (9, "TEGetHeight", 12),
    (10, "TEContinuousStyle", 12), (11, "TEUseStyleScrap", 18), (12, "TECustomHook", 10), (13, "TENumStyles", 12), (14, "TEFeatureFlag", 8),
];

fn rd16<U: GuestUi>(m: &Mac<U>, at: u32) -> Result<u16, Refusal> { m.mem.read_u16(at).map_err(Refusal::mem("TextEdit frame")) }
fn rd32<U: GuestUi>(m: &Mac<U>, at: u32) -> Result<u32, Refusal> { m.mem.read_u32(at).map_err(Refusal::mem("TextEdit frame")) }

impl<U: GuestUi> Mac<U> {
    /// Decode the style data behind a `TEStyleHandle` (`len` is the record's `teLength`).
    pub(super) fn te_style_read(&self, sh: u32, _len: usize) -> Result<(u32, StyleTab), Refusal> {
        let e = Refusal::mem("TEStyleRec");
        let p = self.mm.block(sh).map(|b| b.ptr).ok_or_else(|| Refusal(format!("styled TextEdit: the record's TEStyleHandle {sh:#x} is not a live handle")))?;
        let (n_runs, n_styles, st) = (self.mem.read_u16(p).map_err(&e)? as usize, self.mem.read_u16(p + 2).map_err(&e)? as usize, self.mem.read_u32(p + 4).map_err(&e)?);
        let mut tab = StyleTab::default();
        for i in 0..n_runs {
            let at = p + STYLE_REC_HEAD + 4 * i as u32;
            tab.runs.push((self.mem.read_u16(at).map_err(&e)?, self.mem.read_u16(at + 2).map_err(&e)? as i16));
        }
        let sp = self.mm.block(st).map(|b| b.ptr).ok_or_else(|| Refusal(format!("styled TextEdit: the style table {st:#x} is not a live handle")))?;
        for i in 0..n_styles {
            let at = sp + ST_ELEMENT * i as u32;
            let attrs = Attrs::read(self.mem.bytes(self.mem.span(at + 6, TEXT_STYLE).map_err(&e)?).map_err(&e)?).ok_or_else(|| Refusal(String::from("style element")))?;
            tab.styles.push(StyleEl { count: self.mem.read_u16(at).map_err(&e)? as i16, height: self.mem.read_u16(at + 2).map_err(&e)? as i16, ascent: self.mem.read_u16(at + 4).map_err(&e)? as i16, attrs });
        }
        Ok((sh, tab))
    }

    /// Write the style data back: runs (with the `{ len + 1, -1 }` sentinel) and the style table, resizing both handles.
    pub(super) fn te_style_write(&mut self, sh: u32, tab: &StyleTab, len: usize) -> Result<(), Refusal> {
        let e = Refusal::mem("TEStyleRec");
        let nomem = |what: &str| { let w = String::from(what); move |x: i16| Refusal(format!("styled TextEdit: no memory for the {w} (memFullErr {x})")) };
        self.mm.set_handle_size(&mut self.mem, &mut self.heap, sh, STYLE_REC_HEAD + 4 * (tab.runs.len() as u32 + 1)).map_err(nomem("run table"))?;
        let p = self.mm.block(sh).ok_or_else(|| Refusal(String::from("TEStyleRec lost its block")))?.ptr;
        let st = self.mem.read_u32(p + 4).map_err(&e)?;
        self.mem.write_u16(p, tab.runs.len() as u16).map_err(&e)?;
        self.mem.write_u16(p + 2, tab.styles.len() as u16).map_err(&e)?;
        for (i, &(start, idx)) in tab.runs.iter().enumerate() {
            let at = p + STYLE_REC_HEAD + 4 * i as u32;
            self.mem.write_u16(at, start).map_err(&e)?;
            self.mem.write_u16(at + 2, idx as u16).map_err(&e)?;
        }
        let at = p + STYLE_REC_HEAD + 4 * tab.runs.len() as u32;
        self.mem.write_u16(at, (len + 1) as u16).map_err(&e)?;
        self.mem.write_u16(at + 2, 0xFFFF).map_err(&e)?;
        self.mm.set_handle_size(&mut self.mem, &mut self.heap, st, ST_ELEMENT * tab.styles.len() as u32).map_err(nomem("style table"))?;
        let sp = self.mm.block(st).ok_or_else(|| Refusal(String::from("style table lost its block")))?.ptr;
        for (i, s) in tab.styles.iter().enumerate() {
            let at = sp + ST_ELEMENT * i as u32;
            self.mem.write_u16(at, s.count as u16).map_err(&e)?;
            self.mem.write_u16(at + 2, s.height as u16).map_err(&e)?;
            self.mem.write_u16(at + 4, s.ascent as u16).map_err(&e)?;
            self.mem.write_bytes(at + 6, &s.attrs.write()).map_err(&e)?;
        }
        Ok(())
    }

    /// `TESetText` on a styled record: one run of the first style.
    pub(super) fn te_style_reset(&mut self, sh: u32, len: usize) -> Result<(), Refusal> {
        let (_, mut tab) = self.te_style_read(sh, len)?;
        tab.runs = alloc::vec![(0, 0)];
        tab.stabilise();
        self.te_style_write(sh, &tab, len)
    }

    /// `lhTab`: one `{ height, ascent }` per line plus one (the ROM's spare element). The stand-in font has one height.
    pub(super) fn te_style_line_heights(&mut self, sh: u32, n_lines: usize) -> Result<(), Refusal> {
        let e = Refusal::mem("lhTab");
        let p = self.mm.block(sh).map(|b| b.ptr).ok_or_else(|| Refusal(String::from("TEStyleRec is not a live handle")))?;
        let lh = self.mem.read_u32(p + 8).map_err(&e)?;
        self.mm.set_handle_size(&mut self.mem, &mut self.heap, lh, 4 * (n_lines as u32 + 1)).map_err(|x| Refusal(format!("styled TextEdit: no memory for the line-height table (memFullErr {x})")))?;
        let lp = self.mm.block(lh).ok_or_else(|| Refusal(String::from("lhTab lost its block")))?.ptr;
        let (h, a) = (self.qd.font.y_size as u16, self.qd.font.baseline as u16 + 1);
        for i in 0..=n_lines as u32 { self.mem.write_u16(lp + 4 * i, h).map_err(&e)?; self.mem.write_u16(lp + 4 * i + 2, a).map_err(&e)?; }
        Ok(())
    }

    fn te_null_scrap(&self, sh: u32) -> Result<u32, Refusal> {
        let e = Refusal::mem("NullStRec");
        let p = self.mm.block(sh).map(|b| b.ptr).ok_or_else(|| Refusal(String::from("TEStyleRec is not a live handle")))?;
        let ns = self.mem.read_u32(p + 16).map_err(&e)?;
        let np = self.mm.block(ns).map(|b| b.ptr).ok_or_else(|| Refusal(format!("nullStyle {ns:#x} is not a live handle")))?;
        let scrap = self.mem.read_u32(np + 4).map_err(&e)?;
        self.mm.block(scrap).map(|b| b.ptr).ok_or_else(|| Refusal(format!("nullScrap {scrap:#x} is not a live handle")))
    }

    /// The style typed text takes right now at an insertion point, if a `TESetStyle` set one (the null style).
    pub(super) fn te_null_attrs(&self, sh: u32) -> Option<Attrs> {
        let sp = self.te_null_scrap(sh).ok()?;
        if self.mem.read_u16(sp).ok()? != 1 { return None; }
        Attrs::read(self.mem.bytes(self.mem.span(sp + 2 + 8, TEXT_STYLE).ok()?).ok()?)
    }
    pub(super) fn te_null_set(&mut self, sh: u32, a: Option<Attrs>) -> Result<(), Refusal> {
        let sp = self.te_null_scrap(sh)?;
        let e = Refusal::mem("StScrpRec");
        match a {
            None => self.mem.write_u16(sp, 0).map_err(&e),
            Some(a) => {
                self.mem.write_u16(sp, 1).map_err(&e)?;
                self.mem.write_bytes(sp + 2 + 8, &a.write()).map_err(&e)
            }
        }
    }
    /// A click or arrow key ends a pending null style (Executor `TEClick` / `doarrow`).
    pub(super) fn te_null_clear(&mut self, h: u32) {
        if let Ok(p) = self.te_ptr(h, "TextEdit") {
            if self.r16(p, T_TX_SIZE).ok() == Some(0xFFFF) { if let Ok(sh) = self.r32(p, T_TX_FONT) { let _ = self.te_null_set(sh, None); } }
        }
    }

    /// The style (index into `tab.styles`) the character at `at` has; past the end, the last character's.
    fn style_at(tab: &StyleTab, at: usize) -> usize {
        tab.runs.iter().rposition(|r| usize::from(r.0) <= at).map_or(0, |i| tab.runs[i].1.max(0) as usize).min(tab.styles.len().saturating_sub(1))
    }

    /// The attributes common to `start..stop`: per attribute, the first run's value and whether every run agrees.
    fn continuous(tab: &StyleTab, len: usize, start: usize, stop: usize) -> (Attrs, [bool; 4]) {
        let per = expand(&tab.runs, len);
        let at = |i: usize| tab.styles.get(per.get(i).copied().unwrap_or(0) as usize).map(|s| s.attrs).unwrap_or_default();
        let first = at(start.min(len.saturating_sub(1)));
        let mut same = [true; 4];
        for i in start..stop.min(len) {
            let a = at(i);
            same[0] &= a.font == first.font; same[1] &= a.face == first.face; same[2] &= a.size == first.size; same[3] &= a.color == first.color;
        }
        (first, same)
    }
}

/// `FUNCTION TEStyleNew(destRect, viewRect: Rect): TEHandle`: a record with the style structures of a one-style text (Executor
/// `C_TEStyleNew`): `lineHeight`, `fontAscent` and `txSize` are -1, the `TEStyleHandle` sits at `txFont`.
fn te_style_new<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (dest, view) = (read_rect(&m.mem, c.args.get(0))?, read_rect(&m.mem, c.args.get(1))?);
    let h = m.te_make(dest, view)?;
    let p = m.te_ptr(h, "TEStyleNew")?;
    let attrs = Attrs { font: m.r16(p, T_TX_FONT)? as i16, face: (m.r16(p, T_TX_FACE)? >> 8) as u8, size: m.r16(p, T_TX_SIZE)? as i16, color: [0; 3] };
    let (height, ascent) = (m.qd.font.y_size as i16, m.qd.font.baseline as i16 + 1);
    let alloc = |m: &mut Mac<U>, size: u32, what: &str| m.alloc_handle(size, what);
    let (sh, _) = alloc(m, STYLE_REC_HEAD + 8, "TEStyleRec")?;
    let (st, _) = alloc(m, ST_ELEMENT, "style table")?;
    let (lh, _) = alloc(m, 4, "line-height table")?;
    let (ns, np) = alloc(m, 8, "NullStRec")?;
    let (scrap, _) = alloc(m, 2 + SCRAP_ELEMENT, "null style scrap")?;
    let sp = m.deref_handle(sh, "TEStyleNew")?;
    let e = Refusal::mem("TEStyleRec");
    m.mem.write_u32(sp + 4, st).map_err(&e)?;
    m.mem.write_u32(sp + 8, lh).map_err(&e)?;
    m.mem.write_u32(sp + 16, ns).map_err(&e)?;
    m.mem.write_u32(np + 4, scrap).map_err(&e)?;
    let tab = StyleTab { runs: alloc::vec![(0, 0)], styles: alloc::vec![StyleEl { count: 1, height, ascent, attrs }] };
    m.te_style_write(sh, &tab, 0)?;
    m.te_style_line_heights(sh, 0)?;
    m.te_null_set(sh, None)?;
    // the scrap's one element carries the first style, as the ROM leaves it
    let scp = m.deref_handle(scrap, "TEStyleNew")?;
    m.mem.write_u16(scp + 2 + 4, height as u16).map_err(&e)?;
    m.mem.write_u16(scp + 2 + 6, ascent as u16).map_err(&e)?;
    m.mem.write_bytes(scp + 2 + 8, &attrs.write()).map_err(&e)?;
    m.w16(p, T_LINE_HEIGHT, 0xFFFF)?;
    m.w16(p, T_ASCENT, 0xFFFF)?;
    m.w16(p, T_TX_SIZE, 0xFFFF)?;
    m.mem.write_u32(p + T_TX_FONT, sh).map_err(&e)?;
    Ok(Done { result: h, log: format!("[textedit] TEStyleNew dest ({},{},{},{}) -> {h:#x} (TEStyleHandle {sh:#x}, one style: font {} face {} size {})", dest.top, dest.left, dest.bottom, dest.right, attrs.font, attrs.face, attrs.size) })
}

/// `FUNCTION TEGetOffset(pt: Point; hTE): INTEGER`: the character offset nearest `pt` (plain and styled records).
fn te_get_offset<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (pt, h) = (c.args.get(0), c.args.get(1));
    let r = m.te_load(h, "TEGetOffset")?;
    let at = m.te_find(&r, i32::from((pt >> 16) as u16 as i16), i32::from(pt as u16 as i16));
    Ok(Done { result: at as u32, log: format!("[textedit] TEGetOffset ({},{}) -> {at}", (pt >> 16) as u16 as i16, pt as u16 as i16) })
}

enum Ret { None, Word(u16), Long(u32), Bool(bool) }

fn run_selector<U: GuestUi>(m: &mut Mac<U>, sel: u16, sp: u32) -> Result<(Ret, String), Refusal> {
    let h = rd32(m, sp + 2)?; // hTE is the last argument of every routine but the handle setters' second
    match sel {
        1 => {
            let (redraw, new_at, mode) = (rd16(m, sp + 6)? >> 8 != 0, rd32(m, sp + 8)?, rd16(m, sp + 12)? as i16);
            let new = Attrs::read(m.mem.bytes(m.mem.span(new_at, TEXT_STYLE).map_err(Refusal::mem("TextStyle"))?).map_err(Refusal::mem("TextStyle"))?).unwrap_or_default();
            let r = m.te_load(h, "TESetStyle")?;
            let Some((sh, mut tab)) = r.style.clone() else { return Ok((Ret::None, String::from("TESetStyle on a plain record: nothing to style"))) };
            let (a, b) = r.sel;
            let (height, ascent) = (m.qd.font.y_size as i16, m.qd.font.baseline as i16 + 1);
            if a == b {
                let cur = m.te_null_attrs(sh).unwrap_or_else(|| tab.styles.get(Mac::<U>::style_at(&tab, a.saturating_sub(1))).map(|s| s.attrs).unwrap_or_default());
                m.te_null_set(sh, Some(adjust(&cur, &new, mode, None)))?;
                return Ok((Ret::None, format!("TESetStyle mode {mode:#x} at the insertion point: kept as the null style for the next typed text")));
            }
            let (cont, _) = Mac::<U>::continuous(&tab, r.text.len(), a, b);
            tab.map_range(r.text.len(), a, b, |_, o| (adjust(&o, &new, mode, Some(&cont)), height, ascent));
            m.te_style_write(sh, &tab, r.text.len())?;
            if redraw { m.te_draw(h, r.geo.view, "TESetStyle")?; }
            Ok((Ret::None, format!("TESetStyle mode {mode:#x} on {a}..{b} -> {} run(s)", tab.runs.len())))
        }
        3 => {
            let (ascent_p, lh_p, style_p, off) = (rd32(m, sp + 6)?, rd32(m, sp + 10)?, rd32(m, sp + 14)?, rd16(m, sp + 18)? as i16);
            let r = m.te_load(h, "TEGetStyle")?;
            let (attrs, lh, asc) = match &r.style {
                Some((_, tab)) => {
                    let el = tab.styles.get(Mac::<U>::style_at(tab, off.max(0) as usize)).copied().ok_or_else(|| Refusal(String::from("TEGetStyle: the record has no styles")))?;
                    (el.attrs, el.height, el.ascent)
                }
                None => (Attrs { font: r.geo.font as i16, face: r.geo.face, size: r.geo.size as i16, color: [0; 3] }, r.geo.line_height, r.geo.ascent),
            };
            m.mem.write_bytes(style_p, &attrs.write()).map_err(Refusal::mem("TextStyle"))?;
            m.mem.write_u16(lh_p, lh as u16).map_err(Refusal::mem("lineHeight"))?;
            m.mem.write_u16(ascent_p, asc as u16).map_err(Refusal::mem("fontAscent"))?;
            Ok((Ret::None, format!("TEGetStyle {off} -> font {} face {} size {}", attrs.font, attrs.face, attrs.size)))
        }
        4 => {
            let p = m.te_ptr(h, "TEGetStyleHandle")?;
            let sh = if m.r16(p, T_TX_SIZE)? == 0xFFFF { m.r32(p, T_TX_FONT)? } else { 0 };
            Ok((Ret::Long(sh), format!("TEGetStyleHandle -> {sh:#x}")))
        }
        5 => {
            let (new, h) = (rd32(m, sp + 6)?, h);
            let p = m.te_ptr(h, "TESetStyleHandle")?;
            if m.r16(p, T_TX_SIZE)? == 0xFFFF { m.mem.write_u32(p + T_TX_FONT, new).map_err(Refusal::mem("TERec"))?; }
            Ok((Ret::None, format!("TESetStyleHandle {new:#x}")))
        }
        8 => {
            let off = rd16(m, sp + 6)? as i16;
            let r = m.te_load(h, "TEGetPoint")?;
            let tab = draw::width_table(&m.qd.font);
            let g = &r.geo;
            let (v, hh) = layout::char_to_point(&r.text, &r.starts, off.max(0) as usize, i32::from(g.line_height), i32::from(g.dest.right) - i32::from(g.dest.left), g.just, &|c| tab[usize::from(c)]);
            let (pv, ph) = (i32::from(g.dest.top) + v + i32::from(g.ascent), i32::from(g.dest.left) + hh);
            Ok((Ret::Long(((pv as u32) << 16) | (ph as u32 & 0xFFFF)), format!("TEGetPoint {off} -> ({pv},{ph})")))
        }
        9 => {
            let (start, end) = (rd32(m, sp + 6)? as i32, rd32(m, sp + 10)? as i32);
            let r = m.te_load(h, "TEGetHeight")?;
            let n = r.starts.len().saturating_sub(1) as i32;
            let mut s = if start > 0 { start - 1 } else { 0 };
            let mut e = end.min(n);
            e = if e < 0 { 0 } else if e > 0 { e - 1 } else { 0 };
            if s > e { core::mem::swap(&mut s, &mut e); }
            let height = i32::from(r.geo.line_height) * (e - s + 1);
            Ok((Ret::Long(height as u32), format!("TEGetHeight lines {s}..={e} -> {height}")))
        }
        10 => {
            let (style_p, mode_p) = (rd32(m, sp + 6)?, rd32(m, sp + 10)?);
            let r = m.te_load(h, "TEContinuousStyle")?;
            let mode = rd16(m, mode_p)? as i16;
            let Some((_, tab)) = &r.style else { return Ok((Ret::Bool(true), String::from("TEContinuousStyle on a plain record: its one style is continuous"))) };
            let (a, b) = r.sel;
            let (first, same) = if a == b { (m.te_null_attrs(r.style.as_ref().map_or(0, |s| s.0)).unwrap_or_else(|| tab.styles.get(Mac::<U>::style_at(tab, a.saturating_sub(1))).map(|s| s.attrs).unwrap_or_default()), [true; 4]) } else { Mac::<U>::continuous(tab, r.text.len(), a, b) };
            let bits = [DO_FONT, DO_FACE, DO_SIZE, DO_COLOR];
            let out: i16 = bits.iter().zip(same).filter(|&(&bit, s)| mode & bit != 0 && s).fold(0, |acc, (&bit, _)| acc | bit);
            m.mem.write_u16(mode_p, out as u16).map_err(Refusal::mem("mode"))?;
            m.mem.write_bytes(style_p, &first.write()).map_err(Refusal::mem("TextStyle"))?;
            let all = bits.iter().zip(same).all(|(&bit, s)| mode & bit == 0 || s);
            Ok((Ret::Bool(all), format!("TEContinuousStyle {a}..{b} mode {mode:#x} -> {out:#x}, {all}")))
        }
        13 => {
            let (start, stop) = (rd32(m, sp + 10)? as i32, rd32(m, sp + 6)? as i32);
            let r = m.te_load(h, "TENumStyles")?;
            let n = match &r.style {
                Some((_, tab)) => 1 + tab.runs.iter().filter(|&&(s, _)| i32::from(s) > start && i32::from(s) < stop).count(),
                None => 1,
            };
            Ok((Ret::Long(n as u32), format!("TENumStyles {start}..{stop} -> {n}")))
        }
        14 => {
            // feature 0 is teFAutoScroll and 2 teFOutlineHilite (the two modelled: view.rs, draw.rs); action 0 clears, 1 sets, -1 tests; the answer is the PREVIOUS state
            let (action, feature) = (rd16(m, sp + 6)? as i16, rd16(m, sp + 8)? as i16);
            let (bit, what) = match feature { 0 => (view::AUTO_VIEW_BIT, "auto-scroll"), 2 => (view::OUTLINE_BIT, "outline highlight"), _ => (0, "") };
            if bit == 0 {
                if action == 1 { return Err(Refusal(format!("TEFeatureFlag: setting feature {feature} (text buffering, inline input, text services) is not modelled; it reads as clear"))); }
                return Ok((Ret::Word(0), format!("TEFeatureFlag feature {feature} action {action} -> clear")));
            }
            let was = m.te_flag(h, bit)?;
            if action == 1 || action == 0 { m.te_set_flag(h, bit, action == 1)?; }
            Ok((Ret::Word(u16::from(was)), format!("TEFeatureFlag {what} action {action} -> was {was}")))
        }
        other => {
            let name = SEL_FRAMES.iter().find(|f| f.0 == other).map_or("?", |f| f.1);
            Err(Refusal(format!("_TEDispatch selector {other} ({name}) is not implemented (its frame was popped; a result slot, if it has one, is untouched)")))
        }
    }
}

/// `_TEDispatch` ($A83D): the selector word is on top of the stack, the frame below it depends on the selector.
fn te_dispatch<U: GuestUi>(m: &mut Mac<U>, cpu: &mut Cpu, _: &mut Regs) -> Result<String, Refusal> {
    let sp = cpu.areg(7);
    let sel = m.mem.read_u16(sp).map_err(Refusal::mem("_TEDispatch selector"))?;
    let Some(&(_, _, params)) = SEL_FRAMES.iter().find(|f| f.0 == sel) else {
        return Err(Refusal(format!("_TEDispatch: selector {sel} is not a TextEdit selector (its frame is unknown, the stack is left as the program made it)")));
    };
    let outcome = run_selector(m, sel, sp);
    let new_sp = sp + 2 + params;
    if let Ok((ret, _)) = &outcome {
        match ret {
            Ret::None => Ok(()),
            Ret::Word(v) => m.mem.write_u16(new_sp, *v),
            Ret::Long(v) => m.mem.write_u32(new_sp, *v),
            Ret::Bool(b) => m.mem.write_u16(new_sp, if *b { 0x0100 } else { 0 }),
        }.map_err(Refusal::mem("_TEDispatch result"))?;
    }
    cpu.set_areg(7, new_sp);
    outcome.map(|(_, log)| format!("[textedit] {log}"))
}

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    m.register(0xA83E, &[Arg::L, Arg::L], Some(Arg::L), te_style_new::<U>)?;
    m.register(0xA83C, &[Arg::L, Arg::P], Some(Arg::W), te_get_offset::<U>)?;
    m.register_os(0xA83D, te_dispatch::<U>)
}
