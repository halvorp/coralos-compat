//! TextEdit editing: `TEKey`, `TEInsert`, `TEDelete`, the TE scrap (`TECut` / `TECopy` / `TEPaste`) and `TEClick`.
//!
//! Every edit is one replacement of the selection (Executor's `ROMlib_tedoitall`): the text changes, the insertion point lands
//! after the new text, the lines are recalculated and the view is redrawn. `TEKey` follows `teEdit.cpp`: the four arrow keys move
//! the insertion point (up and down by a line, to the nearest character), Backspace ($08) deletes the selection or the character
//! before the insertion point, Forward Delete ($7F) the one after, the keypad Enter ($03) is a carriage return, and anything else
//! is inserted. The TE scrap is the one in low memory (`TEScrpHandle` $AB4, `TEScrpLength` $AB0); `TECopy` does not touch the desk
//! scrap (that is `TEToScrap`, `TEDispatch`, not implemented yet).
use super::draw::width_table;
use super::layout::{self, CR};
use super::*;
use crate::lowmem::DOUBLE_TIME;

const LEFT: u8 = 0x1C;
const RIGHT: u8 = 0x1D;
const UP: u8 = 0x1E;
const DOWN: u8 = 0x1F;
const BACKSPACE: u8 = 0x08;
const FWD_DELETE: u8 = 0x7F;
const ENTER: u8 = 0x03;

impl<U: GuestUi> Mac<U> {
    /// Replace `text[start..stop]` with `ins`: the insertion point goes after it, the lines are recalculated, the view redrawn.
    fn te_replace(&mut self, h: u32, start: usize, stop: usize, ins: &[u8], what: &str) -> Result<(), Refusal> {
        let r = self.te_load(h, what)?;
        let (start, stop) = (start.min(r.text.len()), stop.min(r.text.len()).max(start.min(r.text.len())));
        let mut text = r.text.clone();
        text.splice(start..stop, ins.iter().copied());
        self.te_store_text(h, &text)?;
        let at = (start + ins.len()) as u16;
        self.w16(r.p, T_SEL_START, at)?;
        self.w16(r.p, T_SEL_END, at)?;
        if r.active { self.w16(r.p, T_CARET_STATE, CARET_VIS)?; }
        self.mem.write_u32(r.p + T_CARET_TIME, self.ev.ticks).map_err(Refusal::mem("TERec"))?;
        self.te_calc(h)?;
        self.te_draw(h, r.geo.view, what)
    }

    fn te_move_caret(&mut self, h: u32, a: usize, b: usize, what: &str) -> Result<(), Refusal> {
        let r = self.te_load(h, what)?;
        self.w16(r.p, T_SEL_START, a as u16)?;
        self.w16(r.p, T_SEL_END, b as u16)?;
        if r.active { self.w16(r.p, T_CARET_STATE, if a != b { HILITE_VIS } else { CARET_VIS })?; }
        self.mem.write_u32(r.p + T_CARET_TIME, self.ev.ticks).map_err(Refusal::mem("TERec"))?;
        self.te_draw(h, r.geo.view, what)
    }

    /// The character offset nearest the local point `(v, h)`.
    fn te_find(&self, r: &Rec, v: i32, h: i32) -> usize {
        let tab = width_table(&self.qd.font);
        let g = &r.geo;
        layout::find(&r.text, &r.starts, v - i32::from(g.dest.top), h - i32::from(g.dest.left), i32::from(g.line_height), i32::from(g.dest.right) - i32::from(g.dest.left), g.just, &|c| tab[usize::from(c)])
    }
}

fn arrow<U: GuestUi>(m: &mut Mac<U>, h: u32, key: u8) -> Result<(), Refusal> {
    let r = m.te_load(h, "TEKey")?;
    let (s, e) = r.sel;
    let new = match key {
        LEFT => s.saturating_sub(1),
        RIGHT => if e < r.text.len() { e + 1 } else { e },
        _ => {
            let tab = width_table(&m.qd.font);
            let g = &r.geo;
            let (v, hh) = layout::char_to_point(&r.text, &r.starts, s, i32::from(g.line_height), i32::from(g.dest.right) - i32::from(g.dest.left), g.just, &|c| tab[usize::from(c)]);
            let dv = i32::from(g.line_height) * if key == UP { -1 } else { 1 };
            m.te_find(&r, i32::from(g.dest.top) + v + dv + 1, i32::from(g.dest.left) + hh)
        }
    };
    m.te_move_caret(h, new, new, "TEKey")
}

