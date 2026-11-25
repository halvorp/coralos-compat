//! TextEdit (Inside Macintosh I "TextEdit", `_TEInit` ... `_TEScroll`, $A9CB-$A9DF): a REAL `TERec` in guest memory behind a
//! Memory Manager handle, so a program that reads `(**hTE).selStart` or walks `lineStarts` sees what the ROM would have written,
//! and the routines work on that record every time (the record is authoritative, the engine keeps nothing beside it).
//!
//! **Layout** (offsets from the PowerPC and 68k TextEdit headers as systemless' `textedit.rs` writes them, and Executor's MIT
//! `teInit.cpp`): `destRect` 0, `viewRect` 8, `selRect` 16, `lineHeight` 24, `fontAscent` 26, `selPoint` 28, `selStart` 32,
//! `selEnd` 34, `active` 36, `wordBreak` 38, `clikLoop` 42, `clickTime` 46, `clickLoc` 50, `caretTime` 52, `caretState` 56,
//! `just` 58, `teLength` 60, `hText` 62, `recalBack` 66, `recalLines` 68, `clikStuff` 70, `crOnly` 72, `txFont` 74, `txFace` 76,
//! `txMode` 78, `txSize` 80, `inPort` 82, `highHook` 86, `caretHook` 90, `nLines` 94, `lineStarts` 96 (`nLines + 1` words).
//!
//! **Drawing** is the program's own: the text, the selection highlight and the caret go through the engine's QuickDraw into the
//! record's `inPort` ([`draw`]), so a window the program draws its TE in is native content exactly like its other drawing. The
//! standard dialog edit-text item is NOT this: a dialog's text fields are projected as native text inputs ([`crate::translate`]).
//!
//! **Named limits** (each is logged where it applies, none is silent): drag selection inside `TEClick` is not tracked (the click
//! selects a point, a double click a word; Shift extends); `clikLoop`, `wordBreak`, `highHook` and `caretHook` are stored but not
//! called; text is drawn in the engine's single stand-in font whatever `txFont` says (the metrics it measures with are the metrics it
//! draws with). Styled TextEdit is in [`styled`] / [`styles`] (`TEStyleNew`, `TEGetOffset`, `_TEDispatch`: nine of its fifteen selectors; the
//! rest are refused by name there) and `TEAutoView` / `TESelView` / `TEPinScroll` in [`view`]. The text limit is 32767 bytes.
pub mod draw;
pub mod edit;
pub mod layout;
pub mod styled;
pub mod styles;
pub mod view;

use crate::lowmem::{CARET_TIME, TE_SCRP_HANDLE, TE_SCRP_LENGTH};
use crate::machine::{Call, Done, Mac};
use crate::pascal::{read_rect, Arg, Rect};
use crate::qd::port::{P_TX_FACE, P_TX_FONT, P_TX_MODE, P_TX_SIZE};
use crate::qd::target::current_port;
use crate::Refusal;
use alloc::format;
use alloc::vec::Vec;
use draw::{Geo, Mark};
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

pub const T_DEST: u32 = 0;
pub const T_VIEW: u32 = 8;
pub const T_SEL_RECT: u32 = 16;
pub const T_LINE_HEIGHT: u32 = 24;
pub const T_ASCENT: u32 = 26;
pub const T_SEL_START: u32 = 32;
pub const T_SEL_END: u32 = 34;
pub const T_ACTIVE: u32 = 36;
pub const T_CLIK_LOOP: u32 = 42;
pub const T_CLICK_TIME: u32 = 46;
pub const T_CLICK_LOC: u32 = 50;
pub const T_CARET_TIME: u32 = 52;
pub const T_CARET_STATE: u32 = 56;
pub const T_JUST: u32 = 58;
pub const T_LENGTH: u32 = 60;
pub const T_HTEXT: u32 = 62;
pub const T_CR_ONLY: u32 = 72;
pub const T_TX_FONT: u32 = 74;
pub const T_TX_FACE: u32 = 76;
pub const T_TX_MODE: u32 = 78;
pub const T_TX_SIZE: u32 = 80;
pub const T_IN_PORT: u32 = 82;
pub const T_N_LINES: u32 = 94;
pub const T_LINE_STARTS: u32 = 96;
/// `active` as the ROM stores it (a Boolean in the high byte of a word, `-256`), and `caretState`'s three values.
pub const ACTIVE: u16 = 0xFF00;
pub const CARET_INVIS: u16 = 0x00FF;
pub const CARET_VIS: u16 = 0xFFFF;
pub const HILITE_VIS: u16 = 0;
/// The longest text a record holds.
pub const MAX_TEXT: usize = 32767;
const DEFAULT_TICKS: u32 = 32;

