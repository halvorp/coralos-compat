//! The Dialog Manager as a toolkit table (ruling 17, piece 3): DITL item type -> role, plus `GetNewDialog`, `ModalDialog`,
//! `GetDialogItem`, `SetDialogItemText`/`GetDialogItemText` and `DisposeDialog`.
//!
//! **Real structures.** A `DialogRecord` is a `WindowRecord` (156 bytes) followed by `items` (the DITL handle, 156),
//! `textH` (160), `editField` (164), `editOpen` (166) and `aDefItem` (168): 170 bytes. The dialog owns a private COPY of
//! its `DITL` (so the resource stays as the file has it) and, as the real Dialog Manager does, writes each item's
//! placeholder long with the item's handle: a `ControlHandle` for button / check box / radio / `resCtrl` items, a plain
//! `Handle` to the text for static and editable text. `GetDialogItem` and `SetDialogItemText` work on those handles.
//!
//! **The table** ([`ITEM_TABLE`]): item type (the disabled bit 128 masked off) -> what the item is. Button, check box,
//! radio and `resCtrl` items ARE controls and take their role from the Control Manager table; static and editable text are
//! `Label` and `Field`. Icon, picture and user items have no contract to translate: they are named in the log and shown
//! as nothing (a user item is the program's own drawing, a content plane).
//!
//! **Behaviour.** A modal dialog's controls report through the Dialog Manager, not through `mouseDown`: a push button,
//! check box or radio hit is the ITEM NUMBER `ModalDialog` returns in `itemHit` (the program toggles a check box itself,
//! as on a real Mac); a scroll bar's `contrlValue` is moved first (`DialogSelect` runs `TrackControl`, then reports the item).
//! Typing in an enabled `editText` item returns that item's number, as `DialogSelect` does after `TEKey`: a native text
//! commit writes the text into the item's handle and queues the hit; a key reaching the dialog window is applied to the
//! edit field the same way. Keys are the dialog's, never the program's event queue's. Return or Enter is the default
//! item's hit (`aDefItem`, 1 unless the program changed it); an `aDefItem` that names no item is ignored by name.
//! A disabled item's hit is ignored by name, as `ModalDialog` ignores clicks in disabled items. `ModalDialog` blocks like an
//! event call; the PowerPC adapter parks the caller on the same wake condition ([`Mac::modal_ready`]).
//! A non-NIL `filterProc` is a guest callback the trap bridge cannot run from inside a trap: it is named as a refusal
//! (the standard filtering applies), never ignored silently.
use super::{guest_rect, Obj};
use crate::machine::{Call, Done, Mac};
use crate::pascal::{read_pstring, read_rect, Arg, Rect};
use crate::toolbox::WindowSpec;
use crate::Refusal;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use guestui_abi::{CallbackToken, ControlKind, ControlModel, GuestUi, Intent};
use retro_translate::{row, On, Role};
use retro_m68k::Cpu;

pub const RECORD_SIZE: u32 = 170;
const DR_ITEMS: u32 = 156;
const DR_EDIT_FIELD: u32 = 164;
const DR_DEF_ITEM: u32 = 168;

/// What a DITL item type is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Item { Control { proc_id: u16 }, ResControl, StaticText, EditText, /** the program's own drawing: its handle is a draw PROCEDURE ([`super::useritems`]) */ User, NoContract(&'static str) }

/// THE dialog item table: type (disabled bit masked off) -> item.
pub const ITEM_TABLE: &[(u8, Item)] = &[
    (4, Item::Control { proc_id: 0 }),
    (5, Item::Control { proc_id: 1 }),
    (6, Item::Control { proc_id: 2 }),
    (7, Item::ResControl),
    (8, Item::StaticText),
    (16, Item::EditText),
    (32, Item::NoContract("iconItem: an ICON is drawn by the program or not at all")),
    (64, Item::NoContract("picItem: a PICT is drawn by the program or not at all")),
    (0, Item::User),
];

impl Item {
    /// The shared role a text item is shown as (`None`: it takes no intent; static text is a label, buttons are controls).
    fn role(self) -> Option<Role> { matches!(self, Item::EditText).then_some(Role::Field) }
}

/// What a dialog text intent writes FIRST.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect { /** the text goes into the item's handle (`TEKey`'s result) */ SetText }

/// What the Dialog Manager reports after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Post { /** `DialogSelect` returns the item number (`editField + 1`) */ ItemHit }

pub type Row = retro_translate::Row<Effect, Post>;

/// THE dialog intent table: role x intent -> effect, then the report (the twin of `controls::TABLE`; button, check box and radio
/// items use that one).
pub const TABLE: &[Row] = &[row(Role::Field, On::CommitText, Effect::SetText, Some(Post::ItemHit))];

