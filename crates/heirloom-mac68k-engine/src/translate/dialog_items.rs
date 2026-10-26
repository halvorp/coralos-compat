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
use heirloom_cpu_68k::Cpu;

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
    m.register(0xA98E, &[L, W, W, L, L], None, set_dialog_item::<U>)
}
