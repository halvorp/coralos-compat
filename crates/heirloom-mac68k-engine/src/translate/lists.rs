//! The List Manager (`_Pack0`, $A9E7) as a toolkit table (ruling 17, piece 3): a standard-text-LDEF list is shown as a native
//! `List`, a native row choice becomes the exact input a Mac saw (a `mouseDown` in the list, the program's `LClick`), and a custom
//! LDEF stays a content plane (no role is inferred from what its own drawing looks like).
//!
//! **Real structures.** A `ListHandle` is a Memory Manager handle to a 88-byte `ListRec` (Inside Macintosh IV "List Manager"; the
//! offsets are those systemless' `LNew` writes, used as the oracle): `rView` 0, `port` 8, `indent` 12, `cellSize` 16, `visible` 20,
//! `vScroll` 28, `hScroll` 32, `selFlags` 36, `lActive` 37, `listFlags` 39, `clikTime` 40, `clikLoc` 44, `mouseLoc` 48,
//! `lClickLoop` 52, `lastClick` 56, `refCon` 60, `listDefProc` 64, `userHandle` 68, `dataBounds` 72, `cells` 80, `maxIndex` 84.
//! A Rect is top, left, bottom, right; a Point/Cell is v (row), h (column). Geometry, bounds and the scroll origin (`visible`) are
//! read from the record every time, so a program that pokes them is believed. The CELL CONTENTS and the selection flags are held by
//! the engine (the packed offset array behind `cells` is not modelled: `cells` is a stand-in handle a program must not read).
//! Scroll bars: `LNew` with scrollH / scrollV TRUE makes real, HIDDEN `ControlRecord`s in `hScroll` / `vScroll` (a program sets their refCon and ranges);
//! the native list draws and moves its own scroll bars and tells the record through `visible`.
//!
//! **The trap.** `_Pack0` is a Pascal-style call whose selector word is pushed LAST, so the frame differs per selector. [`FRAMES`]
//! holds every selector's name, popped bytes and result size (the IM equate table, checked against systemless' dispatcher); the 14
//! the task names are implemented, the rest pop their frame and are named refusals.
//!
//! | selector | name | frame after the selector (deepest first) | result |
//! |---|---|---|---|
//! | $44 | `LNew` | rView, dataBounds (Rect ptrs), cSize (Point), proc, window, drawIt, hasGrow, scrollH, scrollV | ListHandle |
//! | $28 | `LDispose` | lHandle | |
//! | $2C | `LDoDraw` | drawIt, lHandle | (sets / clears `lDrawingModeOff` in `listFlags`) |
//! | $08 | `LAddRow` | count, rowNum, lHandle | INTEGER (the first new row) |
//! | $24 | `LDelRow` | count, rowNum, lHandle | |
//! | $58 | `LSetCell` | dataPtr, dataLen, theCell, lHandle | |
//! | $38 | `LGetCell` | dataPtr, VAR dataLen, theCell, lHandle | |
//! | $5C | `LSetSelect` | setIt, theCell, lHandle | |
//! | $3C | `LGetSelect` | next, VAR theCell, lHandle | BOOLEAN |
//! | $18 | `LClick` | pt, modifiers, lHandle | BOOLEAN (a double click) |
//! | $64 / $30 | `LUpdate` / `LDraw` | rgn / theCell, lHandle | (the host draws a projected list) |
//! | $50 | `LScroll` | dCols, dRows, lHandle | |
//! | $60 | `LSize` | listWidth, listHeight, lHandle | |
//! | $10 | `LAutoScroll` | lHandle | |
//!
//! Facts the engine follows: `LNew` with a zero `cSize` picks cell height `txSize + 2` (at least 11) and width = view width / columns;
//! `LAddRow` inserts BEFORE `rowNum` (clamped into the list; past the end appends) and returns that row; `LDelRow` with a zero
//! count deletes nothing; an ordinary `LClick` clears the selection, selects the cell under the point (all of it when `selFlags`
//! has `lOnlyOne` $80), and returns TRUE when the same cell was clicked within 20 ticks; Shift or Command keeps the others. A cell
//! outside `dataBounds` in `LSetCell`/`LGetCell`/`LSetSelect` is a named refusal (the ROM would corrupt the list).
use super::{guest_rect, Obj};
use crate::events::{MOUSE_DOWN, MOUSE_UP};
use crate::lowmem::TICKS;
use crate::machine::{Mac, Regs};
use crate::pascal::Rect;
use crate::Refusal;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_ui_abi::{CallbackToken, ControlKind, ControlModel, GuestUi, Intent};
use heirloom_cpu_68k::Cpu;

