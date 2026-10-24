//! Appearance Manager entry points and the Control / Dialog / Window Manager Carbon accessors a Carbon program uses where a Classic one reached into the records
//! (`GetControlBounds` for `(*ctl)->contrlRect`, `GetDialogPort` for casting the dialog, `IsWindowVisible` for `window->visible` ...). The records are REAL guest memory (see
//! `mac68k-engine/src/translate/controls.rs` and `dialogs.rs`), so the accessors read them; whatever changes the host-side presentation (moving, sizing, showing, hiliting a
//! control; invalidating a rectangle; the arrow/I-beam cursors) runs the SAME Toolbox trap handler a 68k program reaches. What the Control Manager keeps outside the 68k records --
//! properties, the keyboard-focus control, the root/embedding hierarchy -- is plain bookkeeping here ([`CxState`]); the embedding hierarchy is RECORDED, not laid out (no
//! sub-control clipping or drawing order follows from it: the host draws the controls), and a root control is a real zeroed ControlRecord owned by its window.
//! `RegisterAppearanceClient` is a truthful success: the Appearance Manager is part of this system and registering a client has no state to keep.
//! Not done, left unbound so the program stops BY NAME: `HideWindow` (the host has no hide call, only open/close), `GetControlData`/`SetControlData` (tag-by-tag, as a program needs them),
//! `StandardAlert`, `DrawThemeFocusRect`/`DrawThemeEditTextFrame`/`UseThemeFont`/`SetThemeBackground` (theme chrome is drawn by the host, never by the guest).
use crate::clib::arg;
use crate::machine::MacPpc;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use guestui_abi::GuestUi;
use mac68k_engine::pascal::Rect;
use retro_ppc::Cpu;

/// Control Manager / Window Manager bookkeeping with no home in the 68k records.
#[derive(Default)]
pub struct CxState {
    /// (object, creator, tag) -> the property's bytes (`Set/GetControlProperty`, `Set/GetWindowProperty`).
    props: BTreeMap<(u32, u32, u32), Vec<u8>>,
    /// window -> its keyboard-focus control.
    focus: BTreeMap<u32, u32>,
    /// window -> its root control.
    root: BTreeMap<u32, u32>,
    /// control -> the container it was embedded in.
    parent: BTreeMap<u32, u32>,
    /// dialog -> its cancel item.
    cancel: BTreeMap<u32, i16>,
    /// (control, part, tag) -> the bytes `SetControlData` stored.
    cdata: BTreeMap<(u32, i16, u32), Vec<u8>>,
    /// control -> its view size (`SetControlViewSize`).
    view: BTreeMap<u32, i32>,
    /// (menu handle, item) -> command ID (`SetMenuItemCommandID`). Keyed by item NUMBER: a program that inserts or deletes items before setting IDs sets them again, as on Mac OS.
    cmds: BTreeMap<(u32, u16), u32>,
}

impl CxState {
    /// The command ID of a menu item (0 = none).
    pub(crate) fn command_id(&self, menu: u32, item: u16) -> u32 { self.cmds.get(&(menu, item)).copied().unwrap_or(0) }
}

const CONTROL_PROPERTY_INVALID: i32 = -5603;
const PROPERTY_NOT_FOUND: i32 = -5604;
const C_OWNER: u32 = 4;
const C_RECT: u32 = 8;
const C_VIS: u32 = 16;
const WR_VISIBLE: u32 = 110;
const WR_HILITED: u32 = 111;
const WR_NEXT: u32 = 144;
const WINDOW_LIST: u32 = 0x09D6;
const DR_ITEMS: u32 = 156;
const DR_TEXT_H: u32 = 160;
const DR_EDIT_FIELD: u32 = 164;
const DR_DEF_ITEM: u32 = 168;
/// `WindowRegionCode`s with a defined rectangle here: the content area and the port's global rectangle (the frame is the host's chrome).
const REGION_CONTENT: u32 = 33;
const REGION_GLOBAL_PORT: u32 = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CxFn {
    RegisterAppearanceClient, SetThemeCursor,
    GetControlBounds, SetControlBounds, GetControlOwner, IsControlVisible, SetControlVisibility, ActivateControl, DeactivateControl,
    GetControlProperty, SetControlProperty, GetWindowProperty, SetWindowProperty,
    GetKeyboardFocus, SetKeyboardFocus, ClearKeyboardFocus, CreateRootControl, GetRootControl, EmbedControl, FindControlUnderMouse,
    GetDialogPort, GetDialogWindow, SetPortDialogPort, CountDITL, GetDialogKeyboardFocusItem, GetDialogTextEditHandle, SetDialogDefaultItem, SetDialogCancelItem,
    SetMenuItemCommandID, GetMenuItemCommandID,
    GetControlData, SetControlData, SetControlFontStyle, SetControlViewSize, GetControlViewSize, GetDialogItemAsControl, SetThemeBackground,
    TestControl, KeyTranslate,
    IsWindowVisible, IsWindowHilited, GetWindowFromPort, GetNextWindow, GetWindowList, GetWindowBounds, InvalWindowRect, GetRegionBounds,
}