/// What a record says right now, with its text and line table read out of guest memory.
pub struct Rec { pub h: u32, pub p: u32, pub geo: Geo, pub port: u32, pub sel: (usize, usize), pub active: bool, pub cr_only: i16, pub caret_state: u16, pub text: Vec<u8>, pub starts: Vec<u16>, /** a styled record: its TEStyleHandle and the decoded style data */ pub style: Option<(u32, styles::StyleTab)> }

impl<U: GuestUi> Mac<U> {
    /// The record's address (a live handle at least `lineStarts` big); anything else is not a `TEHandle`.
    fn te_ptr(&self, h: u32, what: &str) -> Result<u32, Refusal> {
        match self.mm.block(h) {
            Some(b) if b.size >= T_LINE_STARTS => Ok(b.ptr),
            Some(b) => Err(Refusal(format!("{what}: {h:#x} is a {}-byte handle, not a TextEdit record (96 bytes at least)", b.size))),
            None => Err(Refusal(format!("{what}: {h:#x} is not a live handle (a TEHandle was expected)"))),
        }
    }
    fn w16(&mut self, p: u32, off: u32, v: u16) -> Result<(), Refusal> { self.mem.write_u16(p + off, v).map_err(Refusal::mem("TERec")) }
    fn r16(&self, p: u32, off: u32) -> Result<u16, Refusal> { self.mem.read_u16(p + off).map_err(Refusal::mem("TERec")) }
    fn r32(&self, p: u32, off: u32) -> Result<u32, Refusal> { self.mem.read_u32(p + off).map_err(Refusal::mem("TERec")) }

    /// Read a record. The text is `teLength` bytes of `hText` (clamped to what the handle holds); the line table is the record's.
    pub fn te_load(&self, h: u32, what: &str) -> Result<Rec, Refusal> {
        let p = self.te_ptr(h, what)?;
        let ht = self.r32(p, T_HTEXT)?;
        let tb = self.mm.block(ht).ok_or_else(|| Refusal(format!("{what}: the record's hText {ht:#x} is not a live handle")))?;
        let len = usize::from(self.r16(p, T_LENGTH)?).min(tb.size as usize);
        let text = self.mem.bytes(self.mem.span(tb.ptr, len as u32).map_err(Refusal::mem("hText"))?).map_err(Refusal::mem("hText"))?.to_vec();
        let n = usize::from(self.r16(p, T_N_LINES)?);
        let room = ((self.mm.block(h).map_or(0, |b| b.size) - T_LINE_STARTS) / 2) as usize;
        let mut starts = Vec::with_capacity(n + 1);
        for i in 0..=n.min(room.saturating_sub(1)) { starts.push(self.r16(p, T_LINE_STARTS + 2 * i as u32)?); }
        if starts.is_empty() { starts.push(0); }
        let mut geo = Geo {
            dest: read_rect(&self.mem, p + T_DEST)?, view: read_rect(&self.mem, p + T_VIEW)?, line_height: self.r16(p, T_LINE_HEIGHT)? as i16, ascent: self.r16(p, T_ASCENT)? as i16,
            just: self.r16(p, T_JUST)? as i16, font: self.r16(p, T_TX_FONT)?, face: (self.r16(p, T_TX_FACE)? >> 8) as u8, mode: self.r16(p, T_TX_MODE)?, size: self.r16(p, T_TX_SIZE)?,
        };
        // a styled record (txSize -1) keeps its TEStyleHandle in the four bytes at txFont and its line metrics per line; the stand-in font
        // has one height, so every line is that tall (a named limit: a style's size does not change its glyphs or line)
        let style = if geo.size == 0xFFFF { Some(self.te_style_read(self.r32(p, T_TX_FONT)?, len)?) } else { None };
        if let Some((_, tab)) = &style {
            let first = tab.styles.first().map(|s| s.attrs).unwrap_or_default();
            geo.line_height = self.qd.font.y_size as i16;
            geo.ascent = self.qd.font.baseline as i16 + 1;
            (geo.font, geo.face, geo.size) = (first.font as u16, first.face, first.size as u16);
        }
        let sel = |off| usize::from(self.r16(p, off).unwrap_or(0)).min(len);
        let (a, b) = (sel(T_SEL_START), sel(T_SEL_END));
        Ok(Rec { h, p, geo, port: self.r32(p, T_IN_PORT)?, sel: (a.min(b), b), active: self.r16(p, T_ACTIVE)? != 0, cr_only: self.r16(p, T_CR_ONLY)? as i16, caret_state: self.r16(p, T_CARET_STATE)?, text, starts, style })
    }