/// `PROCEDURE TEKey(key: CHAR; hTE)`.
fn te_key<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (key, h) = ((c.args.get(0) & 0xFF) as u8, c.args.get(1));
    let r = m.te_load(h, "TEKey")?;
    let (s, e) = r.sel;
    match key {
        LEFT | RIGHT | UP | DOWN => arrow(m, h, key)?,
        BACKSPACE | FWD_DELETE => {
            let (a, b) = if s != e { (s, e) } else if key == BACKSPACE { (s.saturating_sub(1), s) } else { (s, (e + 1).min(r.text.len())) };
            m.te_replace(h, a, b, &[], "TEKey")?;
        }
        _ => m.te_replace(h, s, e, &[if key == ENTER { CR } else { key }], "TEKey")?,
    }
    Ok(Done { result: 0, log: format!("[textedit] TEKey {key:#04x}") })
}

/// `PROCEDURE TEInsert(text: Ptr; length: LONGINT; hTE)`: insert at the selection (replacing it).
fn te_insert<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (src, len, h) = (c.args.get(0), c.args.get(1) as i32, c.args.get(2));
    let n = usize::try_from(len).map_err(|_| Refusal(format!("TEInsert: negative length {len}")))?;
    let bytes = m.mem.bytes(m.mem.span(src, n as u32).map_err(Refusal::mem("TEInsert text"))?).map_err(Refusal::mem("TEInsert text"))?.to_vec();
    let r = m.te_load(h, "TEInsert")?;
    m.te_replace(h, r.sel.0, r.sel.1, &bytes, "TEInsert")?;
    Ok(Done { result: 0, log: format!("[textedit] TEInsert {n} bytes") })
}

/// `PROCEDURE TEDelete(hTE)`: remove the selection.
fn te_delete<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let h = c.args.get(0);
    let r = m.te_load(h, "TEDelete")?;
    m.te_replace(h, r.sel.0, r.sel.1, &[], "TEDelete")?;
    Ok(Done { result: 0, log: format!("[textedit] TEDelete {}..{}", r.sel.0, r.sel.1) })
}

fn copy_to_scrap<U: GuestUi>(m: &mut Mac<U>, h: u32, what: &str) -> Result<usize, Refusal> {
    let r = m.te_load(h, what)?;
    let bytes = r.text[r.sel.0..r.sel.1].to_vec();
    let sh = m.mem.read_u32(TE_SCRP_HANDLE).map_err(Refusal::mem("TEScrpHandle"))?;
    if m.mm.block(sh).is_none() { return Err(Refusal(format!("{what}: no TextEdit scrap (TEScrpHandle {sh:#x}): the program never called TEInit"))); }
    m.mm.set_handle_size(&mut m.mem, &mut m.heap, sh, bytes.len() as u32).map_err(|e| Refusal(format!("{what}: no memory for the TextEdit scrap (memFullErr {e})")))?;
    let at = m.mm.block(sh).map(|b| b.ptr).ok_or_else(|| Refusal(format!("{what}: the TextEdit scrap lost its block")))?;
    m.mem.write_bytes(at, &bytes).map_err(Refusal::mem("TEScrp"))?;
    m.mem.write_u16(TE_SCRP_LENGTH, bytes.len() as u16).map_err(Refusal::mem("TEScrpLength"))?;
    Ok(bytes.len())
}

/// `PROCEDURE TECopy(hTE)`: the selection into the TE scrap.
fn te_copy<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let n = copy_to_scrap(m, c.args.get(0), "TECopy")?;
    Ok(Done { result: 0, log: format!("[textedit] TECopy {n} bytes") })
}