impl CxFn {
    pub fn by_name(name: &str) -> Option<CxFn> {
        use CxFn::*;
        Some(match name {
            "RegisterAppearanceClient" => RegisterAppearanceClient, "SetThemeCursor" => SetThemeCursor,
            "GetControlBounds" => GetControlBounds, "SetControlBounds" => SetControlBounds, "GetControlOwner" => GetControlOwner, "IsControlVisible" => IsControlVisible,
            "SetControlVisibility" => SetControlVisibility, "ActivateControl" => ActivateControl, "DeactivateControl" => DeactivateControl,
            "GetControlProperty" => GetControlProperty, "SetControlProperty" => SetControlProperty, "GetWindowProperty" => GetWindowProperty, "SetWindowProperty" => SetWindowProperty,
            "GetKeyboardFocus" => GetKeyboardFocus, "SetKeyboardFocus" => SetKeyboardFocus, "ClearKeyboardFocus" => ClearKeyboardFocus,
            "CreateRootControl" => CreateRootControl, "GetRootControl" => GetRootControl, "EmbedControl" => EmbedControl, "FindControlUnderMouse" => FindControlUnderMouse,
            "GetDialogPort" => GetDialogPort, "GetDialogWindow" => GetDialogWindow, "SetPortDialogPort" => SetPortDialogPort, "CountDITL" => CountDITL,
            "GetDialogKeyboardFocusItem" => GetDialogKeyboardFocusItem, "GetDialogTextEditHandle" => GetDialogTextEditHandle,
            "SetDialogDefaultItem" => SetDialogDefaultItem, "SetDialogCancelItem" => SetDialogCancelItem,
            "GetControlData" => GetControlData, "SetControlData" => SetControlData, "SetControlFontStyle" => SetControlFontStyle, "SetControlViewSize" => SetControlViewSize,
            "GetControlViewSize" => GetControlViewSize, "GetDialogItemAsControl" => GetDialogItemAsControl, "SetThemeBackground" => SetThemeBackground,
            "SetMenuItemCommandID" => SetMenuItemCommandID, "GetMenuItemCommandID" => GetMenuItemCommandID,
            "TestControl" => TestControl, "KeyTranslate" => KeyTranslate,
            "IsWindowVisible" => IsWindowVisible, "IsWindowHilited" => IsWindowHilited, "GetWindowFromPort" => GetWindowFromPort, "GetNextWindow" => GetNextWindow,
            "GetWindowList" => GetWindowList, "GetWindowBounds" => GetWindowBounds, "InvalWindowRect" => InvalWindowRect, "GetRegionBounds" => GetRegionBounds,
            _ => return None,
        })
    }
}

