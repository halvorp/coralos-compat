//! `SetDialogItem` (Inside Macintosh I, "The Dialog Manager": `SetDialogItem(theDialog, itemNo, itemType, item, box)`): replace an
//! item's type, handle and display rectangle in the dialog's item list. The call stores what it is given: it does not create,
//! dispose, move or draw anything. The program owns what it passes (a `ControlHandle` for a control item, the text `Handle` for
//! a text item, a draw PROCEDURE for a user item), and the program's own `NewControl` / `MoveControl` keep a control's geometry:
//! a control draws where its record says, so the native control keeps its model and only the item list changes.
//!
//! What the engine keeps in step with the item list: the dialog's private `DITL` copy (placeholder long, rect and type of the
//! item, as `GetDialogItem` reads them back), the slot the Dialog Manager table is keyed on (a control item is found by its
//! handle, a text item by its handle) and the native part of a TEXT item: a static or editable text item is a native label or
//! field made from its handle and rect, so a changed rect, a changed handle or a changed type (static <-> editable, or no longer
//! text at all) is pushed to the host and the part that no longer stands for the item is retired.
use super::dialogs::{item_of, parse_ditl, slot_of, Item};
use super::Obj;
use crate::machine::{Call, Done, Mac};
use crate::pascal::{read_rect, Arg, Rect};
use crate::Refusal;
use alloc::format;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

const DITL_RECT: u32 = 4;
const DITL_TYPE: u32 = 12;

fn is_text(i: Item) -> bool { matches!(i, Item::StaticText | Item::EditText) }

impl<U: GuestUi> Mac<U> {
    /// Replace item `n` of `win` with (`ty`, `handle`, `rect`). Everything the call can refuse is checked before anything changes.
    pub fn set_dialog_item(&mut self, win: u32, n: i16, ty: u8, handle: u32, rect: Rect) -> Result<(), Refusal> {
        const WHAT: &str = "SetDialogItem";
        let old = slot_of(self, win, n, WHAT)?;
        let item = item_of(ty).ok_or_else(|| Refusal(format!("{WHAT}: item type {ty} is not one the Dialog Manager defines (ctrlItem + 0..3, statText, editText, iconItem, picItem, userItem)")))?;
        match item {
            Item::Control { .. } | Item::ResControl if !self.tr.procs.contains_key(&handle) => return Err(Refusal(format!("{WHAT}: item {n} is a control item but {handle:#x} is not a ControlHandle"))),
            Item::StaticText | Item::EditText if handle != 0 && self.mm.block(handle).is_none() => return Err(Refusal(format!("{WHAT}: item {n} is a text item but {handle:#x} is not a live Handle"))),
            _ => {}
        }
        let items = self.tr.dlg.dialogs[&win].items;
        let ip = self.mm.block(items).map(|b| b.ptr).ok_or_else(|| Refusal(format!("{WHAT}: the item list of {win:#x} has no block")))?;
        let at = {
            let bytes = self.mem.bytes(self.mem.span(ip, self.mm.block(items).map_or(0, |b| b.size)).map_err(Refusal::mem("DITL"))?).map_err(Refusal::mem("DITL"))?;
            parse_ditl(bytes)?.get(n as usize - 1).map(|r| r.at as u32).ok_or_else(|| Refusal(format!("{WHAT}: item {n} is not in the item list")))?
        };
        let e = Refusal::mem("DITL");
        self.mem.write_u32(ip + at, handle).map_err(&e)?;
        for (i, v) in [rect.top, rect.left, rect.bottom, rect.right].into_iter().enumerate() { self.mem.write_u16(ip + at + DITL_RECT + 2 * i as u32, v as u16).map_err(&e)?; }
        self.mem.write_u8(ip + at + DITL_TYPE, ty).map_err(&e)?;
        let idx = n as usize - 1;
        if let Some(d) = self.tr.dlg.dialogs.get_mut(&win) { d.list[idx] = super::dialogs::Slot { raw_type: ty, rect, handle, item }; }
        // the native part of a text item follows its slot: the old part goes unless the same handle is still a text item
        if is_text(old.item) && !(is_text(item) && old.handle == handle) { self.sync_projection(win, Obj::Text(old.handle), None)?; }
        if is_text(item) && handle != 0 { self.sync_text(win, idx)?; }
        Ok(())
    }
}