pub(super) fn item_of(ty: u8) -> Option<Item> { ITEM_TABLE.iter().find(|(t, _)| *t == ty & 0x7F).map(|(_, i)| *i) }

/// A dialog item as the engine keeps it. Item `n` (1-based) is `list[n - 1]`.
#[derive(Debug, Clone)]
pub struct Slot { pub raw_type: u8, pub rect: Rect, pub handle: u32, pub item: Item }

pub struct Dialog { pub items: u32, pub list: Vec<Slot>, /** the engine allocated the DialogRecord */ pub storage: Option<u32> }

#[derive(Default)]
pub struct State {
    pub dialogs: BTreeMap<u32, Dialog>,
    /// Item hits waiting for `ModalDialog`: (dialog, item number), oldest first.
    pub hits: VecDeque<(u32, i16)>,
    /// A blocked `ModalDialog`: where its `itemHit` result goes.
    pub modal: Option<u32>,
    /// The alert a blocked `Alert` call is showing: disposed when its hit arrives (alerts are not the program's to dispose).
    pub alert: Option<u32>,
    /// `ParamText`'s four strings (`^0`..`^3`), as raw bytes; a NIL argument leaves its string as it was.
    pub params: [Vec<u8>; 4],
}

/// What a dialog or alert window is built from.
pub(crate) struct Def { pub what: String, pub rect: Rect, pub title: String, pub proc_id: u16, pub go_away: bool, pub ref_con: u32, pub ditl: i16, pub show: bool, /** `NewDialog`'s item list: the dialog uses the program's own handle, as the Dialog Manager does (`GetNewDialog` and alerts copy a `DITL` resource) */ pub items: Option<u32> }

pub(super) struct Raw { pub rect: Rect, pub ty: u8, pub data: Vec<u8>, pub at: usize }

/// `DITL`: count - 1 (2), then per item: placeholder (4), rect (8), type (1), length (1), data (padded to even).
pub(super) fn parse_ditl(b: &[u8]) -> Result<Vec<Raw>, Refusal> {
    let bad = |what: &str| Refusal(format!("DITL: {what} ({} bytes)", b.len()));
    if b.len() < 2 { return Err(bad("shorter than its count")); }
    let n = i32::from(i16::from_be_bytes([b[0], b[1]])) + 1;
    let (mut pos, mut out) = (2usize, Vec::new());
    for _ in 0..n.max(0) {
        if pos + 14 > b.len() { return Err(bad("an item runs past the end")); }
        let be = |i: usize| i16::from_be_bytes([b[pos + i], b[pos + i + 1]]);
        let len = b[pos + 13] as usize;
        if pos + 14 + len > b.len() { return Err(bad("an item's data runs past the end")); }
        out.push(Raw { rect: Rect { top: be(4), left: be(6), bottom: be(8), right: be(10) }, ty: b[pos + 12], data: b[pos + 14..pos + 14 + len].to_vec(), at: pos });
        pos += 14 + len + (len & 1);
    }
    Ok(out)
}

fn text_model(slot: &Slot, text: String) -> ControlModel {
    let kind = if slot.item == Item::EditText { ControlKind::Field { text, max_len: 255, editable: true, integer: false } } else { ControlKind::Label { text, framed: false } };
    ControlModel { kind, label: String::new(), rect: guest_rect(slot.rect), enabled: true, token: CallbackToken(u64::from(slot.handle)) }
}

impl<U: GuestUi> Mac<U> {
    /// The text a text item's handle holds.
    fn handle_text(&self, h: u32) -> Result<String, Refusal> {
        let b = self.mm.block(h).ok_or_else(|| Refusal(format!("{h:#x} is not a live handle")))?;
        let bytes = self.mem.bytes(self.mem.span(b.ptr, b.size).map_err(Refusal::mem("item text"))?).map_err(Refusal::mem("item text"))?;
        Ok(bytes.iter().map(|&c| char::from(c)).collect())
    }

    fn set_handle_text(&mut self, h: u32, text: &str) -> Result<(), Refusal> {
        let bytes: Vec<u8> = text.chars().map(|c| c as u8).collect();
        self.mm.set_handle_size(&mut self.mem, &mut self.heap, h, bytes.len() as u32).map_err(|e| Refusal(format!("item text: no memory for {} bytes (memFullErr {e})", bytes.len())))?;
        let p = self.mm.block(h).map(|b| b.ptr).ok_or_else(|| Refusal(format!("{h:#x} is not a live handle")))?;
        self.mem.write_bytes(p, &bytes).map_err(Refusal::mem("item text"))
    }