impl<U: GuestUi> MacPpc<U> {
    /// Run the Pascal Toolbox trap `name` (the handler a 68k program reaches) with `values`.
    fn cx_trap(&mut self, name: &str, values: &[u32]) -> Result<u32, String> {
        let op = mac68k_engine::mixed::trap_opcode(name).ok_or_else(|| format!("{name} is not a trap in the engine's tables"))?;
        self.mac.pascal_call(op, values).map(|r| r.value).map_err(|e| format!("{name}: {e:?}"))
    }
    fn cx_rec(&self, handle: u32, who: &str) -> Result<u32, String> {
        self.mac.mm.block(handle).map(|b| b.ptr).ok_or_else(|| format!("{who}: {handle:#x} is not a control or dialog handle"))
    }
    fn cx_u16(&self, at: u32, who: &str) -> Result<u16, String> { self.mac.mem.read_u16(at).map_err(|_| format!("{who}: {at:#x} leaves guest RAM")) }
    fn cx_u32(&self, at: u32, who: &str) -> Result<u32, String> { self.mac.mem.read_u32(at).map_err(|_| format!("{who}: {at:#x} leaves guest RAM")) }
    fn cx_put32(&mut self, at: u32, v: u32, who: &str) -> Result<(), String> { self.mac.mem.write_u32(at, v).map_err(|_| format!("{who}: the result pointer {at:#x} leaves guest RAM")) }
    fn cx_rect(&self, at: u32, who: &str) -> Result<Rect, String> { mac68k_engine::pascal::read_rect(&self.mac.mem, at).map_err(|e| format!("{who}: {}", e.0)) }
    fn cx_put_rect(&mut self, at: u32, r: Rect, who: &str) -> Result<(), String> {
        let m = &mut self.mac.mem;
        m.write_u16(at, r.top as u16).and_then(|_| m.write_u16(at + 2, r.left as u16)).and_then(|_| m.write_u16(at + 4, r.bottom as u16)).and_then(|_| m.write_u16(at + 6, r.right as u16))
            .map_err(|_| format!("{who}: the Rect at {at:#x} leaves guest RAM"))
    }
    fn cx_window(&self, w: u32, who: &str) -> Result<(), String> {
        if self.mac.windows.contains_key(&w) { Ok(()) } else { Err(format!("{who}: {w:#x} is not an open window")) }
    }