/// The classic "hidden" encoding (spec macos9-specs/DialogsLib/HideDialogItem.md, ShowDialogItem.md): a hidden item's rect is moved right by
/// 0x4000; an item whose left edge is at or past 0x2000 IS hidden. The native part follows the rect off the window and back.
const HIDE_OFFSET: i16 = 0x4000;
const HIDDEN_AT: i16 = 0x2000;
const DR_EDIT_FIELD: u32 = 164;

impl<U: GuestUi> Mac<U> {
    /// `HideDialogItem` (`hide`) / `ShowDialogItem`: a NULL or unknown dialog, an item number outside the list, hiding a hidden item or showing a
    /// shown one change nothing. Hiding the current edit field clears the edit field (editField -1); showing an edit text item in a dialog with no
    /// edit field makes it the edit field. A control item's control is moved with the rect (MoveControl), so its hit area follows.
    pub fn show_dialog_item(&mut self, win: u32, n: i16, hide: bool) -> Result<bool, Refusal> {
        let Some(slot) = self.tr.dlg.dialogs.get(&win).and_then(|d| d.list.get(n as usize - 1).filter(|_| n >= 1)).cloned() else { return Ok(false) };
        if (slot.rect.left >= HIDDEN_AT) == hide { return Ok(false); }
        let dx = if hide { HIDE_OFFSET } else { -HIDE_OFFSET };
        let rect = Rect { left: slot.rect.left.wrapping_add(dx), right: slot.rect.right.wrapping_add(dx), ..slot.rect };
        let e = Refusal::mem("DialogRecord editField");
        let edit = self.mem.read_i16(win + DR_EDIT_FIELD).map_err(&e)?;
        if hide && edit == n - 1 { self.mem.write_u16(win + DR_EDIT_FIELD, 0xFFFF).map_err(&e)?; }
        if !hide && edit < 0 && slot.item == super::dialogs::Item::EditText { self.mem.write_u16(win + DR_EDIT_FIELD, (n - 1) as u16).map_err(&e)?; }
        self.set_dialog_item(win, n, slot.raw_type, slot.handle, rect)?;
        if matches!(slot.item, super::dialogs::Item::Control { .. } | super::dialogs::Item::ResControl) {
            let op = crate::mixed::trap_opcode("MoveControl").ok_or_else(|| Refusal("MoveControl has no trap word".into()))?;
            self.pascal_call(op, &[slot.handle, rect.left as u16 as u32, rect.top as u16 as u32]).map_err(|e| Refusal(format!("HideDialogItem/ShowDialogItem: MoveControl: {e}")))?;
        }
        Ok(true)
    }

    /// `FindDialogItem(theDialog, thePt)`: the ZERO-based index of the first item (list order) whose rect contains the point, -1 for none or an
    /// unknown dialog. A hidden item never matches (its rect is off the window).
    pub fn find_dialog_item(&self, win: u32, (v, h): (i16, i16)) -> i16 {
        let Some(d) = self.tr.dlg.dialogs.get(&win) else { return -1 };
        d.list.iter().position(|s| v >= s.rect.top && v < s.rect.bottom && h >= s.rect.left && h < s.rect.right).map_or(-1, |i| i as i16)
    }
}