    /// The (dialog, 0-based slot) a text handle belongs to.
    pub(super) fn text_slot(&self, h: u32) -> Option<(u32, usize)> {
        self.tr.dlg.dialogs.iter().find_map(|(&w, d)| d.list.iter().position(|s| s.handle == h && matches!(s.item, Item::StaticText | Item::EditText)).map(|i| (w, i)))
    }

    pub(super) fn sync_text(&mut self, win: u32, idx: usize) -> Result<(), Refusal> {
        let slot = self.tr.dlg.dialogs[&win].list[idx].clone();
        let text = self.handle_text(slot.handle)?;
        self.sync_projection(win, Obj::Text(slot.handle), Some(text_model(&slot, text)))
    }

    pub(crate) fn project_dialog_text(&mut self, win: u32) -> Result<(), Refusal> {
        let n = self.tr.dlg.dialogs.get(&win).map_or(0, |d| d.list.len());
        for i in 0..n { if matches!(self.tr.dlg.dialogs[&win].list[i].item, Item::StaticText | Item::EditText) { self.sync_text(win, i)?; } }
        Ok(())
    }

    /// The 1-based item number a dialog's control handle is, if it is one.
    pub(crate) fn dialog_item_of(&self, win: u32, ctl: u32) -> Option<i16> {
        self.tr.dlg.dialogs.get(&win)?.list.iter().position(|s| s.handle == ctl && matches!(s.item, Item::Control { .. } | Item::ResControl)).map(|i| i as i16 + 1)
    }

    pub(crate) fn default_item(&self, win: u32) -> Result<i16, Refusal> {
        self.mem.read_i16(win + DR_DEF_ITEM).map_err(Refusal::mem("aDefItem"))
    }

    /// The user hit item `item` of dialog `win`: queue it for `ModalDialog` (a disabled item ignores it).
    pub fn dialog_hit(&mut self, win: u32, item: i16) {
        // item numbers are guest data (aDefItem): 1-based, anything else is no item
        let slot = usize::try_from(i32::from(item) - 1).ok().and_then(|i| self.tr.dlg.dialogs.get(&win).and_then(|d| d.list.get(i)));
        let enabled = slot.is_some_and(|s| s.raw_type & 0x80 == 0);
        if enabled { self.tr.dlg.hits.push_back((win, item)); }
        else { self.log.push(format!("[dialog] item {item} of {win:#x} is disabled (or not an item): the hit is ignored")); }
    }

    pub(crate) fn text_intent(&mut self, h: u32, intent: Intent) -> Result<(), Refusal> {
        let (win, idx) = self.text_slot(h).ok_or_else(|| Refusal(format!("Intent: {h:#x} is not a dialog text item")))?;
        let item = self.tr.dlg.dialogs[&win].list[idx].item;
        let none = |why: &str| Refusal(format!("Intent {intent:?} has no meaning for a {item:?} dialog item{why}"));
        let role = item.role().ok_or_else(|| none(""))?;
        let r = retro_translate::lookup_role(TABLE, role, &intent).map_err(|e| none(&format!(" ({e})")))?;
        match (r.effect, &intent) {
            (Effect::SetText, Intent::CommitText(t)) => {
                self.set_handle_text(h, t)?;
                self.adopt_native_state(Obj::Text(h), &guestui_abi::ControlValue::Text(t.clone()));
                self.sync_text(win, idx)?;
                if r.post == Some(Post::ItemHit) { self.dialog_hit(win, idx as i16 + 1); } // DialogSelect reports editField + 1 after TEKey
                Ok(())
            }
            (_, i) => Err(none(&format!(": the row's effect does not take {i:?}"))),
        }
    }

    /// The frontmost dialog window.
    pub(crate) fn front_dialog(&self) -> Option<u32> { self.z.iter().copied().find(|w| self.tr.dlg.dialogs.contains_key(w)) }

    /// A key typed into dialog `win` (never the program's event queue): applied to the edit field like `TEKey`, and the
    /// field's item is reported. Command-key equivalents and keys with no edit field are consumed by name.
    pub(crate) fn dialog_key(&mut self, win: u32, ch: u8, command: bool) -> Result<bool, Refusal> {
        let field = self.mem.read_i16(win + DR_EDIT_FIELD).map_err(Refusal::mem("editField"))?;
        let idx = usize::try_from(field).ok().filter(|&i| self.tr.dlg.dialogs[&win].list.get(i).is_some_and(|s| s.item == Item::EditText));
        let Some(idx) = idx.filter(|_| !command) else {
            self.log.push(format!("[dialog] key {ch:#04x} in {win:#x} consumed: {}", if command { "command-key equivalents are not mapped in a modal dialog" } else { "the dialog has no edit field" }));
            return Ok(false);
        };
        let h = self.tr.dlg.dialogs[&win].list[idx].handle;
        let mut text = self.handle_text(h)?;
        match ch { 0x08 => { text.pop(); } c if c >= 0x20 && c != 0x7F => text.push(char::from(c)), _ => return Ok(false) }
        self.set_handle_text(h, &text)?;
        self.sync_text(win, idx)?;
        self.dialog_hit(win, idx as i16 + 1);
        Ok(true)
    }