    pub(crate) fn cx_call(&mut self, f: CxFn, cpu: &mut Cpu) -> Result<(), String> {
        use CxFn::*;
        let name = format!("{f:?}");
        let n = name.as_str();
        let (a0, a1, a2, a3) = (arg(self, cpu, 0)?, arg(self, cpu, 1).unwrap_or(0), arg(self, cpu, 2).unwrap_or(0), arg(self, cpu, 3).unwrap_or(0));
        let r: u32 = match f {
            RegisterAppearanceClient => 0,
            // ThemeCursor: kThemeArrowCursor 0, IBeam 1, Cross 2, Plus 3, Watch 4 are the classic cursors (`CURS` 0 = InitCursor, 1..4 the system resources)
            SetThemeCursor => match a0 {
                0 => { self.cx_trap("InitCursor", &[])?; 0 }
                1..=4 => {
                    let h = self.cx_trap("GetCursor", &[a0])?;
                    if h == 0 { (-192i32) as u32 } else { let p = self.cx_rec(h, n)?; self.cx_trap("SetCursor", &[p])?; 0 }
                }
                c => return Err(format!("SetThemeCursor: theme cursor {c} is not implemented (arrow, I-beam, crosshair, plus and watch only)")),
            },
            // ---- controls: the record is guest memory
            GetControlBounds => { let p = self.cx_rec(a0, n)?; let r = self.cx_rect(p + C_RECT, n)?; self.cx_put_rect(a1, r, n)?; a1 }
            SetControlBounds => {
                let p = self.cx_rec(a0, n)?;
                let (old, new) = (self.cx_rect(p + C_RECT, n)?, self.cx_rect(a1, n)?);
                if (new.right - new.left, new.bottom - new.top) != (old.right - old.left, old.bottom - old.top) {
                    self.cx_trap("SizeControl", &[a0, u32::from((new.right - new.left) as u16), u32::from((new.bottom - new.top) as u16)])?;
                }
                // MoveControl(h, v) takes the new top-left in the owner's LOCAL coordinates, which is what contrlRect is in
                if (new.left, new.top) != (old.left, old.top) { self.cx_trap("MoveControl", &[a0, new.left as u16 as u32, new.top as u16 as u32])?; }
                0
            }
            GetControlOwner => { let p = self.cx_rec(a0, n)?; self.cx_u32(p + C_OWNER, n)? }
            IsControlVisible => { let p = self.cx_rec(a0, n)?; u32::from(self.mac.mem.read_u8(p + C_VIS).map_err(|_| format!("{n}: control record leaves guest RAM"))? != 0) }
            SetControlVisibility => { self.cx_trap(if a1 & 0xFF != 0 { "ShowControl" } else { "HideControl" }, &[a0])?; 0 }
            ActivateControl => { self.cx_trap("HiliteControl", &[a0, 0])?; 0 }
            DeactivateControl => { self.cx_trap("HiliteControl", &[a0, 255])?; 0 }
            // ---- properties: OSStatus; size in/out as the Controls.h / MacWindows.h contracts
            GetControlProperty | GetWindowProperty => {
                let (creator, tag, cap, actual_p, buf) = (a1, a2, a3, arg(self, cpu, 4)?, arg(self, cpu, 5)?);
                match self.cx.props.get(&(a0, creator, tag)).cloned() {
                    None => PROPERTY_NOT_FOUND as u32,
                    Some(v) => {
                        if actual_p != 0 { self.cx_put32(actual_p, v.len() as u32, n)?; }
                        if (v.len() as u32) > cap { CONTROL_PROPERTY_INVALID as u32 } else { self.put_bytes(buf, &v, n)?; 0 }
                    }
                }
            }
            SetControlProperty | SetWindowProperty => {
                let (creator, tag, size, buf) = (a1, a2, a3, arg(self, cpu, 4)?);
                let bytes = self.get_bytes(buf, size, n)?;
                self.cx.props.insert((a0, creator, tag), bytes);
                0
            }
            // ---- keyboard focus and the embedding hierarchy (recorded)
            GetKeyboardFocus => { let c = self.cx.focus.get(&a0).copied().unwrap_or(0); self.cx_put32(a1, c, n)?; 0 }
            SetKeyboardFocus => { self.cx_window(a0, n)?; if a1 == 0 { self.cx.focus.remove(&a0); } else { self.cx.focus.insert(a0, a1); } 0 }
            ClearKeyboardFocus => { self.cx.focus.remove(&a0); 0 }
            CreateRootControl => {
                self.cx_window(a0, n)?;
                if self.cx.root.contains_key(&a0) { return Err(format!("CreateRootControl: window {a0:#x} already has a root control (errRootAlreadyExists is not modelled)")); }
                let h = self.mac.mm.new_handle(&mut self.mac.mem, &mut self.mac.heap, mac68k_engine::translate::controls::RECORD_SIZE, true).map_err(|_| format!("{n}: out of memory"))?;
                let p = self.cx_rec(h, n)?;
                self.mac.mem.write_u32(p + C_OWNER, a0).map_err(|_| format!("{n}: control record leaves guest RAM"))?;
                self.mac.mem.write_u8(p + C_VIS, 0xFF).map_err(|_| format!("{n}: control record leaves guest RAM"))?;
                self.cx.root.insert(a0, h);
                self.cx_put32(a1, h, n)?;
                0
            }
            GetRootControl => match self.cx.root.get(&a0).copied() { Some(h) => { self.cx_put32(a1, h, n)?; 0 } None => { self.cx_put32(a1, 0, n)?; (-30599i32) as u32 /* errNoRootControl */ } },
            EmbedControl => { self.cx_rec(a0, n)?; self.cx_rec(a1, n)?; self.cx.parent.insert(a0, a1); 0 }
            // FindControlUnderMouse(Point inWhere, WindowRef, ControlPartCode *outPart): the Control Manager's own hit test, local coordinates
            FindControlUnderMouse => {
                self.cx_window(a1, n)?;
                let tmp = self.mac.mm.new_ptr(&mut self.mac.mem, &mut self.mac.heap, 4, true).map_err(|_| format!("{n}: out of memory"))?;
                let part = self.cx_trap("FindControl", &[a0, a1, tmp]);
                let ctl = self.mac.mem.read_u32(tmp).unwrap_or(0);
                let _ = self.mac.mm.dispose_ptr(&mut self.mac.heap, tmp);
                let part = part?;
                if a2 != 0 { self.mac.mem.write_u16(a2, part as u16).map_err(|_| format!("{n}: the part pointer {a2:#x} leaves guest RAM"))?; }
                ctl
            }
            // ---- dialogs: a DialogRecord begins with its WindowRecord, so the dialog IS its port and its window
            GetDialogPort | GetDialogWindow => a0,
            SetPortDialogPort => { self.cx_trap("SetPort", &[a0])?; 0 }
            CountDITL => { let h = self.cx_u32(a0 + DR_ITEMS, n)?; let p = self.cx_rec(h, n)?; u32::from(self.cx_u16(p, n)?.wrapping_add(1)) }
            // editField is the 0-based text item (-1 = none); the Carbon call answers a 1-based index, 0 = none
            GetDialogKeyboardFocusItem => u32::from((self.cx_u16(a0 + DR_EDIT_FIELD, n)? as i16 + 1) as u16),
            GetDialogTextEditHandle => self.cx_u32(a0 + DR_TEXT_H, n)?,
            SetDialogDefaultItem => { self.mac.mem.write_u16(a0 + DR_DEF_ITEM, a1 as u16).map_err(|_| format!("{n}: dialog record leaves guest RAM"))?; 0 }
            SetDialogCancelItem => { self.cx.cancel.insert(a0, a1 as u16 as i16); 0 }
            // ---- menus: the command ID a chosen item carries in kEventCommandProcess; paramErr (-50) for a menu or item that does not exist
            SetMenuItemCommandID | GetMenuItemCommandID => {
                let item = a1 as u16;
                let count = if self.mac.mm.block(a0).is_some() { self.cx_trap("CountMItems", &[a0])? as u16 } else { 0 };
                if item == 0 || item > count { cpu.gpr[3] = (-50i32) as u32; return Ok(()); }
                if f == SetMenuItemCommandID { if a2 == 0 { self.cx.cmds.remove(&(a0, item)); } else { self.cx.cmds.insert((a0, item), a2); } } else { let v = self.cx.command_id(a0, item); self.cx_put32(a2, v, n)?; }
                0
            }
            KeyTranslate => return self.key_translate(cpu),
            // TestControl(ctl, Point local): the part under the point, 0 if the control is hidden, inactive (hilite 255) or the point is outside; the part comes from the SAME geometry
            // FindControl uses. A control covered by a sibling at that point is refused by name (its part would need the CDEF).
            TestControl => {
                let p = self.cx_rec(a0, n)?;
                let (vis, hilite) = (self.mac.mem.read_u8(p + C_VIS).unwrap_or(0), self.mac.mem.read_u8(p + 17).unwrap_or(0));
                let r = self.cx_rect(p + C_RECT, n)?;
                let (v, h) = ((a1 >> 16) as i16, a1 as i16);
                if vis == 0 || hilite == 255 || v < r.top || v >= r.bottom || h < r.left || h >= r.right { 0 } else {
                    let owner = self.cx_u32(p + C_OWNER, n)?;
                    let tmp = self.mac.mm.new_ptr(&mut self.mac.mem, &mut self.mac.heap, 4, true).map_err(|_| format!("{n}: out of memory"))?;
                    let part = self.cx_trap("FindControl", &[a1, owner, tmp]);
                    let found = self.mac.mem.read_u32(tmp).unwrap_or(0);
                    let _ = self.mac.mm.dispose_ptr(&mut self.mac.heap, tmp);
                    let part = part?;
                    if found != a0 { return Err(format!("TestControl: control {a0:#x} is covered by {found:#x} at ({v},{h}); the part of a covered control is not computed")); }
                    part
                }
            }
            // ---- control data (spec: macos9-specs/ControlsLib/GetControlData, SetControlData, SetControlFontStyle, SetControlViewSize)
            SetControlData | GetControlData | SetControlFontStyle => return self.cx_control_data(f, cpu, [a0, a1, a2, a3]),
            SetControlViewSize => { if self.mac.tr.procs.contains_key(&a0) { self.cx.view.insert(a0, a1 as i32); } 0 }
            GetControlViewSize => self.cx.view.get(&a0).copied().unwrap_or(0) as u32,
            // GetDialogItemAsControl(dialog, itemNo, &control): -50 for a NULL/unknown dialog or an item outside the list; the help item and (without a root control) a non-control item are
            // errItemNotControl (-30596); the item's handle is the ControlRef (spec: macos9-specs/AppearanceLib/GetDialogItemAsControl)
            GetDialogItemAsControl => {
                let item = a1 as u16 as i16;
                match (item >= 1).then(|| self.mac.dialog_item_slot(a0, item)).flatten() {
                    None => (-50i32) as u32,
                    Some((ty, handle)) => {
                        let t = ty & 0x7F;
                        if t == 1 || (t & 4 == 0 && !self.cx.root.contains_key(&a0)) { (-30596i32) as u32 } else { self.cx_put32(a2, handle, n)?; 0 }
                    }
                }
            }
            // SetThemeBackground(brush, depth, isColorDevice): black (-1) and white (-2) set the port's background colour and the library's solid background pattern; brushes 1..48 belong
            // to the theme (below); anything else is themeInvalidBrushErr (-30560) with no port change
            SetThemeBackground => {
                let brush = a0 as u16 as i16;
                match brush {
                    -1 | -2 => {
                        let c = if brush == -1 { 0u16 } else { 0xFFFF };
                        let tmp = self.mac.mm.new_ptr(&mut self.mac.mem, &mut self.mac.heap, 16, true).map_err(|_| format!("{n}: out of memory"))?;
                        for i in 0..3 { let _ = self.mac.mem.write_u16(tmp + 2 * i, c); }
                        // the "solid" background pattern fills with the background colour: an all-zero (white) pattern
                        let r = self.cx_trap("RGBBackColor", &[tmp]).and_then(|_| self.cx_trap("BackPat", &[tmp + 8]));
                        let _ = self.mac.mm.dispose_ptr(&mut self.mac.heap, tmp);
                        r?;
                        0
                    }
                    1..=48 => { self.mac.log.push(format!("[theme] SetThemeBackground brush {brush}: the host theme paints window and dialog backgrounds; the port's background colour is unchanged")); 0 }
                    _ => (-30560i32) as u32,
                }
            }
            // ---- windows
            IsWindowVisible => { self.cx_window(a0, n)?; u32::from(self.mac.mem.read_u8(a0 + WR_VISIBLE).unwrap_or(0) != 0) }
            IsWindowHilited => { self.cx_window(a0, n)?; u32::from(self.mac.mem.read_u8(a0 + WR_HILITED).unwrap_or(0) != 0) }
            GetWindowFromPort => if self.mac.windows.contains_key(&a0) { a0 } else { 0 },
            GetNextWindow => { self.cx_window(a0, n)?; self.cx_u32(a0 + WR_NEXT, n)? }
            GetWindowList => self.cx_u32(WINDOW_LIST, n)?,
            // GetWindowBounds(window, regionCode, Rect *): GLOBAL coordinates; the frame (title bar, borders) belongs to the host's window chrome, so only the content area is known
            GetWindowBounds => {
                self.cx_window(a0, n)?;
                match a1 & 0xFFFF {
                    REGION_CONTENT | REGION_GLOBAL_PORT => { let r = self.mac.windows[&a0].rect; self.cx_put_rect(a2, r, n)?; 0 }
                    c => return Err(format!("GetWindowBounds: window region {c} is not implemented (the content area, 33, and the port's global rectangle, 40, only: the frame is the host's chrome)")),
                }
            }
            // InvalWindowRect(window, rect): InvalRect in that window's port, thePort restored after
            InvalWindowRect => {
                self.cx_window(a0, n)?;
                let slot = self.mac.qd_globals;
                if slot == 0 { return Err(String::from("InvalWindowRect: InitGraf has not run")); }
                let old = self.cx_u32(slot, n)?;
                self.cx_trap("SetPort", &[a0])?;
                let r = self.cx_trap("InvalRect", &[a1]);
                self.cx_trap("SetPort", &[old])?;
                r?;
                0
            }
            // GetRegionBounds(RgnHandle, Rect*): rgnBBox is at offset 2 of the region record
            GetRegionBounds => { let p = self.cx_rec(a0, n)?; let r = self.cx_rect(p + 2, n)?; self.cx_put_rect(a1, r, n)?; a1 }
        };
        cpu.gpr[3] = r;
        Ok(())
    }
}