/// `PROCEDURE TECut(hTE)`: `TECopy`, then `TEDelete`.
fn te_cut<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let h = c.args.get(0);
    let n = copy_to_scrap(m, h, "TECut")?;
    let r = m.te_load(h, "TECut")?;
    m.te_replace(h, r.sel.0, r.sel.1, &[], "TECut")?;
    Ok(Done { result: 0, log: format!("[textedit] TECut {n} bytes") })
}

/// `PROCEDURE TEPaste(hTE)`: the TE scrap replaces the selection.
fn te_paste<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let h = c.args.get(0);
    let sh = m.mem.read_u32(TE_SCRP_HANDLE).map_err(Refusal::mem("TEScrpHandle"))?;
    let len = usize::from(m.mem.read_u16(TE_SCRP_LENGTH).map_err(Refusal::mem("TEScrpLength"))?);
    let b = m.mm.block(sh).ok_or_else(|| Refusal(format!("TEPaste: no TextEdit scrap (TEScrpHandle {sh:#x}): the program never called TEInit")))?;
    let bytes = m.mem.bytes(m.mem.span(b.ptr, (len as u32).min(b.size)).map_err(Refusal::mem("TEScrp"))?).map_err(Refusal::mem("TEScrp"))?.to_vec();
    let r = m.te_load(h, "TEPaste")?;
    m.te_replace(h, r.sel.0, r.sel.1, &bytes, "TEPaste")?;
    Ok(Done { result: 0, log: format!("[textedit] TEPaste {} bytes", bytes.len()) })
}

/// `PROCEDURE TEClick(pt: Point; extend: BOOLEAN; hTE)`: a click puts the insertion point at the character nearest `pt`; a double
/// click (the same place within `DoubleTime` ticks) selects the word; with `extend` (Shift) the selection grows to `pt`.
/// The drag the ROM tracks inside this call is NOT tracked here (see the module note).
fn te_click<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (pt, extend, h) = (c.args.get(0), c.args.get(1) != 0, c.args.get(2));
    let (v, hh) = (i32::from((pt >> 16) as u16 as i16), i32::from(pt as u16 as i16));
    let r = m.te_load(h, "TEClick")?;
    let at = m.te_find(&r, v, hh);
    let now = m.ev.ticks;
    let dbl = match m.mem.read_u32(DOUBLE_TIME) { Ok(0) | Err(_) => 32, Ok(t) => t };
    let (last_t, last_at) = (m.r32(r.p, T_CLICK_TIME)?, usize::from(m.r16(r.p, T_CLICK_LOC)?));
    let (s, e) = r.sel;
    let (a, b, how) = if extend && (at <= s || at >= e) && !(s == e && s == at) {
        if at < s { (at, e, "extend left") } else { (s, at, "extend right") }
    } else if last_t != 0 && now.wrapping_sub(last_t) <= dbl && last_at == at {
        let (wa, wb) = layout::word_at(&r.text, at);
        (wa, wb, "word")
    } else { (at, at, "point") };
    m.mem.write_u32(r.p + T_CLICK_TIME, now).map_err(Refusal::mem("TERec"))?;
    m.w16(r.p, T_CLICK_LOC, at as u16)?;
    m.te_move_caret(h, a, b, "TEClick")?;
    Ok(Done { result: 0, log: format!("[textedit] TEClick ({v},{hh}) -> char {at}, selection ({a},{b}) {how} (drag selection is not tracked)") })
}

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    m.register(0xA9DC, &[Arg::W, Arg::P], None, te_key::<U>)?;
    m.register(0xA9DE, &[Arg::L, Arg::L, Arg::P], None, te_insert::<U>)?;
    m.register(0xA9D7, &[Arg::P], None, te_delete::<U>)?;
    m.register(0xA9D5, &[Arg::P], None, te_copy::<U>)?;
    m.register(0xA9D6, &[Arg::P], None, te_cut::<U>)?;
    m.register(0xA9DB, &[Arg::P], None, te_paste::<U>)?;
    m.register(0xA9D4, &[Arg::L, Arg::B, Arg::P], None, te_click::<U>)
}