    pub(crate) fn take_hit(&mut self, win: u32) -> Option<i16> {
        let i = self.tr.dlg.hits.iter().position(|&(w, _)| w == win)?;
        self.tr.dlg.hits.remove(i).map(|(_, item)| item)
    }

    /// Is a blocked `ModalDialog` ready to return? (Pumps the host first.) The PowerPC adapter's wake condition.
    pub fn modal_ready(&mut self) -> bool {
        self.pump_ui();
        self.front_dialog().is_some_and(|w| self.tr.dlg.hits.iter().any(|&(h, _)| h == w))
    }

    /// Finish a blocked `ModalDialog` if its hit has arrived. False while the guest must stay blocked.
    pub(crate) fn complete_modal(&mut self) -> bool {
        let Some(at) = self.tr.dlg.modal else { return true };
        self.pump_ui();
        let Some(item) = self.front_dialog().and_then(|w| self.take_hit(w)) else { return false };
        if let Err(Refusal(why)) = self.mem.write_u16(at, item as u16).map_err(Refusal::mem("itemHit")) { self.note_refusal("ModalDialog", why); }
        self.tr.dlg.modal = None;
        if let Some(w) = self.tr.dlg.alert.take() { if let Err(Refusal(why)) = self.dispose_window(w) { self.note_refusal("Alert", why); } }
        true
    }

    /// Build a dialog from `DLOG` `id` in the open resource files.
    fn new_dialog(&mut self, id: i16, storage: u32, behind: u32) -> Result<Option<u32>, Refusal> {
        let Ok(dh) = self.get_resource(*b"DLOG", id, false) else { return Ok(None) };
        let b = self.handle_bytes(dh).unwrap_or_default();
        if b.len() < 21 || b.len() < 21 + b[20] as usize { return Err(Refusal(format!("GetNewDialog {id}: DLOG resource is {} bytes, too short for its fields and title", b.len()))); }
        let be = |i: usize| i16::from_be_bytes([b[i], b[i + 1]]);
        let title: String = b[21..21 + b[20] as usize].iter().map(|&x| char::from(x)).collect();
        let def = Def { what: format!("GetNewDialog {id}"), rect: Rect { top: be(0), left: be(2), bottom: be(4), right: be(6) }, title, proc_id: be(8) as u16, go_away: b[12] != 0,
            ref_con: u32::from_be_bytes([b[14], b[15], b[16], b[17]]), ditl: be(18), show: b[10] != 0, items: None };
        self.build_dialog(def, storage, behind).map(Some)
    }