    /// `TESetText`'s store: the text replaces the old, and a styled record's runs start over as one run of its first style.
    pub fn te_store_text(&mut self, h: u32, text: &[u8]) -> Result<(), Refusal> {
        self.te_write_text(h, text)?;
        let p = self.te_ptr(h, "TextEdit")?;
        if self.r16(p, T_TX_SIZE)? == 0xFFFF { self.te_style_reset(self.r32(p, T_TX_FONT)?, text.len())?; }
        Ok(())
    }

    /// Store `text` as the record's `hText` contents and `teLength`, leaving the style data to the caller.
    pub fn te_write_text(&mut self, h: u32, text: &[u8]) -> Result<(), Refusal> {
        if text.len() > MAX_TEXT { return Err(Refusal(format!("TextEdit: {} bytes of text is past the {MAX_TEXT}-byte limit of a text edit record", text.len()))); }
        let p = self.te_ptr(h, "TextEdit")?;
        let ht = self.r32(p, T_HTEXT)?;
        self.mm.set_handle_size(&mut self.mem, &mut self.heap, ht, text.len() as u32).map_err(|e| Refusal(format!("TextEdit: no memory for {} bytes of text (memFullErr {e})", text.len())))?;
        let at = self.mm.block(ht).ok_or_else(|| Refusal(format!("TextEdit: hText {ht:#x} lost its block")))?.ptr;
        self.mem.write_bytes(at, text).map_err(Refusal::mem("hText"))?;
        self.w16(p, T_LENGTH, text.len() as u16)
    }

    /// `TECalText`: break the text into lines and write `nLines` and `lineStarts` (growing the record to hold them).
    pub fn te_calc(&mut self, h: u32) -> Result<(), Refusal> {
        let r = self.te_load(h, "TECalText")?;
        let width = i32::from(r.geo.dest.right) - i32::from(r.geo.dest.left);
        let tab = draw::width_table(&self.qd.font);
        let starts = if width <= 0 { alloc::vec![0u16] } else { layout::line_starts(&r.text, width, r.cr_only, &|c| tab[usize::from(c)]) };
        let n = starts.len() - 1;
        let need = T_LINE_STARTS + 2 * (n as u32 + 4);
        if self.mm.block(h).map_or(0, |b| b.size) < need {
            self.mm.set_handle_size(&mut self.mem, &mut self.heap, h, need).map_err(|e| Refusal(format!("TECalText: no memory to hold {n} lines (memFullErr {e})")))?;
        }
        let p = self.te_ptr(h, "TECalText")?;
        self.w16(p, T_N_LINES, n as u16)?;
        for (i, s) in starts.iter().enumerate() { self.w16(p, T_LINE_STARTS + 2 * i as u32, *s)?; }
        self.w16(p, T_LINE_STARTS + 2 * (n as u32 + 1), 0)?; // the ROM's spare word after the table
        if self.r16(p, T_TX_SIZE)? == 0xFFFF { self.te_style_line_heights(self.r32(p, T_TX_FONT)?, n)?; }
        Ok(())
    }