pub const RECORD_SIZE: u32 = 88;
pub const L_RVIEW: u32 = 0;
pub const L_PORT: u32 = 8;
pub const L_CELL_SIZE: u32 = 16;
pub const L_VISIBLE: u32 = 20;
pub const L_SEL_FLAGS: u32 = 36;
pub const L_V_SCROLL: u32 = 28;
pub const L_H_SCROLL: u32 = 32;
pub const L_ACTIVE: u32 = 37;
pub const L_LIST_FLAGS: u32 = 39;
/// `listFlags` bit 3, `lDrawingModeOff`: the program asked the list not to draw (`LDoDraw(FALSE)`).
pub const L_DRAWING_MODE_OFF: u8 = 0x08;
pub const L_CLICK_TIME: u32 = 40;
pub const L_LAST_CLICK: u32 = 56;
pub const L_DATA_BOUNDS: u32 = 72;
pub const L_CELLS: u32 = 80;
/// `selFlags` bit: only one cell can be selected.
pub const L_ONLY_ONE: u8 = 0x80;
/// The standard text LDEF; any other `theProc` is the program's own definition.
pub const LDEF_TEXT: i16 = 0;
/// A double click is the same cell again within this many ticks (the Toolbox default `DoubleTime` is 32; systemless uses 20).
const DOUBLE_CLICK_TICKS: u32 = 20;
const SHIFT_OR_CMD: u16 = 0x0300;