    /// Build a dialog window from its definition (a `DLOG`, or an `ALRT` for [`super::alerts`]) and its `DITL`.
    pub(crate) fn build_dialog(&mut self, d: Def, storage: u32, behind: u32) -> Result<u32, Refusal> {
        let id = &d.what;
        let (src, own_items) = match d.items {
            Some(h) => (self.handle_bytes(h).ok_or_else(|| Refusal(format!("{id}: the item list {h:#x} is not a live handle")))?, Some(h)),
            None => {
                let ditl = self.get_resource(*b"DITL", d.ditl, false).map_err(|e| Refusal(format!("{id}: DITL {} is missing (resError {e})", d.ditl)))?;
                (self.handle_bytes(ditl).unwrap_or_default(), None)
            }
        };
        let raw = parse_ditl(&src)?;
        let own = if storage == 0 { Some(self.mm.new_ptr(&mut self.mem, &mut self.heap, RECORD_SIZE, true).map_err(|e| Refusal(format!("{id}: no memory for a DialogRecord (memFullErr {e})")))?) } else { None };
        let record = own.unwrap_or(storage);
        self.mem.zero(record, RECORD_SIZE).map_err(Refusal::mem("DialogRecord"))?;
        let items = match own_items {
            Some(h) => h,
            None => {
                let h = self.mm.new_handle(&mut self.mem, &mut self.heap, src.len() as u32, false).map_err(|e| Refusal(format!("{id}: no memory for the item list (memFullErr {e})")))?;
                let at = self.mm.block(h).map(|b| b.ptr).ok_or_else(|| Refusal(format!("{id}: the item list has no block")))?;
                self.mem.write_bytes(at, &src).map_err(Refusal::mem("DITL"))?;
                h
            }
        };
        let ip = self.mm.block(items).map(|b| b.ptr).ok_or_else(|| Refusal(format!("{id}: the item list has no block")))?;
        let spec = WindowSpec { storage: record, rect: d.rect, title: d.title, visible: false, proc_id: d.proc_id, behind, go_away: d.go_away, ref_con: d.ref_con };
        let win = self.make_window(spec)?;
        let e = Refusal::mem("DialogRecord");
        self.mem.write_u32(win + DR_ITEMS, items).map_err(&e)?;
        self.mem.write_u16(win + crate::toolbox::WR_KIND, crate::toolbox::DIALOG_KIND).map_err(&e)?; // windowKind: dialogKind, not userKind
        let dlg_font = self.mem.read_u16(crate::lowmem::DLG_FONT).map_err(&e)?; // the Dialog Manager sets the dialog's font (SetDAFont changes it)
        self.mem.write_u16(win + crate::qd::port::P_TX_FONT, dlg_font).map_err(&e)?;
        self.mem.write_u16(win + DR_EDIT_FIELD, 0xFFFF).map_err(&e)?;
        self.mem.write_u16(win + DR_DEF_ITEM, 1).map_err(&e)?;
        self.tr.dlg.dialogs.insert(win, Dialog { items, list: Vec::new(), storage: own });
        for (n, r) in raw.iter().enumerate() {
            let slot = self.make_item(win, n as i16 + 1, r)?;
            self.mem.write_u32(ip + r.at as u32, slot.handle).map_err(Refusal::mem("DITL"))?;
            if slot.item == Item::EditText && self.mem.read_i16(win + DR_EDIT_FIELD).unwrap_or(-1) < 0 { self.mem.write_u16(win + DR_EDIT_FIELD, n as u16).map_err(&e)?; }
            if let Some(d) = self.tr.dlg.dialogs.get_mut(&win) { d.list.push(slot); }
        }
        if d.show { self.show_window(win)?; }
        Ok(win)
    }

    /// One DITL item -> its guest object (a control, or a text handle) and its slot.
    fn make_item(&mut self, win: u32, n: i16, r: &Raw) -> Result<Slot, Refusal> {
        let item = item_of(r.ty).ok_or_else(|| Refusal(format!("GetNewDialog: item {n} has type {}, which is not in the Dialog Manager table", r.ty)))?;
        // A DITL item's data IS the text (the item's length byte counts it); there is no Pascal length inside (Inside Macintosh: Dialog Manager).
        let pstr = |d: &[u8]| -> String { d.iter().map(|&c| char::from(c)).collect() };
        let handle = match item {
            Item::Control { proc_id } => self.create_control(win, r.rect, &pstr(&r.data), true, 0, 0, 1, proc_id, 0)?,
            Item::ResControl => {
                let id = r.data.get(..2).map(|d| i16::from_be_bytes([d[0], d[1]])).ok_or_else(|| Refusal(format!("GetNewDialog: item {n} is a resCtrl without a CNTL id")))?;
                let h = self.get_resource(*b"CNTL", id, false).map_err(|e| Refusal(format!("GetNewDialog: item {n}: CNTL {id} is missing (resError {e})")))?;
                self.control_from_cntl(h, id, win)?
            }
            Item::StaticText | Item::EditText => {
                let h = self.mm.new_handle(&mut self.mem, &mut self.heap, 0, false).map_err(|e| Refusal(format!("GetNewDialog: no memory for item {n} (memFullErr {e})")))?;
                let text = if item == Item::StaticText { self.substitute_params(&pstr(&r.data)) } else { pstr(&r.data) };
                self.set_handle_text(h, &text)?;
                h
            }
            Item::User => { self.log.push(format!("[dialog] item {n} of {win:#x}: userItem (the program sets its draw procedure with SetDialogItem)")); 0 }
            Item::NoContract(why) => { self.log.push(format!("[dialog] item {n} of {win:#x}: {why}; not shown")); 0 }
        };
        Ok(Slot { raw_type: r.ty, rect: r.rect, handle, item })
    }

    /// The Dialog Manager's part of a window going away: its text handles and item list. Returns the DialogRecord the
    /// engine allocated (to free after the window), if any. Its controls go with [`Mac::kill_controls`].
    pub(crate) fn release_dialog(&mut self, win: u32) -> Option<u32> {
        let d = self.tr.dlg.dialogs.remove(&win)?;
        self.tr.dlg.hits.retain(|&(w, _)| w != win);
        for s in &d.list { if matches!(s.item, Item::StaticText | Item::EditText) { let _ = self.dispose_handle(s.handle); } }
        let _ = self.dispose_handle(d.items);
        d.storage
    }
}

