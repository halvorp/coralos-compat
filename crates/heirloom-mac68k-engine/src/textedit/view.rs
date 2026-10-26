//! Keeping the selection in view: `TEAutoView` ($A813), `TESelView` ($A811), `TEPinScroll` ($A812) (Inside Macintosh IV p. 57;
//! Executor's MIT `teIMIV.cpp`), and the auto-scroll feature flag of `TEFeatureFlag`.
//!
//! The flag lives where the ROM keeps it: a 20-byte hidden record behind the `recalBack` field of the `TERec` (four hook words, then a
//! long of flags; bit 0 = auto-scroll). `TENew` makes it, `TEDispose` frees it. Auto-scroll only changes what `TESelView` does (as in
//! Executor, which uses it from the click loop; the editing routines do not scroll by themselves): with the bit set, `TESelView`
//! scrolls the minimum that brings the selection into the view rectangle.
use super::layout;
use alloc::string::String;
use super::*;

/// `recalBack` holds the hidden record's handle.
pub const T_HIDDEN: u32 = 66;
pub const HIDDEN_SIZE: u32 = 20;
pub const HIDDEN_FLAGS: u32 = 16;
pub const AUTO_VIEW_BIT: u32 = 1;
/// `teFOutlineHilite`: a deactivated record shows its selection as an outline instead of not at all.
pub const OUTLINE_BIT: u32 = 2;

impl<U: GuestUi> Mac<U> {
    fn te_hidden(&self, h: u32) -> Result<u32, Refusal> {
        let p = self.te_ptr(h, "TextEdit")?;
        let hh = self.r32(p, T_HIDDEN)?;
        self.mm.block(hh).map(|b| b.ptr).ok_or_else(|| Refusal(format!("TextEdit: the record {h:#x} has no hidden record (recalBack {hh:#x} is not a live handle)")))
    }
    pub(super) fn te_auto_view(&self, h: u32) -> Result<bool, Refusal> { self.te_flag(h, AUTO_VIEW_BIT) }
    pub(super) fn te_set_auto_view(&mut self, h: u32, on: bool) -> Result<(), Refusal> { self.te_set_flag(h, AUTO_VIEW_BIT, on) }
    pub(super) fn te_flag(&self, h: u32, bit: u32) -> Result<bool, Refusal> {
        Ok(self.mem.read_u32(self.te_hidden(h)? + HIDDEN_FLAGS).map_err(Refusal::mem("TEHidden"))? & bit != 0)
    }
    pub(super) fn te_set_flag(&mut self, h: u32, bit: u32, on: bool) -> Result<(), Refusal> {
        let at = self.te_hidden(h)? + HIDDEN_FLAGS;
        let f = self.mem.read_u32(at).map_err(Refusal::mem("TEHidden"))?;
        self.mem.write_u32(at, if on { f | bit } else { f & !bit }).map_err(Refusal::mem("TEHidden"))
    }

    /// `TEPinScroll`: scroll by (`dh`, `dv`) but never past the text: `destRect.top` stays at or above `viewRect.top`, the end of the
    /// text at or below `viewRect.bottom` (and the same horizontally against `destRect.left`).
    pub(super) fn te_pin_scroll(&mut self, h: u32, dh: i16, dv: i16) -> Result<(), Refusal> {
        let r = self.te_load(h, "TEPinScroll")?;
        let (view, dest) = (r.geo.view, r.geo.dest);
        let mut dv = i32::from(dv);
        if dv > 0 {
            dv = dv.min(i32::from(view.top) - i32::from(dest.top));
        } else {
            let tab = draw::width_table(&self.qd.font);
            let g = &r.geo;
            let (v, _) = layout::char_to_point(&r.text, &r.starts, r.text.len(), i32::from(g.line_height), i32::from(dest.right) - i32::from(dest.left), g.just, &|c| tab[usize::from(c)]);
            let bottom = i32::from(dest.top) + v + i32::from(g.line_height);
            dv = dv.max(i32::from(view.bottom) - bottom).min(0);
        }
        let max_h = i32::from(view.left) - i32::from(dest.left);
        let mut dh = i32::from(dh);
        dh = if dh > 0 { if max_h > 0 { dh.min(max_h) } else { 0 } } else if max_h < 0 { dh.max(max_h) } else { 0 };
        if dh == 0 && dv == 0 { return Ok(()); }
        let p = r.p;
        for (i, v) in [dest.top.wrapping_add(dv as i16), dest.left.wrapping_add(dh as i16), dest.bottom.wrapping_add(dv as i16), dest.right.wrapping_add(dh as i16)].into_iter().enumerate() { self.w16(p, T_DEST + 2 * i as u32, v as u16)?; }
        self.te_draw(h, view, "TEPinScroll")
    }
}

fn delta(sel_start: i32, sel_stop: i32, view_start: i32, view_stop: i32) -> i32 {
    if sel_start < view_start { view_start - sel_start }
    else if sel_stop > view_stop { if sel_stop - sel_start > view_stop - view_start { view_start - sel_start } else { view_stop - sel_stop } }
    else { 0 }
}

/// `PROCEDURE TEAutoView(fAuto: BOOLEAN; hTE)`.
fn te_auto_view<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (on, h) = (c.args.get(0) as u8 != 0, c.args.get(1));
    m.te_set_auto_view(h, on)?;
    Ok(Done { result: 0, log: format!("[textedit] TEAutoView {on}") })
}

/// `PROCEDURE TESelView(hTE)`: with auto-scroll on, bring the selection into the view (the least scroll).
fn te_sel_view<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let h = c.args.get(0);
    if !m.te_auto_view(h)? { return Ok(Done { result: 0, log: String::from("[textedit] TESelView (auto-scroll is off: nothing to do)") }); }
    let r = m.te_load(h, "TESelView")?;
    let tab = draw::width_table(&m.qd.font);
    let g = &r.geo;
    let width = i32::from(g.dest.right) - i32::from(g.dest.left);
    let pt = |sel| layout::char_to_point(&r.text, &r.starts, sel, i32::from(g.line_height), width, g.just, &|c| tab[usize::from(c)]);
    let ((sv, sh), (ev, eh)) = (pt(r.sel.0), pt(r.sel.1));
    let (top, left) = (i32::from(g.dest.top), i32::from(g.dest.left));
    let dv = delta(top + sv, top + ev + i32::from(g.line_height), i32::from(g.view.top), i32::from(g.view.bottom));
    let dh = delta(left + sh, left + eh, i32::from(g.view.left), i32::from(g.view.right));
    m.te_pin_scroll(h, dh as i16, dv as i16)?;
    Ok(Done { result: 0, log: format!("[textedit] TESelView -> scroll ({dh},{dv}) requested") })
}

/// `PROCEDURE TEPinScroll(dh, dv: INTEGER; hTE)`.
fn te_pin_scroll<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (dh, dv, h) = (c.args.get(0) as i16, c.args.get(1) as i16, c.args.get(2));
    m.te_pin_scroll(h, dh, dv)?;
    Ok(Done { result: 0, log: format!("[textedit] TEPinScroll ({dh},{dv})") })
}

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    m.register(0xA813, &[Arg::B, Arg::P], None, te_auto_view::<U>)?;
    m.register(0xA811, &[Arg::P], None, te_sel_view::<U>)?;
    m.register(0xA812, &[Arg::W, Arg::W, Arg::P], None, te_pin_scroll::<U>)
}