    /// Draw `area` of the record (LOCAL), with its highlight or caret when it is active.
    pub fn te_draw(&mut self, h: u32, area: Rect, what: &str) -> Result<(), Refusal> {
        let r = self.te_load(h, what)?;
        if r.port == 0 { return Ok(()); }
        let mark = match (r.active, r.sel) {
            (false, (a, b)) if a != b && self.te_flag(h, view::OUTLINE_BIT)? => Mark::Outline(a, b),
            (false, _) => Mark::None,
            (true, (a, b)) if a != b => Mark::Range(a, b),
            (true, (a, _)) if r.caret_state == CARET_VIS => Mark::Caret(a),
            _ => Mark::None,
        };
        draw::paint(self, r.port, &r.geo, &r.text, &r.starts, area, mark, r.style.as_ref().map(|s| &s.1))?;
        // Use the actually-painted box (area ∩ viewRect ∩ port clipRgn) so horizontal scroll slack in viewRect
        // does not widen the native edit field past its edit-text bevel into neighbouring popup controls.
        let mut paint_box = crate::qd::port::raster(area).intersect(&crate::qd::port::raster(r.geo.view));
        if let Ok(ch) = self.mem.read_u32(r.port.wrapping_add(crate::qd::port::P_CLIP)) {
            if ch != 0 {
                if let Ok(cr) = self.read_region(ch) {
                    paint_box = paint_box.intersect(&cr.bounds());
                }
            }
        }
        let fit = Rect { top: paint_box.y0 as i16, left: paint_box.x0 as i16, bottom: paint_box.y1 as i16, right: paint_box.x1 as i16 };
        self.sync_te_field(h, r.port, fit, &r.text)
    }

    /// Project a live single-line editable `TEHandle` (marked via `TEFeatureFlag(teFOutlineHilite)` or `TEActivate`)
    /// as a native editable `ControlKind::Field` at its window-pixel edit box (`viewRect` expanded by the standard 3 px
    /// Mac edit-text frame so the kit field cleanly replaces the guest's white box and bevel in light and dark mode).
    pub(crate) fn sync_te_field(&mut self, h: u32, port: u32, view: Rect, text: &[u8]) -> Result<(), Refusal> {
        let Some(win) = self.windows.get(&port) else {
            return Ok(());
        };
        let role = win.def_role.map(|(r, _)| r).or_else(|| self.window_role(win.proc_id).ok().map(|(r, _)| r));
        if !self.tr.te_editable.contains(&h) || role != Some(heirloom_ui_abi::WindowRole::Dialog) {
            return Ok(());
        }
        let vh = i32::from(view.bottom) - i32::from(view.top);
        let vw = i32::from(view.right) - i32::from(view.left);
        if !(10..=26).contains(&vh) || vw < 24 {
            return Ok(());
        }
        let pr = read_rect(&self.mem, port + crate::qd::port::P_RECT).unwrap_or(Rect { top: 0, left: 0, bottom: 0, right: 0 });
        let win_r = Rect {
            top: view.top.wrapping_sub(pr.top).saturating_sub(3).max(0),
            left: view.left.wrapping_sub(pr.left).saturating_sub(3).max(0),
            bottom: view.bottom.wrapping_sub(pr.top).saturating_add(3),
            right: view.right.wrapping_sub(pr.left).saturating_add(3),
        };
        self.tr.te_boxes.insert(h, (port, win_r));
        let s: alloc::string::String = text.iter().map(|&b| crate::macroman::to_char(b)).collect();
        let model = heirloom_ui_abi::ControlModel {
            kind: heirloom_ui_abi::ControlKind::Field { text: s, max_len: 255, editable: true, integer: false },
            label: alloc::string::String::new(),
            rect: crate::translate::guest_rect(win_r),
            enabled: true,
            token: heirloom_ui_abi::CallbackToken(u64::from(h)),
        };
        self.sync_projection(port, crate::translate::Obj::TextEdit(h), Some(model))
    }