fn done(log: String) -> Result<Done, Refusal> { Ok(Done { result: 0, log }) }

fn get_new_dialog<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let id = c.args.get(0) as u16 as i16;
    match m.new_dialog(id, c.args.get(1), c.args.get(2))? {
        Some(win) => Ok(Done { result: win, log: format!("[dialog] GetNewDialog {id} -> {win:#x} ({} items)", m.tr.dlg.dialogs[&win].list.len()) }),
        None => done(format!("[dialog] GetNewDialog {id}: no DLOG resource")),
    }
}

/// `NewDialog(dStorage, boundsRect, title, visible, procID, behind, goAwayFlag, refCon, items): DialogPtr` (Inside Macintosh I, "The
/// Dialog Manager"): a dialog from a program-built item list. `dStorage` NIL has the Dialog Manager allocate the `DialogRecord`; `behind`
/// -1 puts the window in front and NIL behind all. The item list is the program's own handle (it is not copied), and every item's
/// placeholder long is replaced with its handle as in `GetNewDialog`; `DisposeDialog` disposes the list.
fn new_dialog<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (storage, rect, title, visible, proc_id, behind) = (c.args.get(0), read_rect(&m.mem, c.args.get(1))?, read_pstring(&m.mem, c.args.get(2))?, c.args.get(3) as u8 != 0, c.args.get(4) as u16, c.args.get(5));
    let (go_away, ref_con, items) = (c.args.get(6) as u8 != 0, c.args.get(7), c.args.get(8));
    if items == 0 { return Err(Refusal("NewDialog: the item list is NIL".into())); }
    let def = Def { what: "NewDialog".into(), rect, title, proc_id, go_away, ref_con, ditl: 0, show: visible, items: Some(items) };
    let win = m.build_dialog(def, storage, behind)?;
    Ok(Done { result: win, log: format!("[dialog] NewDialog -> {win:#x} ({} items)", m.tr.dlg.dialogs[&win].list.len()) })
}

fn dispose_dialog<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let win = c.args.get(0);
    if !m.tr.dlg.dialogs.contains_key(&win) { return Err(Refusal(format!("DisposeDialog: {win:#x} is not a dialog"))); }
    m.dispose_window(win)?;
    done(format!("[dialog] DisposeDialog {win:#x}"))
}

pub(super) fn slot_of<U: GuestUi>(m: &Mac<U>, dlg: u32, n: i16, what: &str) -> Result<Slot, Refusal> {
    let d = m.tr.dlg.dialogs.get(&dlg).ok_or_else(|| Refusal(format!("{what}: {dlg:#x} is not a dialog")))?;
    d.list.get((n as isize - 1).max(-1) as usize).filter(|_| n >= 1).cloned().ok_or_else(|| Refusal(format!("{what}: dialog {dlg:#x} has no item {n} (it has {})", d.list.len())))
}

/// `GetDialogItem(dialog, itemNo, VAR itemType, VAR item, VAR box)`.
fn get_dialog_item<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (dlg, n) = (c.args.get(0), c.args.get(1) as i16);
    let s = slot_of(m, dlg, n, "GetDialogItem")?;
    let e = Refusal::mem("GetDialogItem result");
    m.mem.write_u16(c.args.get(2), u16::from(s.raw_type)).map_err(&e)?;
    m.mem.write_u32(c.args.get(3), s.handle).map_err(&e)?;
    for (i, v) in [s.rect.top, s.rect.left, s.rect.bottom, s.rect.right].into_iter().enumerate() { m.mem.write_u16(c.args.get(4) + 2 * i as u32, v as u16).map_err(&e)?; }
    done(format!("[dialog] GetDialogItem {dlg:#x} #{n} -> type {} handle {:#x}", s.raw_type, s.handle))
}

/// `SetDialogItemText(item, text)`: a text item's handle, or a button's / check box's control (its title).
fn set_dialog_item_text<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (h, t) = (c.args.get(0), read_pstring(&m.mem, c.args.get(1))?);
    if m.tr.procs.contains_key(&h) { m.set_control_title(h, &t)?; }
    else if let Some((win, idx)) = m.text_slot(h) { m.set_handle_text(h, &t)?; m.sync_text(win, idx)?; }
    else { return Err(Refusal(format!("SetDialogItemText: {h:#x} is not a dialog item handle"))); }
    done(format!("[dialog] SetDialogItemText {h:#x} \"{t}\""))
}