impl<U: GuestUi> Mac<U> {
    /// `ShortenDITL(theDialog, numberItems)` (spec macos9-specs/DialogsLib/ShowDialogItem.md): the LAST `numberItems` items go (all of them when
    /// it is larger): a control item's control is disposed, a text item's native part retired, and the dialog's item list (count and bytes) is
    /// cut to the items that stay; an edit field among the removed ones is no longer the edit field. Returns how many were removed.
    pub fn shorten_ditl(&mut self, win: u32, number: i16) -> Result<usize, Refusal> {
        let Some(d) = self.tr.dlg.dialogs.get(&win) else { return Err(Refusal(format!("ShortenDITL: {win:#x} is not a dialog"))) };
        let (count, items) = (d.list.len(), d.items);
        let gone = (number.max(0) as usize).min(count);
        if gone == 0 { return Ok(0); }
        let keep = count - gone;
        let removed: alloc::vec::Vec<super::dialogs::Slot> = self.tr.dlg.dialogs[&win].list[keep..].to_vec();
        for slot in &removed {
            match slot.item {
                super::dialogs::Item::Control { .. } | super::dialogs::Item::ResControl if self.tr.procs.contains_key(&slot.handle) => self.dispose_control(slot.handle, true)?,
                super::dialogs::Item::StaticText | super::dialogs::Item::EditText => self.sync_projection(win, Obj::Text(slot.handle), None)?,
                _ => {}
            }
        }
        let e = Refusal::mem("DITL");
        let block = self.mm.block(items).ok_or_else(|| Refusal(format!("ShortenDITL: the item list of {win:#x} has no block")))?;
        let bytes = self.mem.bytes(self.mem.span(block.ptr, block.size).map_err(&e)?).map_err(&e)?.to_vec();
        let end = parse_ditl(&bytes)?.get(keep).map_or(bytes.len(), |r| r.at);
        self.mem.write_u16(block.ptr, (keep as i16 - 1) as u16).map_err(&e)?;
        self.mm.set_handle_size(&mut self.mem, &mut self.heap, items, end as u32).map_err(|err| Refusal(format!("ShortenDITL: memory manager error {err}")))?;
        if let Some(d) = self.tr.dlg.dialogs.get_mut(&win) { d.list.truncate(keep); }
        let edit = self.mem.read_i16(win + DR_EDIT_FIELD).map_err(Refusal::mem("editField"))?;
        if edit >= keep as i16 { self.mem.write_u16(win + DR_EDIT_FIELD, 0xFFFF).map_err(Refusal::mem("editField"))?; }
        Ok(gone)
    }
}

fn hide_or_show<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (dlg, n, hide) = (c.args.get(0), c.args.get(1) as u16 as i16, c.opcode & 0x0FFF == 0x0827);
    let changed = m.show_dialog_item(dlg, n, hide)?;
    Ok(Done { result: 0, log: format!("[dialog] {} {dlg:#x} #{n}{}", if hide { "HideDialogItem" } else { "ShowDialogItem" }, if changed { "" } else { ": nothing to change" }) })
}

fn find_dialog_item<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (dlg, pt) = (c.args.get(0), c.args.get(1));
    let i = m.find_dialog_item(dlg, ((pt >> 16) as i16, pt as i16));
    Ok(Done { result: i as u16 as u32, log: format!("[dialog] FindDialogItem {dlg:#x} ({},{}) -> {i}", (pt >> 16) as i16, pt as i16) })
}

/// `SetDialogItem(theDialog, itemNo, itemType, item, box)`.
fn set_dialog_item<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (dlg, n, ty, handle) = (c.args.get(0), c.args.get(1) as u16 as i16, c.args.get(2) as u16, c.args.get(3));
    let rect = read_rect(&m.mem, c.args.get(4))?;
    let item_type = u8::try_from(ty).map_err(|_| Refusal(format!("SetDialogItem: item type {ty:#x} does not fit the item list's type byte")))?;
    m.set_dialog_item(dlg, n, item_type, handle, rect)?;
    Ok(Done { result: 0, log: format!("[dialog] SetDialogItem {dlg:#x} #{n} -> type {ty} handle {handle:#x} box ({},{},{},{})", rect.top, rect.left, rect.bottom, rect.right) })
}

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    use Arg::{L, W};
    m.register(0xA98E, &[L, W, W, L, L], None, set_dialog_item::<U>)?;
    m.register(0xA827, &[Arg::P, W], None, hide_or_show::<U>)?;
    m.register(0xA828, &[Arg::P, W], None, hide_or_show::<U>)?;
    m.register(0xA984, &[Arg::P, L], Some(W), find_dialog_item::<U>)
}