    /// Route native host intents (`Press`, `Drag(Text)`, `CommitText`) on a projected single-line `TEHandle` back into
    /// guest TextEdit memory and focus.
    pub(crate) fn te_intent(&mut self, h: u32, intent: heirloom_ui_abi::Intent) -> Result<(), Refusal> {
        use heirloom_ui_abi::{ControlValue, Intent};
        let Some(&(win, win_r)) = self.tr.te_boxes.get(&h) else {
            return Err(Refusal(format!("Intent: TextEdit {h:#x} is not projected")));
        };
        let is_commit = matches!(intent, Intent::CommitText(_));
        match intent {
            Intent::Press => {
                if let Some(w) = self.windows.get(&win).map(|w| w.rect) {
                    let gv = w.top.wrapping_add(((i32::from(win_r.top) + i32::from(win_r.bottom)) / 2) as i16);
                    let gh = w.left.wrapping_add(((i32::from(win_r.left) + i32::from(win_r.right)) / 2) as i16);
                    self.mouse_event(crate::events::MOUSE_DOWN, gv, gh, 0)?;
                    self.mouse_event(crate::events::MOUSE_UP, gv, gh, 0)?;
                }
                Ok(())
            }
            Intent::Drag(ControlValue::Text(t)) | Intent::CommitText(t) => {
                let bytes: Vec<u8> = t.chars().map(|c| crate::macroman::from_char(c).unwrap_or(b'?')).collect();
                self.te_store_text(h, &bytes)?;
                self.te_calc(h)?;
                self.adopt_native_state(crate::translate::Obj::TextEdit(h), &ControlValue::Text(t));
                if let Ok(r) = self.te_load(h, "te_intent") {
                    let _ = self.sync_te_field(h, r.port, r.geo.view, &r.text);
                }
                if is_commit {
                    self.post(crate::events::KEY_DOWN, (0x24u32 << 8) | 0x0D, 0, 0);
                }
                Ok(())
            }
            other => Err(Refusal(format!("Intent {other:?} has no meaning for a TextEdit field"))),
        }
    }

    pub(crate) fn caret_ticks(&self) -> u32 { match self.mem.read_u32(CARET_TIME) { Ok(0) | Err(_) => DEFAULT_TICKS, Ok(t) => t } }
}

fn done(c: &Call, extra: &str) -> Result<Done, Refusal> { Ok(Done { result: 0, log: format!("[textedit] {}{extra}", c.name) }) }

/// `TEInit`: the TextEdit scrap handle and length ($AB4, $AB0) exist from here on.
fn te_init<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let h = m.mm.new_handle(&mut m.mem, &mut m.heap, 0, false).map_err(|e| Refusal(format!("TEInit: no memory for the TextEdit scrap (memFullErr {e})")))?;
    m.mem.write_u32(TE_SCRP_HANDLE, h).map_err(Refusal::mem("TEScrpHandle"))?;
    m.mem.write_u16(TE_SCRP_LENGTH, 0).map_err(Refusal::mem("TEScrpLength"))?;
    done(c, &format!(" (TEScrpHandle {h:#x})"))
}

/// `FUNCTION TENew(destRect, viewRect: Rect): TEHandle`. The record takes the current port's text characteristics and the
/// engine font's line metrics; `crOnly` is 1 (wrap), `just` is `teFlushDefault`, the caret is invisible, `nLines` 0.
fn te_new<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (dest, view) = (read_rect(&m.mem, c.args.get(0))?, read_rect(&m.mem, c.args.get(1))?);
    let h = m.te_make(dest, view)?;
    let (lh, ascent) = (m.qd.font.y_size as i16, m.qd.font.baseline as i16 + 1);
    Ok(Done { result: h, log: format!("[textedit] TENew dest ({},{},{},{}) view ({},{},{},{}) -> {h:#x} (lineHeight {lh}, ascent {ascent})", dest.top, dest.left, dest.bottom, dest.right, view.top, view.left, view.bottom, view.right) })
}