/// `GetDialogItemText(item, VAR text)`.
fn get_dialog_item_text<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let h = c.args.get(0);
    let t = if m.tr.procs.contains_key(&h) { m.control_facts(h)?.title } else if m.text_slot(h).is_some() { m.handle_text(h)? }
        else { return Err(Refusal(format!("GetDialogItemText: {h:#x} is not a dialog item handle"))); };
    let bytes: Vec<u8> = t.chars().take(255).map(|c| c as u8).collect();
    m.mem.write_u8(c.args.get(1), bytes.len() as u8).map_err(Refusal::mem("text result"))?;
    m.mem.write_bytes(c.args.get(1) + 1, &bytes).map_err(Refusal::mem("text result"))?;
    done(format!("[dialog] GetDialogItemText {h:#x} -> \"{t}\""))
}

/// `ModalDialog(filterProc, VAR itemHit)`: return a queued hit of the frontmost dialog, else block until one arrives.
fn modal_dialog<U: GuestUi>(m: &mut Mac<U>, cpu: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (filter, at) = (c.args.get(0), c.args.get(1));
    let win = m.front_dialog().ok_or_else(|| Refusal("ModalDialog: no dialog is open".into()))?;
    if m.update_modal_dialog(cpu, win)? { return done(format!("[dialog] ModalDialog {win:#x}: the dialog's user items are drawn first, then the call runs again")); }
    if filter != 0 { m.note_refusal("ModalDialog", format!("filterProc {filter:#x} is a guest callback and is not called; the standard filtering applies")); }
    if let Some(item) = m.take_hit(win) {
        m.mem.write_u16(at, item as u16).map_err(Refusal::mem("itemHit"))?;
        m.tr.dlg.modal = None;
        return done(format!("[dialog] ModalDialog {win:#x} -> itemHit {item}"));
    }
    m.tr.dlg.modal = Some(at);
    cpu.block();
    done(format!("[dialog] ModalDialog {win:#x} blocks"))
}

impl<U: GuestUi> Mac<U> {
    /// Replace `^0`..`^3` in a static text with `ParamText`'s strings (the Dialog Manager does this when it draws the item; the
    /// engine does it when the item is made, so a `ParamText` BEFORE `GetNewDialog`/`Alert` is honoured and one after is not).
    pub(crate) fn substitute_params(&self, text: &str) -> String {
        let mut out = String::new();
        let mut it = text.chars().peekable();
        while let Some(c) = it.next() {
            match (c, it.peek().and_then(|d| d.to_digit(10))) {
                ('^', Some(n)) if n < 4 => { it.next(); out.extend(self.tr.dlg.params[n as usize].iter().map(|&b| char::from(b))); }
                _ => out.push(c),
            }
        }
        out
    }
}

/// `IsDialogEvent(theEvent): BOOLEAN` (Inside Macintosh I, "The Dialog Manager"): is this event one `DialogSelect` should handle? An
/// activate or update event for a dialog window; a mouse-down in a dialog window's content; any key or auto-key while a dialog is
/// the front window. Everything else (and every event while no dialog exists) is not a dialog event.
fn is_dialog_event<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let e = Refusal::mem("EventRecord");
    let at = c.args.get(0);
    let (what, message) = (m.mem.read_u16(at).map_err(&e)?, m.mem.read_u32(at + 2).map_err(&e)?);
    let (v, h) = (m.mem.read_i16(at + 10).map_err(&e)?, m.mem.read_i16(at + 12).map_err(&e)?);
    let is_dialog = |w: u32| m.tr.dlg.dialogs.contains_key(&w);
    let hit = match what {
        6 | 8 => is_dialog(message),
        1 => { let (part, win) = m.find_window(v, h); part == crate::toolbox::IN_CONTENT && is_dialog(win) }
        3 | 5 => m.z.first().is_some_and(|&w| is_dialog(w)),
        _ => false,
    };
    Ok(Done { result: u32::from(hit), log: format!("[dialog] IsDialogEvent what {what} -> {hit}") })
}