/// What a selector leaves in the slot above its frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Res { None, Boolean, Integer, Long }
/// One `_Pack0` selector: its name, the bytes its frame pops (selector word included) and its result.
pub struct Frame { pub sel: u16, pub name: &'static str, pub pop: u32, pub res: Res }
const fn fr(sel: u16, name: &'static str, pop: u32, res: Res) -> Frame { Frame { sel, name, pop, res } }
pub const FRAMES: &[Frame] = &[
    fr(0x00, "LActivate", 8, Res::None), fr(0x04, "LAddColumn", 10, Res::Integer), fr(0x08, "LAddRow", 10, Res::Integer), fr(0x0C, "LAddToCell", 16, Res::None),
    fr(0x10, "LAutoScroll", 6, Res::None), fr(0x14, "LCellSize", 10, Res::None), fr(0x18, "LClick", 12, Res::Boolean), fr(0x1C, "LClrCell", 10, Res::None),
    fr(0x20, "LDelColumn", 10, Res::None), fr(0x24, "LDelRow", 10, Res::None), fr(0x28, "LDispose", 6, Res::None), fr(0x2C, "LDoDraw", 8, Res::None),
    fr(0x30, "LDraw", 10, Res::None), fr(0x34, "LFind", 18, Res::None), fr(0x38, "LGetCell", 18, Res::None), fr(0x3C, "LGetSelect", 12, Res::Boolean),
    fr(0x40, "LLastClick", 6, Res::Long), fr(0x44, "LNew", 28, Res::Long), fr(0x48, "LNextCell", 14, Res::Boolean), fr(0x4C, "LRect", 14, Res::None),
    fr(0x50, "LScroll", 10, Res::None), fr(0x54, "LSearch", 20, Res::Boolean), fr(0x58, "LSetCell", 16, Res::None), fr(0x5C, "LSetSelect", 12, Res::None),
    fr(0x60, "LSize", 10, Res::None), fr(0x64, "LUpdate", 10, Res::None),
];

/// A cell: (row, column) = the Mac `Cell` Point's (v, h).
pub type Cell = (i16, i16);

/// What the engine holds for a list beyond its record.
#[derive(Debug, Default, Clone)]
pub struct ListState { pub ldef: i16, pub cells: BTreeMap<Cell, Vec<u8>>, pub selected: BTreeSet<Cell> }

/// What the record says right now.
#[derive(Debug, Clone, Copy)]
pub struct Facts { pub port: u32, pub view: Rect, pub cell_size: (i16, i16), pub visible: Rect, pub bounds: Rect, pub sel_flags: u8 }

impl Facts {
    fn contains(&self, c: Cell) -> bool { c.0 >= self.bounds.top && c.0 < self.bounds.bottom && c.1 >= self.bounds.left && c.1 < self.bounds.right }
    fn rows(&self) -> i16 { (self.bounds.bottom - self.bounds.top).max(0) }
    fn cols(&self) -> i16 { (self.bounds.right - self.bounds.left).max(0) }
    /// The cell under the view-local-global point `(v, h)`, if any.
    fn cell_at(&self, v: i16, h: i16) -> Option<Cell> {
        let r = self.view;
        if v < r.top || v >= r.bottom || h < r.left || h >= r.right { return None; }
        let c = (self.visible.top + (v - r.top) / self.cell_size.0.max(1), self.visible.left + (h - r.left) / self.cell_size.1.max(1));
        self.contains(c).then_some(c)
    }
}

fn nr(r: Rect) -> [i16; 4] { [r.top, r.left, r.bottom, r.right] }

impl<U: GuestUi> Mac<U> {
    fn list_rec(&self, lh: u32, what: &str) -> Result<u32, Refusal> {
        if !self.tr.lists.contains_key(&lh) { return Err(Refusal(format!("{what}: {lh:#x} is not a ListHandle"))); }
        self.mm.block(lh).map(|b| b.ptr).ok_or_else(|| Refusal(format!("{what}: {lh:#x} is not a live handle")))
    }

    pub fn list_facts(&self, lh: u32) -> Result<Facts, Refusal> {
        let p = self.list_rec(lh, "List")?;
        let e = Refusal::mem("ListRec");
        let rect = |at| crate::pascal::read_rect(&self.mem, at);
        Ok(Facts {
            port: self.mem.read_u32(p + L_PORT).map_err(&e)?, view: rect(p + L_RVIEW)?, visible: rect(p + L_VISIBLE)?, bounds: rect(p + L_DATA_BOUNDS)?,
            cell_size: (self.mem.read_i16(p + L_CELL_SIZE).map_err(&e)?, self.mem.read_i16(p + L_CELL_SIZE + 2).map_err(&e)?),
            sel_flags: self.mem.read_u8(p + L_SEL_FLAGS).map_err(&e)?,
        })
    }

    fn put_rect(&mut self, at: u32, r: Rect) -> Result<(), Refusal> {
        for (i, v) in nr(r).into_iter().enumerate() { self.mem.write_u16(at + 2 * i as u32, v as u16).map_err(Refusal::mem("ListRec"))?; }
        Ok(())
    }

    fn put_point(&mut self, at: u32, p: (i16, i16)) -> Result<(), Refusal> {
        self.mem.write_u16(at, p.0 as u16).and_then(|_| self.mem.write_u16(at + 2, p.1 as u16)).map_err(Refusal::mem("ListRec"))
    }

    /// The scroll origin kept inside the bounds and the visible rectangle recomputed from the view and the cell size.
    fn refresh_visible(&mut self, lh: u32, origin: Option<(i16, i16)>) -> Result<(), Refusal> {
        let f = self.list_facts(lh)?;
        let (cv, ch) = (f.cell_size.0.max(1), f.cell_size.1.max(1));
        let rows_in_view = ((f.view.bottom - f.view.top).max(0) + cv - 1) / cv;
        let cols_in_view = ((f.view.right - f.view.left).max(0) + ch - 1) / ch;
        let (ov, oh) = origin.unwrap_or((f.visible.top, f.visible.left));
        let top = ov.clamp(f.bounds.top, (f.bounds.bottom - 1).max(f.bounds.top));
        let left = oh.clamp(f.bounds.left, (f.bounds.right - 1).max(f.bounds.left));
        let p = self.list_rec(lh, "List")?;
        self.put_rect(p + L_VISIBLE, Rect { top, left, bottom: (top + rows_in_view).min(f.bounds.bottom), right: (left + cols_in_view).min(f.bounds.right) })
    }

    /// The native model of a list, or `None` for a list the kit has no part for (a custom LDEF).
    fn list_model(&self, lh: u32) -> Result<Option<(u32, ControlModel)>, Refusal> {
        let f = self.list_facts(lh)?;
        let st = &self.tr.lists[&lh];
        if st.ldef != LDEF_TEXT { return Ok(None); }
        let text = |c: Cell| st.cells.get(&c).map(|b| crate::menu::mac_text(b)).unwrap_or_default();
        let rows: Vec<Vec<String>> = (f.bounds.top..f.bounds.bottom).map(|r| (f.bounds.left..f.bounds.right).map(|c| text((r, c))).collect()).collect();
        let columns = if f.cols() > 1 { (0..f.cols()).map(|_| String::new()).collect() } else { Vec::new() };
        let selected = st.selected.iter().next().map(|&(r, _)| (r - f.bounds.top) as u32);
        let top = (f.visible.top - f.bounds.top).max(0) as u32;
        Ok(Some((f.port, ControlModel { kind: ControlKind::List { columns, rows, selected, top }, label: String::new(), rect: guest_rect(f.view), enabled: true, token: CallbackToken(u64::from(lh)) })))
    }

    /// Re-derive the native list from the record and the engine's cells and push the difference.
    pub(crate) fn sync_list(&mut self, lh: u32) -> Result<(), Refusal> {
        let (port, want) = match self.list_model(lh)? { Some((p, m)) => (p, Some(m)), None => (self.list_facts(lh)?.port, None) };
        self.sync_projection(port, Obj::List(lh), want)
    }

    pub(crate) fn project_lists(&mut self, win: u32) -> Result<(), Refusal> {
        let mine: Vec<u32> = self.tr.lists.keys().copied().collect();
        for lh in mine { if self.list_facts(lh)?.port == win { self.sync_list(lh)?; } }
        Ok(())
    }

    /// A native intent on a list: a row choice is the `mouseDown` the program answers with `LClick`; scrolling moves `visible`.
    pub(crate) fn list_intent(&mut self, lh: u32, intent: Intent) -> Result<(), Refusal> {
        let f = self.list_facts(lh)?;
        let refuse = |why: String| Refusal(format!("Intent {intent:?} on list {lh:#x}: {why}"));
        match intent {
            Intent::SelectRow(r) => {
                let row = f.bounds.top.checked_add(r.min(i16::MAX as u32) as i16).filter(|&row| row < f.bounds.bottom).ok_or_else(|| refuse(format!("row {r} outside the list's {} rows", f.rows())))?;
                if row < f.visible.top || row >= f.visible.bottom { return Err(refuse(format!("row {r} is scrolled out of view (visible rows {}..{})", f.visible.top - f.bounds.top, f.visible.bottom - f.bounds.top))); }
                let col = f.visible.left.max(f.bounds.left);
                let (v, h) = (f.view.top + (row - f.visible.top) * f.cell_size.0 + f.cell_size.0 / 2, f.view.left + (col - f.visible.left) * f.cell_size.1 + f.cell_size.1 / 2);
                let w = self.windows.get(&f.port).ok_or_else(|| refuse("the list's window is not open".into()))?.rect;
                self.tr.supersede();
                let (gv, gh) = (v.wrapping_add(w.top), h.wrapping_add(w.left));
                self.mouse_event(MOUSE_DOWN, gv, gh, 0)?;
                self.mouse_event(MOUSE_UP, gv, gh, 0)
            }
            Intent::ScrollTo(top) => {
                if f.rows() == 0 || top as i64 >= i64::from(f.rows()) { return Err(refuse(format!("top {top} outside the list's {} rows", f.rows()))); }
                self.refresh_visible(lh, Some((f.bounds.top + top as i16, f.visible.left)))?;
                self.sync_list(lh)
            }
            _ => Err(refuse("this intent means nothing for a list".into())),
        }
    }

    fn state_mut(&mut self, lh: u32) -> Result<&mut ListState, Refusal> { self.tr.lists.get_mut(&lh).ok_or_else(|| Refusal(format!("List: {lh:#x} is not a ListHandle"))) }

    fn check_cell(&self, lh: u32, c: Cell, what: &str) -> Result<Facts, Refusal> {
        let f = self.list_facts(lh)?;
        if !f.contains(c) { return Err(Refusal(format!("{what}: cell ({},{}) outside the list's dataBounds ({},{},{},{})", c.0, c.1, f.bounds.top, f.bounds.left, f.bounds.bottom, f.bounds.right))); }
        Ok(f)
    }

    fn make_list(&mut self, a: &NewArgs) -> Result<u32, Refusal> {
        let (view, bounds) = (crate::pascal::read_rect(&self.mem, a.view)?, crate::pascal::read_rect(&self.mem, a.bounds)?);
        if bounds.bottom < bounds.top || bounds.right < bounds.left { return Err(Refusal(format!("LNew: dataBounds ({},{},{},{}) is upside down", bounds.top, bounds.left, bounds.bottom, bounds.right))); }
        let cols = (bounds.right - bounds.left).max(1);
        let tx_size = if self.windows.contains_key(&a.window) { self.mem.read_i16(a.window + 74).unwrap_or(12) } else { 12 };
        let cell = (if a.cell.0 > 0 { a.cell.0 } else { tx_size.max(9) + 2 }, if a.cell.1 > 0 { a.cell.1 } else { ((view.right - view.left).max(1) / cols).max(1) });
        let lh = self.mm.new_handle(&mut self.mem, &mut self.heap, RECORD_SIZE, true).map_err(|e| Refusal(format!("LNew: no memory for a ListRec (memFullErr {e})")))?;
        let cells = self.mm.new_handle(&mut self.mem, &mut self.heap, 8, true).map_err(|e| Refusal(format!("LNew: no memory for the cells handle (memFullErr {e})")))?;
        let p = self.mm.block(lh).map(|b| b.ptr).ok_or_else(|| Refusal("LNew: the new handle has no block".into()))?;
        self.put_rect(p + L_RVIEW, view)?;
        self.mem.write_u32(p + L_PORT, a.window).map_err(Refusal::mem("ListRec"))?;
        self.put_point(p + L_CELL_SIZE, cell)?;
        self.put_rect(p + L_DATA_BOUNDS, bounds)?;
        self.mem.write_u8(p + L_ACTIVE, 1).map_err(Refusal::mem("ListRec"))?;
        self.mem.write_u32(p + L_CELLS, cells).map_err(Refusal::mem("ListRec"))?;
        self.put_point(p + L_LAST_CLICK, (-1, -1))?;
        // hasGrow-independent scroll bars (scrollH / scrollV TRUE): real ControlRecords in vScroll / hScroll, so a program that sets their refCon, ranges
        // or hilite finds them. They are HIDDEN (contrlVis 0, so never projected): the native list draws and moves its own scroll bars.
        for (field, on, rect) in [
            (L_V_SCROLL, a.scroll_v, Rect { top: view.top - 1, left: view.right, bottom: view.bottom + 1, right: view.right + 16 }),
            (L_H_SCROLL, a.scroll_h, Rect { top: view.bottom, left: view.left - 1, bottom: view.bottom + 16, right: view.right + 1 }),
        ] {
            if !on { continue; }
            let ctl = self.create_control(a.window, rect, "", false, 0, 0, 0, 16, 0)?;
            self.mem.write_u32(p + field, ctl).map_err(Refusal::mem("ListRec"))?;
        }
        self.tr.lists.insert(lh, ListState { ldef: a.proc, ..Default::default() });
        self.put_rect(p + L_VISIBLE, Rect { top: bounds.top, left: bounds.left, bottom: bounds.top, right: bounds.left })?;
        self.refresh_visible(lh, Some((bounds.top, bounds.left)))?;
        if a.proc != LDEF_TEXT {
            self.log.push(format!("[list] LDEF {} is not the standard text LDEF: {lh:#x} is not shown natively (its cells are drawn by the program's own definition, which is not run)", a.proc));
        }
        self.sync_list(lh)?;
        Ok(lh)
    }

    fn dispose_list(&mut self, lh: u32) -> Result<(), Refusal> {
        let p = self.list_rec(lh, "LDispose")?;
        let cells = self.mem.read_u32(p + L_CELLS).map_err(Refusal::mem("ListRec"))?;
        let port = self.list_facts(lh)?.port;
        self.sync_projection(port, Obj::List(lh), None)?;
        for field in [L_V_SCROLL, L_H_SCROLL] {
            let ctl = self.mem.read_u32(p + field).unwrap_or(0);
            if ctl != 0 { let _ = self.dispose_control(ctl, true); }
        }
        self.tr.lists.remove(&lh);
        if cells != 0 { let _ = self.dispose_handle(cells); }
        self.dispose_handle(lh).map_err(|e| Refusal(format!("LDispose: memory manager error {e}")))
    }

    fn l_click(&mut self, lh: u32, (v, h): (i16, i16), modifiers: u16) -> Result<bool, Refusal> {
        let f = self.list_facts(lh)?;
        let p = self.list_rec(lh, "LClick")?;
        let now = self.mem.read_u32(TICKS).map_err(Refusal::mem("Ticks"))?;
        let (last_t, last_c) = (self.mem.read_u32(p + L_CLICK_TIME).map_err(Refusal::mem("ListRec"))?, (self.mem.read_i16(p + L_LAST_CLICK).unwrap_or(-1), self.mem.read_i16(p + L_LAST_CLICK + 2).unwrap_or(-1)));
        let in_view = v >= f.view.top && v < f.view.bottom && h >= f.view.left && h < f.view.right;
        let cell = f.cell_at(v, h);
        let ordinary = modifiers & SHIFT_OR_CMD == 0;
        let only_one = f.sel_flags & L_ONLY_ONE != 0;
        let st = self.state_mut(lh)?;
        if in_view && (ordinary || (cell.is_some() && only_one)) { st.selected.clear(); }
        if let Some(c) = cell { st.selected.insert(c); }
        let double = cell.is_some_and(|c| c == last_c && now.wrapping_sub(last_t) <= DOUBLE_CLICK_TICKS);
        self.put_point(p + L_LAST_CLICK, cell.unwrap_or((-1, -1)))?;
        self.mem.write_u32(p + L_CLICK_TIME, now).map_err(Refusal::mem("ListRec"))?;
        self.restore_native(Obj::List(lh))?; // the host showed the choice optimistically; the record + cells are the truth
        self.sync_list(lh)?;
        Ok(double)
    }
}

/// `LNew`'s arguments, read off the frame.
struct NewArgs { view: u32, bounds: u32, cell: (i16, i16), proc: i16, window: u32, scroll_h: bool, scroll_v: bool }

fn word(m: &Mac<impl GuestUi>, at: u32) -> Result<i16, Refusal> { m.mem.read_i16(at).map_err(Refusal::mem("_Pack0 argument")) }
fn long(m: &Mac<impl GuestUi>, at: u32) -> Result<u32, Refusal> { m.mem.read_u32(at).map_err(Refusal::mem("_Pack0 argument")) }
fn boolean(m: &Mac<impl GuestUi>, at: u32) -> Result<bool, Refusal> { Ok(m.mem.read_u8(at).map_err(Refusal::mem("_Pack0 argument"))? != 0) }
fn point(m: &Mac<impl GuestUi>, at: u32) -> Result<Cell, Refusal> { Ok((word(m, at)?, word(m, at + 2)?)) }

/// One selector: `sp` is the selector word's address, so argument `i` bytes in is at `sp + 2 + ...` (the LAST argument is nearest).
/// Returns the result (as the slot's value) and the log line.
fn run_selector<U: GuestUi>(m: &mut Mac<U>, sel: u16, sp: u32) -> Result<(u32, String), Refusal> {
    let lh = |m: &Mac<U>| long(m, sp + 2);
    Ok(match sel {
        0x44 => {
            let a = NewArgs { view: long(m, sp + 24)?, bounds: long(m, sp + 20)?, cell: point(m, sp + 16)?, proc: word(m, sp + 14)?, window: long(m, sp + 10)?, scroll_h: boolean(m, sp + 4)?, scroll_v: boolean(m, sp + 2)? };
            let h = m.make_list(&a)?;
            (h, format!("[list] LNew LDEF {} -> {h:#x}", a.proc))
        }
        0x00 => {
            // LActivate(act, lHandle): the record's `lActive`; the projected control follows (an inactive list takes no clicks)
            let (h, act) = (long(m, sp + 2)?, boolean(m, sp + 6)?);
            let p = m.list_rec(h, "LActivate")?;
            m.mem.write_u8(p + L_ACTIVE, u8::from(act)).map_err(Refusal::mem("ListRec"))?;
            m.sync_list(h)?;
            (0, format!("[list] LActivate {h:#x} {act}"))
        }
        0x28 => { let h = lh(m)?; m.dispose_list(h)?; (0, format!("[list] LDispose {h:#x}")) }
        0x2C => {
            // LDoDraw(drawIt, lHandle): AMS `LSetDrawingMode_call` clears lDrawingModeOff for TRUE and sets it for FALSE. The record is
            // the program's to read; a projected native list always shows its cells (the program batches updates with this call, and
            // the batch is invisible to a user), which is a named limit.
            let (h, draw) = (lh(m)?, boolean(m, sp + 6)?);
            let p = m.list_rec(h, "LDoDraw")?;
            let flags = m.mem.read_u8(p + L_LIST_FLAGS).map_err(Refusal::mem("ListRec"))?;
            let flags = if draw { flags & !L_DRAWING_MODE_OFF } else { flags | L_DRAWING_MODE_OFF };
            m.mem.write_u8(p + L_LIST_FLAGS, flags).map_err(Refusal::mem("ListRec"))?;
            (0, format!("[list] LDoDraw {h:#x} {draw} (listFlags {flags:#04x}; a native list always shows its cells)"))
        }
        0x08 => {
            let (h, row, count) = (lh(m)?, word(m, sp + 6)?, word(m, sp + 8)?);
            let f = m.list_facts(h)?;
            let at = row.clamp(f.bounds.top, f.bounds.bottom);
            if count > 0 {
                let st = m.state_mut(h)?;
                let shift = |&(r, c): &Cell| if r >= at { (r + count, c) } else { (r, c) };
                st.cells = core::mem::take(&mut st.cells).into_iter().map(|(k, v)| (shift(&k), v)).collect();
                st.selected = st.selected.iter().map(shift).collect();
                let p = m.list_rec(h, "LAddRow")?;
                m.put_rect(p + L_DATA_BOUNDS, Rect { bottom: f.bounds.bottom + count, ..f.bounds })?;
                m.refresh_visible(h, None)?;
                m.sync_list(h)?;
            }
            (u32::from(at as u16), format!("[list] LAddRow {h:#x} {count} at {at}"))
        }
        0x24 => {
            let (h, row, count) = (lh(m)?, word(m, sp + 6)?, word(m, sp + 8)?);
            let f = m.list_facts(h)?;
            if count > 0 && row >= f.bounds.top && row < f.bounds.bottom {
                let n = count.min(f.bounds.bottom - row);
                let st = m.state_mut(h)?;
                let keep = |&(r, _): &Cell| r < row || r >= row + n;
                let shift = |(r, c): Cell| if r >= row + n { (r - n, c) } else { (r, c) };
                st.cells = core::mem::take(&mut st.cells).into_iter().filter(|(k, _)| keep(k)).map(|(k, v)| (shift(k), v)).collect();
                st.selected = st.selected.iter().filter(|k| keep(k)).map(|&k| shift(k)).collect();
                let p = m.list_rec(h, "LDelRow")?;
                m.put_rect(p + L_DATA_BOUNDS, Rect { bottom: f.bounds.bottom - n, ..f.bounds })?;
                m.refresh_visible(h, None)?;
                m.sync_list(h)?;
            }
            (0, format!("[list] LDelRow {h:#x} {count} at {row}"))
        }
        0x58 => {
            let (h, cell, len, data) = (lh(m)?, point(m, sp + 6)?, word(m, sp + 10)?, long(m, sp + 12)?);
            m.check_cell(h, cell, "LSetCell")?;
            let bytes = if len > 0 { m.mem.bytes(m.mem.span(data, len as u32).map_err(Refusal::mem("LSetCell data"))?).map_err(Refusal::mem("LSetCell data"))?.to_vec() } else { Vec::new() };
            m.state_mut(h)?.cells.insert(cell, bytes);
            m.sync_list(h)?;
            (0, format!("[list] LSetCell {h:#x} ({},{}) {len} byte(s)", cell.0, cell.1))
        }
        0x38 => {
            let (h, cell, len_ptr, data) = (lh(m)?, point(m, sp + 6)?, long(m, sp + 10)?, long(m, sp + 14)?);
            m.check_cell(h, cell, "LGetCell")?;
            let room = word(m, len_ptr)?.max(0) as usize;
            let bytes: Vec<u8> = m.tr.lists[&h].cells.get(&cell).map(|b| b.iter().copied().take(room).collect()).unwrap_or_default();
            m.mem.write_bytes(data, &bytes).map_err(Refusal::mem("LGetCell data"))?;
            m.mem.write_u16(len_ptr, bytes.len() as u16).map_err(Refusal::mem("LGetCell dataLen"))?;
            (0, format!("[list] LGetCell {h:#x} ({},{}) -> {} byte(s)", cell.0, cell.1, bytes.len()))
        }
        0x5C => {
            let (h, cell, set) = (lh(m)?, point(m, sp + 6)?, boolean(m, sp + 10)?);
            let f = m.check_cell(h, cell, "LSetSelect")?;
            let st = m.state_mut(h)?;
            if set { if f.sel_flags & L_ONLY_ONE != 0 { st.selected.clear(); } st.selected.insert(cell); } else { st.selected.remove(&cell); }
            m.sync_list(h)?;
            (0, format!("[list] LSetSelect {h:#x} ({},{}) {set}", cell.0, cell.1))
        }
        0x3C => {
            let (h, cell_ptr, next) = (lh(m)?, long(m, sp + 6)?, boolean(m, sp + 10)?);
            m.list_facts(h)?;
            let at = point(m, cell_ptr)?;
            let sel = &m.tr.lists.get(&h).ok_or_else(|| Refusal(format!("LGetSelect: {h:#x} is not a ListHandle")))?.selected;
            let found = if next { sel.range(at..).next().copied() } else { sel.contains(&at).then_some(at) };
            if let Some(c) = found { m.put_point(cell_ptr, c)?; }
            (if found.is_some() { 0x0100 } else { 0 }, format!("[list] LGetSelect {h:#x} next={next} -> {found:?}"))
        }
        0x18 => {
            let (h, mods, pt) = (lh(m)?, word(m, sp + 6)? as u16, point(m, sp + 8)?);
            let double = m.l_click(h, pt, mods)?;
            (if double { 0x0100 } else { 0 }, format!("[list] LClick {h:#x} ({},{}) -> {}", pt.0, pt.1, if double { "double click" } else { "single" }))
        }
        0x40 => {
            let h = lh(m)?;
            let p = m.list_rec(h, "LLastClick")?;
            let c = (m.mem.read_u16(p + L_LAST_CLICK).map_err(Refusal::mem("ListRec"))?, m.mem.read_u16(p + L_LAST_CLICK + 2).map_err(Refusal::mem("ListRec"))?);
            (u32::from(c.0) << 16 | u32::from(c.1), format!("[list] LLastClick {h:#x}"))
        }
        0x64 | 0x30 => {
            let h = long(m, sp + 2)?;
            m.list_rec(h, "LUpdate/LDraw")?;
            (0, format!("[list] {} {h:#x}: the host draws a projected list; a custom LDEF's cells are not drawn", if sel == 0x64 { "LUpdate" } else { "LDraw" }))
        }
        0x50 => {
            let (h, d_rows, d_cols) = (lh(m)?, word(m, sp + 6)?, word(m, sp + 8)?);
            let f = m.list_facts(h)?;
            m.refresh_visible(h, Some((f.visible.top.saturating_add(d_rows), f.visible.left.saturating_add(d_cols))))?;
            m.sync_list(h)?;
            (0, format!("[list] LScroll {h:#x} by ({d_rows},{d_cols}) rows,cols"))
        }
        0x60 => {
            let (h, height, width) = (lh(m)?, word(m, sp + 6)?, word(m, sp + 8)?);
            let f = m.list_facts(h)?;
            let p = m.list_rec(h, "LSize")?;
            m.put_rect(p + L_RVIEW, Rect { bottom: f.view.top.saturating_add(height), right: f.view.left.saturating_add(width), ..f.view })?;
            m.refresh_visible(h, None)?;
            m.sync_list(h)?;
            (0, format!("[list] LSize {h:#x} {width}x{height}"))
        }
        0x10 => {
            let h = lh(m)?;
            let f = m.list_facts(h)?;
            let first = m.tr.lists.get(&h).and_then(|s| s.selected.iter().next().copied());
            if let Some((r, c)) = first {
                let vis_rows = f.visible.bottom - f.visible.top;
                let top = if r < f.visible.top { r } else if r >= f.visible.bottom { r - vis_rows.max(1) + 1 } else { f.visible.top };
                let left = if c < f.visible.left { c } else if c >= f.visible.right { c - (f.visible.right - f.visible.left).max(1) + 1 } else { f.visible.left };
                m.refresh_visible(h, Some((top, left)))?;
                m.sync_list(h)?;
            }
            (0, format!("[list] LAutoScroll {h:#x} -> {first:?} in view"))
        }
        other => {
            let name = FRAMES.iter().find(|f| f.sel == other).map_or("?", |f| f.name);
            return Err(Refusal(format!("_Pack0 selector ${other:02X} ({name}) is not implemented")));
        }
    })
}

/// `_Pack0` ($A9E7): the selector word is on top of the stack, the frame below it depends on the selector.
fn pack0<U: GuestUi>(m: &mut Mac<U>, cpu: &mut Cpu, _: &mut Regs) -> Result<String, Refusal> {
    let sp = cpu.areg(7);
    let sel = m.mem.read_u16(sp).map_err(Refusal::mem("_Pack0 selector"))?;
    let Some(frame) = FRAMES.iter().find(|f| f.sel == sel) else {
        return Err(Refusal(format!("_Pack0: selector ${sel:02X} is not a List Manager selector (its frame is unknown, the stack is left as the program made it)")));
    };
    let outcome = run_selector(m, sel, sp);
    // the convention is applied once, here: pop the frame whatever the selector did, store the result in the slot above it
    let new_sp = sp.checked_add(frame.pop).ok_or_else(|| Refusal("_Pack0: sp overflows".into()))?;
    let result = *outcome.as_ref().map(|(r, _)| r).unwrap_or(&0);
    match frame.res {
        Res::Integer => m.mem.write_u16(new_sp, result as u16),
        Res::Boolean => m.mem.write_u16(new_sp, (result as u16) & 0xFF00), // a BOOLEAN is the slot's first (high) byte
        Res::Long => m.mem.write_u32(new_sp, result),
        Res::None => Ok(()),
    }.map_err(Refusal::mem("_Pack0 result"))?;
    cpu.set_areg(7, new_sp);
    outcome.map(|(_, log)| log)
}

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> { m.register_os(0xA9E7, pack0::<U>) }