impl<U: GuestUi> Mac<U> {
    /// Make a plain record (see [`te_new`]); `TEStyleNew` makes this and then the style data.
    pub(super) fn te_make(&mut self, dest: Rect, view: Rect) -> Result<u32, Refusal> {
        let port = current_port(self)?;
        let rd = |m: &Self, off| m.mem.read_u16(port.wrapping_add(off)).map_err(Refusal::mem("GrafPort"));
        let (font, face, mode, size) = (rd(self, P_TX_FONT)?, self.mem.read_u8(port.wrapping_add(P_TX_FACE)).map_err(Refusal::mem("txFace"))?, rd(self, P_TX_MODE)?, rd(self, P_TX_SIZE)?);
        let ascent = self.qd.font.baseline as i16 + 1;
        let lh = self.qd.font.y_size as i16;
        let h = self.mm.new_handle(&mut self.mem, &mut self.heap, T_LINE_STARTS + 8, true).map_err(|e| Refusal(format!("TENew: no memory for the record (memFullErr {e})")))?;
        let ht = self.mm.new_handle(&mut self.mem, &mut self.heap, 0, false).map_err(|e| Refusal(format!("TENew: no memory for the text (memFullErr {e})")))?;
        let p = self.mm.block(h).ok_or_else(|| Refusal(format!("TENew: {h:#x} lost its block")))?.ptr;
        for (off, r) in [(T_DEST, dest), (T_VIEW, view)] {
            for (i, v) in [r.top, r.left, r.bottom, r.right].into_iter().enumerate() { self.w16(p, off + 2 * i as u32, v as u16)?; }
        }
        self.w16(p, T_LINE_HEIGHT, lh as u16)?;
        self.w16(p, T_ASCENT, ascent as u16)?;
        self.w16(p, T_CARET_STATE, CARET_INVIS)?;
        self.w16(p, T_CR_ONLY, 1)?;
        self.mem.write_u32(p + T_HTEXT, ht).map_err(Refusal::mem("TERec"))?;
        self.mem.write_u32(p + T_IN_PORT, port).map_err(Refusal::mem("TERec"))?;
        self.w16(p, T_TX_FONT, font)?;
        self.mem.write_u8(p + T_TX_FACE, face).map_err(Refusal::mem("TERec"))?;
        self.w16(p, T_TX_MODE, mode)?;
        self.w16(p, T_TX_SIZE, size)?;
        // the hidden record behind recalBack (view.rs): hooks and the auto-scroll flag
        let (hh, _) = self.alloc_handle(view::HIDDEN_SIZE, "TextEdit hidden record")?;
        self.mem.write_u32(p + view::T_HIDDEN, hh).map_err(Refusal::mem("TERec"))?;
        Ok(h)
    }
}

/// `PROCEDURE TEDispose(hTE)`: the record and its text go.
fn te_dispose<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let h = c.args.get(0);
    m.tr.te_editable.remove(&h);
    if let Some((win, _)) = m.tr.te_boxes.remove(&h) {
        let _ = m.sync_projection(win, crate::translate::Obj::TextEdit(h), None);
    }
    let p = m.te_ptr(h, "TEDispose")?;
    let ht = m.r32(p, T_HTEXT)?;
    let hid = m.r32(p, view::T_HIDDEN)?;
    if m.mm.block(hid).is_some() { let _ = m.dispose_handle(hid); }
    if m.mm.block(ht).is_some() { m.dispose_handle(ht).map_err(|e| Refusal(format!("TEDispose: hText {ht:#x}: memory manager error {e}")))?; }
    m.dispose_handle(h).map_err(|e| Refusal(format!("TEDispose {h:#x}: memory manager error {e}")))?;
    done(c, &format!(" {h:#x}"))
}

/// `PROCEDURE TESetText(text: Ptr; length: LONGINT; hTE)`: replaces the text and recalculates `lineStarts`
/// (Inside Macintosh: Text, p. 2-74) so a subsequent `TEUpdate` immediately draws the new lines.
fn te_set_text<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (src, len, h) = (c.args.get(0), c.args.get(1) as i32, c.args.get(2));
    let n = usize::try_from(len).map_err(|_| Refusal(format!("TESetText: negative length {len}")))?;
    let bytes = m.mem.bytes(m.mem.span(src, n as u32).map_err(Refusal::mem("TESetText text"))?).map_err(Refusal::mem("TESetText text"))?.to_vec();
    m.te_store_text(h, &bytes)?;
    m.te_calc(h)?;
    let head: alloc::string::String = bytes.iter().take(80).map(|&b| if b == b'\r' { '|' } else { crate::macroman::to_char(b) }).collect();
    done(c, &format!(" {n} bytes {head:?}"))
}

/// `FUNCTION TEGetText(hTE): CharsHandle`: the record's own text handle.
fn te_get_text<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let p = m.te_ptr(c.args.get(0), "TEGetText")?;
    let ht = m.r32(p, T_HTEXT)?;
    Ok(Done { result: ht, log: format!("[textedit] TEGetText -> {ht:#x}") })
}