/// `DialogSelect(theEvent, VAR theDialog, VAR itemHit): BOOLEAN`: the modeless counterpart of `ModalDialog`. When the user has hit an
/// item of a dialog since the last call (a native control's intent was turned into an item hit, see the module docs) the oldest
/// hit is returned: TRUE, with `theDialog` and `itemHit` set. Otherwise FALSE and both are left alone. What a real
/// `DialogSelect` also does with an event (drawing the items on an update, the text-edit field's key handling) the host's native
/// controls already did, so an update or activate event for a dialog window is simply FALSE here.
fn dialog_select<U: GuestUi>(m: &mut Mac<U>, cpu: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let e = Refusal::mem("EventRecord");
    let (what, message) = (m.mem.read_u16(c.args.get(0)).map_err(&e)?, m.mem.read_u32(c.args.get(0) + 2).map_err(&e)?);
    if what == crate::events::UPDATE_EVT && m.tr.dlg.dialogs.contains_key(&message) {
        m.update_dialog_event(cpu, message)?;
        return Ok(Done { result: 0, log: format!("[dialog] DialogSelect -> FALSE (update event: {message:#x} is drawn)") });
    }
    let (dlg_out, item_out) = (c.args.get(1), c.args.get(2));
    let Some((dlg, item)) = m.tr.dlg.hits.pop_front() else { return Ok(Done { result: 0, log: "[dialog] DialogSelect -> FALSE (no item hit pending)".into() }) };
    m.mem.write_u32(dlg_out, dlg).map_err(Refusal::mem("DialogSelect theDialog"))?;
    m.mem.write_u16(item_out, item as u16).map_err(Refusal::mem("DialogSelect itemHit"))?;
    Ok(Done { result: 1, log: format!("[dialog] DialogSelect -> TRUE, dialog {dlg:#x} item {item}") })
}

/// `ParamText(param0, param1, param2, param3)`: four Str255 pointers; a NIL pointer leaves that string unchanged.
fn param_text<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    for i in 0..4 {
        let p = c.args.get(i);
        if p != 0 { m.tr.dlg.params[i] = crate::pascal::read_pstring_bytes(&m.mem, p)?; }
    }
    let shown: Vec<String> = m.tr.dlg.params.iter().map(|p| p.iter().map(|&b| char::from(b)).collect()).collect();
    Ok(Done { result: 0, log: format!("[dialog] ParamText {shown:?}") })
}

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    use Arg::{L, W};
    m.register(0xA98B, &[L, L, L, L], None, param_text::<U>)?;
    m.register(0xA97F, &[Arg::P], Some(Arg::B), is_dialog_event::<U>)?;
    m.register(0xA980, &[Arg::P, Arg::P, Arg::P], Some(Arg::B), dialog_select::<U>)?;
    m.register(0xA97C, &[W, L, L], Some(L), get_new_dialog::<U>)?;
    m.register(0xA97D, &[Arg::P, Arg::P, Arg::P, Arg::B, W, L, Arg::B, L, L], Some(L), new_dialog::<U>)?;
    m.register(0xA983, &[L], None, dispose_dialog::<U>)?;
    m.register(0xA98D, &[L, W, L, L, L], None, get_dialog_item::<U>)?;
    m.register(0xA98F, &[L, L], None, set_dialog_item_text::<U>)?;
    m.register(0xA990, &[L, L], None, get_dialog_item_text::<U>)?;
    m.register(0xA982, &[L], None, close_dialog::<U>)?;
    m.register(0xA97E, &[L, W, W, W], None, select_dialog_item_text::<U>)?;
    m.register(0xA991, &[L, L], None, modal_dialog::<U>)
}

/// `CloseDialog(theDialog)`: take the dialog's items and window away. (The real call leaves the DialogRecord the program supplied
/// for the program to free; the engine frees only a record it allocated itself.)
fn close_dialog<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let win = c.args.get(0);
    if !m.tr.dlg.dialogs.contains_key(&win) { return Err(Refusal(format!("CloseDialog: {win:#x} is not a dialog"))); }
    m.dispose_window(win)?;
    done(format!("[dialog] CloseDialog {win:#x}"))
}

/// `SelectDialogItemText(theDialog, itemNo, strtSel, endSel)`: make `itemNo` the edit field (the insertion point goes there).
/// The selection range itself is the host field's: it is recorded in the log, not applied.
fn select_dialog_item_text<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (win, item, start, end) = (c.args.get(0), c.args.get(1) as u16 as i16, c.args.get(2) as u16 as i16, c.args.get(3) as u16 as i16);
    let idx = usize::try_from(i32::from(item) - 1).ok().filter(|&i| m.tr.dlg.dialogs.get(&win).is_some_and(|d| d.list.get(i).is_some_and(|s| s.item == Item::EditText)));
    let Some(idx) = idx else { return Err(Refusal(format!("SelectDialogItemText: item {item} of {win:#x} is not an editText item"))) };
    m.mem.write_u16(win + DR_EDIT_FIELD, idx as u16).map_err(Refusal::mem("editField"))?;
    done(format!("[dialog] SelectDialogItemText {win:#x} item {item} [{start},{end}) (the host field keeps its own selection)"))
}