const ERR_MESSAGE_NOT_SUPPORTED: i32 = -30580;
const ERR_DATA_NOT_SUPPORTED: i32 = -30581;
const ERR_DATA_SIZE_MISMATCH: i32 = -30591;

impl<U: GuestUi> MacPpc<U> {
    /// Appearance controls (CDEF ids 3 and up, except the classic pop-up CDEF 63) advertise the data-access feature (bit 0x40); the classic CDEFs 0..2 and the classic pop-up do not.
    fn cx_data_access(&self, ctl: u32) -> bool { self.mac.tr.procs.get(&ctl).is_some_and(|&p| { let id = p >> 4; id >= 3 && id != 63 }) }

    /// The tags this engine keeps for a control: (fixed size or 0 = any, may the host see it). Anything else is errDataNotSupported (-30581), as the spec's native-port note says.
    fn cx_tag_size(tag: u32) -> Option<u32> {
        Some(match &tag.to_be_bytes() { b"font" => 24, b"dflt" | b"cncl" | b"inde" => 1, b"size" => 2, b"text" => 0, _ => return None })
    }

    pub(crate) fn cx_control_data(&mut self, f: CxFn, cpu: &mut Cpu, a: [u32; 4]) -> Result<(), String> {
        let n = format!("{f:?}");
        let r: i32 = (|| -> Result<i32, String> {
            let ctl = a[0];
            if ctl == 0 || !self.mac.tr.procs.contains_key(&ctl) { return Ok(-50); }
            if f == CxFn::SetControlFontStyle {
                if a[1] == 0 { return Ok(-50); }
                if !self.cx_data_access(ctl) { return Ok(ERR_MESSAGE_NOT_SUPPORTED); }
                let rec = self.get_bytes(a[1], 24, &n)?;
                self.cx.cdata.insert((ctl, 0, u32::from_be_bytes(*b"font")), rec);
                return Ok(0);
            }
            if !self.cx_data_access(ctl) { return Ok(ERR_MESSAGE_NOT_SUPPORTED); }
            let part = a[1] as u16 as i16;
            let tag = a[2];
            let Some(fixed) = Self::cx_tag_size(tag) else {
                self.mac.log.push(format!("[control] {n} tag '{}': not a tag this engine keeps (errDataNotSupported)", String::from_utf8_lossy(&tag.to_be_bytes())));
                return Ok(ERR_DATA_NOT_SUPPORTED);
            };
            if f == CxFn::SetControlData {
                let (size, data) = (a[3], arg(self, cpu, 4)?);
                if fixed != 0 && size != fixed { return Ok(ERR_DATA_SIZE_MISMATCH); }
                let bytes = self.get_bytes(data, size, &n)?;
                if &tag.to_be_bytes() == b"text" {
                    // a text control's title is its text: the same handler SetControlTitle runs, which updates the native control
                    let t = &bytes[..bytes.len().min(255)];
                    let tmp = self.mac.mm.new_ptr(&mut self.mac.mem, &mut self.mac.heap, 260, true).map_err(|_| format!("{n}: out of memory"))?;
                    let mut b = alloc::vec![t.len() as u8]; b.extend_from_slice(t);
                    let _ = self.mac.mem.write_bytes(tmp, &b);
                    let r = self.cx_trap("SetControlTitle", &[ctl, tmp]);
                    let _ = self.mac.mm.dispose_ptr(&mut self.mac.heap, tmp);
                    r?;
                }
                self.cx.cdata.insert((ctl, part, tag), bytes);
                return Ok(0);
            }
            // GetControlData(ctl, part, tag, size, buffer, &actual)
            let (size, buf, actual) = (a[3], arg(self, cpu, 4)?, arg(self, cpu, 5)?);
            let stored = match self.cx.cdata.get(&(ctl, part, tag)) {
                Some(v) => v.clone(),
                None if &tag.to_be_bytes() == b"text" => { let p = self.cx_rec(ctl, &n)?; let l = self.mac.mem.read_u8(p + 40).unwrap_or(0); self.get_bytes(p + 41, u32::from(l), &n)? }
                None => return Ok(ERR_DATA_NOT_SUPPORTED),
            };
            if fixed != 0 && size < fixed { return Ok(ERR_DATA_SIZE_MISMATCH); }
            let take = stored.len().min(size as usize);
            if buf != 0 { self.put_bytes(buf, &stored[..take], &n)?; }
            if actual != 0 { self.mac.mem.write_u32(actual, stored.len() as u32).map_err(|_| format!("{n}: outActualSize leaves guest RAM"))?; }
            Ok(0)
        })()?;
        cpu.gpr[3] = r as u32;
        Ok(())
    }
}