/// `PROCEDURE TECalText(hTE)`: `teLength` is re-read from `hText` (programs poke the handle), then the lines are broken.
fn te_cal_text<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let h = c.args.get(0);
    let p = m.te_ptr(h, "TECalText")?;
    let ht = m.r32(p, T_HTEXT)?;
    let size = m.mm.block(ht).ok_or_else(|| Refusal(format!("TECalText: hText {ht:#x} is not a live handle")))?.size;
    m.w16(p, T_LENGTH, size.min(MAX_TEXT as u32) as u16)?;
    m.te_calc(h)?;
    let n = m.r16(p, T_N_LINES)?;
    done(c, &format!(" -> {n} lines"))
}

/// `PROCEDURE TEUpdate(rUpdate: Rect; hTE)`: erase and draw the part of the view inside `rUpdate`.
fn te_update<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (r, h) = (read_rect(&m.mem, c.args.get(0))?, c.args.get(1));
    m.te_draw(h, r, "TEUpdate")?;
    done(c, "")
}

/// `PROCEDURE TESetSelect(selStart, selEnd: LONGINT; hTE)`: clamped to the text; the highlight follows if the record is active.
fn te_set_select<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (a, b, h) = (c.args.get(0) as i32, c.args.get(1) as i32, c.args.get(2));
    let r = m.te_load(h, "TESetSelect")?;
    let len = r.text.len() as i32;
    let stop = b.min(len);
    let start = a.max(0).min(stop);
    if (start as usize, stop as usize) != r.sel {
        let p = r.p;
        m.w16(p, T_SEL_START, start as u16)?;
        m.w16(p, T_SEL_END, stop as u16)?;
        if r.active { m.w16(p, T_CARET_STATE, if start != stop { HILITE_VIS } else { CARET_INVIS })?; }
        m.te_draw(h, r.geo.view, "TESetSelect")?;
    }
    done(c, &format!(" ({start},{stop})"))
}

fn set_active<U: GuestUi>(m: &mut Mac<U>, c: &Call, on: bool) -> Result<Done, Refusal> {
    let h = c.args.get(0);
    if on { m.tr.te_editable.insert(h); }
    let p = m.te_ptr(h, c.name)?;
    if (m.r16(p, T_ACTIVE)? != 0) != on {
        m.w16(p, T_ACTIVE, if on { ACTIVE } else { 0 })?;
        if !on { m.w16(p, T_CARET_STATE, CARET_INVIS)?; }
        let view = read_rect(&m.mem, p + T_VIEW)?;
        m.te_draw(h, view, c.name)?;
    }
    done(c, "")
}
fn te_activate<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { set_active(m, c, true) }
fn te_deactivate<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { set_active(m, c, false) }

/// `PROCEDURE TEIdle(hTE)`: blink the caret once `CaretTime` ticks have passed.
fn te_idle<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let h = c.args.get(0);
    let r = m.te_load(h, "TEIdle")?;
    let now = m.ev.ticks;
    let last = m.r32(r.p, T_CARET_TIME)?;
    if r.active && r.sel.0 == r.sel.1 && r.port != 0 && now.wrapping_sub(last) > m.caret_ticks() {
        let next = if r.caret_state == CARET_VIS { CARET_INVIS } else { CARET_VIS };
        m.w16(r.p, T_CARET_STATE, next)?;
        m.mem.write_u32(r.p + T_CARET_TIME, now).map_err(Refusal::mem("TERec"))?;
        draw::toggle_caret(m, r.port, &r.geo, &r.text, &r.starts, r.sel.0)?;
        return done(c, &format!(" (caret {})", if next == CARET_VIS { "on" } else { "off" }));
    }
    done(c, " (no blink due)")
}

/// `PROCEDURE TESetAlignment(just: INTEGER; hTE)` (`TESetJust` in the 1985 manual): recalculate and redraw.
fn te_set_alignment<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (j, h) = (c.args.get(0) as u16, c.args.get(1));
    let p = m.te_ptr(h, "TESetAlignment")?;
    m.w16(p, T_JUST, j)?;
    m.te_calc(h)?;
    let view = read_rect(&m.mem, p + T_VIEW)?;
    m.te_draw(h, view, "TESetAlignment")?;
    done(c, &format!(" {}", j as i16))
}

/// `PROCEDURE TEScroll(dh, dv: INTEGER; hTE)`: the text moves by (dh, dv) inside the view.
fn te_scroll<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (dh, dv, h) = (c.args.get(0) as i16, c.args.get(1) as i16, c.args.get(2));
    let p = m.te_ptr(h, "TEScroll")?;
    let d = read_rect(&m.mem, p + T_DEST)?;
    for (i, v) in [d.top.wrapping_add(dv), d.left.wrapping_add(dh), d.bottom.wrapping_add(dv), d.right.wrapping_add(dh)].into_iter().enumerate() { m.w16(p, T_DEST + 2 * i as u32, v as u16)?; }
    let view = read_rect(&m.mem, p + T_VIEW)?;
    m.te_draw(h, view, "TEScroll")?;
    done(c, &format!(" ({dh},{dv})"))
}

/// `PROCEDURE TETextBox(text: Ptr; length: LONGINT; box: Rect; just: INTEGER)`: erase `box` and draw the text wrapped into it
/// in the current port's font, flush by `just`; no record is kept.
fn te_text_box<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (src, len, boxr, just) = (c.args.get(0), c.args.get(1) as i32, read_rect(&m.mem, c.args.get(2))?, c.args.get(3) as i16);
    let n = usize::try_from(len).map_err(|_| Refusal(format!("TETextBox: negative length {len}")))?;
    let text = m.mem.bytes(m.mem.span(src, n as u32).map_err(Refusal::mem("TETextBox text"))?).map_err(Refusal::mem("TETextBox text"))?.to_vec();
    let port = current_port(m)?;
    let rd = |m: &Mac<U>, off| m.mem.read_u16(port.wrapping_add(off)).map_err(Refusal::mem("GrafPort"));
    let geo = Geo {
        dest: boxr, view: boxr, line_height: m.qd.font.y_size as i16, ascent: m.qd.font.baseline as i16 + 1, just, font: rd(m, P_TX_FONT)?,
        face: m.mem.read_u8(port.wrapping_add(P_TX_FACE)).map_err(Refusal::mem("txFace"))?, mode: rd(m, P_TX_MODE)?, size: rd(m, P_TX_SIZE)?,
    };
    let tab = draw::width_table(&m.qd.font);
    let width = i32::from(boxr.right) - i32::from(boxr.left);
    let starts = if width <= 0 { alloc::vec![0u16] } else { layout::line_starts(&text, width, 1, &|ch| tab[usize::from(ch)]) };
    draw::paint(m, port, &geo, &text, &starts, boxr, Mark::None, None)?;
    done(c, &format!(" {n} bytes into ({},{},{},{})", boxr.top, boxr.left, boxr.bottom, boxr.right))
}

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    m.register(0xA9CC, &[], None, te_init::<U>)?;
    m.register(0xA9D2, &[Arg::L, Arg::L], Some(Arg::L), te_new::<U>)?;
    m.register(0xA9CD, &[Arg::P], None, te_dispose::<U>)?;
    m.register(0xA9CF, &[Arg::L, Arg::L, Arg::P], None, te_set_text::<U>)?;
    m.register(0xA9CB, &[Arg::P], Some(Arg::L), te_get_text::<U>)?;
    m.register(0xA9D0, &[Arg::P], None, te_cal_text::<U>)?;
    m.register(0xA9D3, &[Arg::L, Arg::P], None, te_update::<U>)?;
    m.register(0xA9D1, &[Arg::L, Arg::L, Arg::P], None, te_set_select::<U>)?;
    m.register(0xA9D8, &[Arg::P], None, te_activate::<U>)?;
    m.register(0xA9D9, &[Arg::P], None, te_deactivate::<U>)?;
    m.register(0xA9DA, &[Arg::P], None, te_idle::<U>)?;
    m.register(0xA9DF, &[Arg::W, Arg::P], None, te_set_alignment::<U>)?;
    m.register(0xA9DD, &[Arg::W, Arg::W, Arg::P], None, te_scroll::<U>)?;
    m.register(0xA9CE, &[Arg::L, Arg::L, Arg::L, Arg::W], None, te_text_box::<U>)?;
    edit::register(m)?;
    styled::register(m)?;
    view::register(m)
}
