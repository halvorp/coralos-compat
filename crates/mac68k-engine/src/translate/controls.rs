//! The Control Manager as a toolkit table (ruling 17, piece 3): `procID` -> role, the attribute map into the real
//! `ControlRecord` (Inside Macintosh: Macintosh Toolbox Essentials, "Control Manager"), the part geometry and the traps.
//!
//! **The record is real guest memory** (296 bytes: `nextControl` 0, `contrlOwner` 4, `contrlRect` 8, `contrlVis` 16,
//! `contrlHilite` 17, `contrlValue` 18, `contrlMin` 20, `contrlMax` 22, `contrlDefProc` 24, `contrlData` 28, `contrlAction`
//! 32, `contrlRfCon` 36, `contrlTitle` 40) linked into the owner window's `controlList` (WindowRecord offset 140), so a
//! program that walks the list or reads `(*ctl)->contrlValue` sees the truth. The procID is not in the record (a CDEF
//! handle is), so [`super::Translate::procs`] keeps it; `contrlDefProc` is NIL: the behaviour is the table row, and a
//! program that installs its own CDEF is outside this table (its control would be a content plane).
//!
//! **Intents become the exact guest input** (the original OS's trace): the user's complete click on a push button is a
//! `mouseDown` in the control's rect (global coordinates, from the control's own rect) and the part code `TrackControl`
//! then returns (`inButton` 10); a check box click is the same with `inCheckBox` 11 and the VALUE UNTOUCHED (the Control
//! Manager never flips it: the program does, with `SetControlValue`); a scroll bar drag is a `mouseDown` on the thumb where it was
//! (`FindControl` says `inThumb` 129), and `TrackControl` leaves `contrlValue` at the dragged value before it returns `inThumb`. There is no mouse to follow, so `TrackControl` answers from the gesture the
//! host already completed; with none pending it answers the part under the start point (a click released at once).
//! Dialog controls report through the Dialog Manager instead ([`super::dialogs`]).
//!
//! Scroll bar geometry (classic CDEF 16): orientation from the rect (height >= width is vertical), a 16-pixel arrow box at
//! each end, a 16-pixel thumb moving over the rest in proportion to `(value - min) / (max - min)`. systemless (the oracle
//! this table was compared with) uses an 8-pixel thumb; the 16-pixel box is the System 7 scroll box.
use super::{guest_rect, Obj};
use crate::events::MOUSE_DOWN;
use crate::machine::{Call, Done, Mac};
use crate::pascal::{read_pstring, read_rect, Arg, Rect};
use crate::Refusal;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use guestui_abi::{CallbackToken, ControlKind, ControlModel, ControlValue, GuestUi, Intent};
use retro_m68k::Cpu;

pub const RECORD_SIZE: u32 = 296;
const C_NEXT: u32 = 0;
const C_OWNER: u32 = 4;
const C_RECT: u32 = 8;
const C_VIS: u32 = 16;
const C_HILITE: u32 = 17;
const C_VALUE: u32 = 18;
const C_MIN: u32 = 20;
const C_MAX: u32 = 22;
const C_REF_CON: u32 = 36;
const C_TITLE: u32 = 40;
/// `controlList` in a WindowRecord.
pub const WR_CONTROL_LIST: u32 = 140;
/// `contrlHilite` 255: the control is inactive (dimmed, takes no clicks). 254 is the same for hit-testing.
const INACTIVE: u8 = 0xFE;

pub const IN_BUTTON: u16 = 10;
pub const IN_CHECK_BOX: u16 = 11;
pub const IN_UP_BUTTON: u16 = 20;
pub const IN_DOWN_BUTTON: u16 = 21;
pub const IN_PAGE_UP: u16 = 22;
pub const IN_PAGE_DOWN: u16 = 23;
pub const IN_THUMB: u16 = 129;

/// The roles the Control Manager's definition functions produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role { Button, Check, Radio, Scroll }

/// One row of the toolkit table.
pub struct Row { pub proc_id: u16, pub name: &'static str, pub role: Role, /** the part a complete gesture ends in */ pub part: u16 }

/// THE control table: `procID` (16 * CDEF id + variation; variation bit 8 `useWFont` is masked off) -> role.
pub const TABLE: &[Row] = &[
    Row { proc_id: 0, name: "pushButProc", role: Role::Button, part: IN_BUTTON },
    Row { proc_id: 1, name: "checkBoxProc", role: Role::Check, part: IN_CHECK_BOX },
    // A radio button is a CHOICE; the closed role vocabulary reserves it until a host consumer exists, so it has no kit part yet.
    Row { proc_id: 2, name: "radioButProc", role: Role::Radio, part: IN_CHECK_BOX },
    Row { proc_id: 16, name: "scrollBarProc", role: Role::Scroll, part: IN_THUMB },
];

pub fn row_of(proc_id: u16) -> Option<&'static Row> { let p = proc_id & !8; TABLE.iter().find(|r| r.proc_id == p) }

/// What the record says right now.
#[derive(Debug, Clone)]
pub struct Facts { pub owner: u32, pub rect: Rect, pub vis: bool, pub hilite: u8, pub value: i16, pub min: i16, pub max: i16, pub proc_id: u16, pub title: String }

/// The attribute map, as data over the record: facts -> the role's model. `None`: the role has no kit part yet.
fn model(row: &Row, f: &Facts, token: u32) -> Option<ControlModel> {
    let (w, h) = (i32::from(f.rect.right) - i32::from(f.rect.left), i32::from(f.rect.bottom) - i32::from(f.rect.top));
    let kind = match row.role {
        Role::Button => ControlKind::Button,
        Role::Check => ControlKind::Checkbox { checked: f.value != 0 },
        // The Control Manager has no "visible amount": the value ranges min..=max one unit at a time.
        Role::Scroll => ControlKind::ScrollBar { vertical: h >= w, total: (i32::from(f.max) - i32::from(f.min)).max(0) as u32 + 1, visible: 1, top: (i32::from(f.value) - i32::from(f.min)).max(0) as u32 },
        Role::Radio => return None,
    };
    let label = if row.role == Role::Scroll { String::new() } else { f.title.clone() };
    Some(ControlModel { kind, label, rect: guest_rect(f.rect), enabled: f.hilite < INACTIVE, token: CallbackToken(u64::from(token)) })
}

fn arrow_len(len: i32) -> i32 { len.clamp(1, 16) }

/// Where along its axis a scroll bar's thumb starts (local coordinate), and how long it is.
fn thumb_span(f: &Facts) -> (i32, i32) {
    let (w, h) = (i32::from(f.rect.right) - i32::from(f.rect.left), i32::from(f.rect.bottom) - i32::from(f.rect.top));
    let (start, len) = if h >= w { (i32::from(f.rect.top), h) } else { (i32::from(f.rect.left), w) };
    let arrow = arrow_len(len);
    let track = (len - 2 * arrow).max(1);
    let thumb = 16.min(track);
    let span = (i32::from(f.max) - i32::from(f.min)).max(1);
    let rel = (i32::from(f.value) - i32::from(f.min)).clamp(0, span);
    (start + arrow + rel * (track - thumb) / span, thumb)
}

/// The part under the local point `(v, h)`, 0 when it is outside the rect.
pub fn part_at(row: &Row, f: &Facts, v: i16, h: i16) -> u16 {
    let r = f.rect;
    if v < r.top || v >= r.bottom || h < r.left || h >= r.right { return 0; }
    match row.role {
        Role::Button => IN_BUTTON,
        Role::Check | Role::Radio => IN_CHECK_BOX,
        Role::Scroll => {
            let vertical = r.bottom - r.top >= r.right - r.left;
            let (start, end, at) = if vertical { (i32::from(r.top), i32::from(r.bottom), i32::from(v)) } else { (i32::from(r.left), i32::from(r.right), i32::from(h)) };
            let arrow = arrow_len(end - start);
            let (ts, tl) = thumb_span(f);
            if at < start + arrow { IN_UP_BUTTON } else if at >= end - arrow { IN_DOWN_BUTTON }
            else if at < ts { IN_PAGE_UP } else if at < ts + tl { IN_THUMB } else { IN_PAGE_DOWN }
        }
    }
}

/// Where a complete gesture on the control starts, in local coordinates: the thumb for a scroll bar, else the middle.
fn click_point(row: &Row, f: &Facts) -> (i16, i16) {
    let r = f.rect;
    let (cv, ch) = ((r.top + r.bottom) / 2, (r.left + r.right) / 2);
    if row.role != Role::Scroll { return (cv, ch); }
    let (ts, tl) = thumb_span(f);
    let mid = (ts + tl / 2) as i16;
    if r.bottom - r.top >= r.right - r.left { (mid, ch) } else { (cv, mid) }
}

/// The meaning of a role-level intent for a role: the part the original OS reports, and the value it leaves behind.
struct Gesture { part: u16, value: Option<i16> }

fn gesture(row: &Row, f: &Facts, intent: &Intent) -> Result<Gesture, Refusal> {
    match (row.role, intent) {
        (Role::Button, Intent::Activate) => Ok(Gesture { part: row.part, value: None }),
        (Role::Check | Role::Radio, Intent::Activate | Intent::SetValue(ControlValue::Checked(_))) => Ok(Gesture { part: row.part, value: None }),
        (Role::Scroll, Intent::ScrollTo(top) | Intent::Drag(ControlValue::Scroll { top, .. })) => {
            let v = (i32::from(f.min) + (*top).min(i32::MAX as u32) as i32).clamp(i32::from(f.min), i32::from(f.max).max(i32::from(f.min)));
            Ok(Gesture { part: row.part, value: Some(v as i16) })
        }
        (_, i) => Err(Refusal(format!("Intent {i:?} has no meaning for {} ({:?}); Press/Release/Drag gestures are not bridged, a complete click arrives as Activate", row.name, row.role))),
    }
}

impl<U: GuestUi> Mac<U> {
    fn control_rec(&self, ctl: u32, what: &str) -> Result<u32, Refusal> {
        if !self.tr.procs.contains_key(&ctl) { return Err(Refusal(format!("{what}: {ctl:#x} is not a control"))); }
        self.mm.block(ctl).map(|b| b.ptr).ok_or_else(|| Refusal(format!("{what}: {ctl:#x} is not a live handle")))
    }

    pub fn control_facts(&self, ctl: u32) -> Result<Facts, Refusal> {
        let p = self.control_rec(ctl, "control")?;
        let e = Refusal::mem("ControlRecord");
        Ok(Facts {
            owner: self.mem.read_u32(p + C_OWNER).map_err(&e)?, rect: read_rect(&self.mem, p + C_RECT)?,
            vis: self.mem.read_u8(p + C_VIS).map_err(&e)? != 0, hilite: self.mem.read_u8(p + C_HILITE).map_err(&e)?,
            value: self.mem.read_i16(p + C_VALUE).map_err(&e)?, min: self.mem.read_i16(p + C_MIN).map_err(&e)?, max: self.mem.read_i16(p + C_MAX).map_err(&e)?,
            proc_id: self.tr.procs[&ctl], title: read_pstring(&self.mem, p + C_TITLE)?,
        })
    }

    /// Re-derive the control's native model from its record and push the difference.
    pub(crate) fn sync_control(&mut self, ctl: u32) -> Result<(), Refusal> {
        let f = self.control_facts(ctl)?;
        let want = if f.vis { row_of(f.proc_id).and_then(|r| model(r, &f, ctl)) } else { None };
        self.sync_projection(f.owner, Obj::Control(ctl), want)
    }

    /// The controls of `win`, front of the list first. Bounded by the number of controls that exist, so a list the
    /// guest corrupted into a cycle ends.
    pub fn window_controls(&self, win: u32) -> Result<Vec<u32>, Refusal> {
        let (mut out, e) = (Vec::new(), Refusal::mem("controlList"));
        let mut c = self.mem.read_u32(win + WR_CONTROL_LIST).map_err(&e)?;
        while c != 0 && out.len() <= self.tr.procs.len() {
            out.push(c);
            c = self.mem.read_u32(self.control_rec(c, "controlList")? + C_NEXT).map_err(&e)?;
        }
        Ok(out)
    }

    pub(crate) fn project_controls(&mut self, win: u32) -> Result<(), Refusal> {
        for c in self.window_controls(win)?.into_iter().rev() { self.sync_control(c)?; }
        Ok(())
    }

    /// `NewControl`: build the record, link it at the head of the window's list, show it.
    pub fn create_control(&mut self, win: u32, rect: Rect, title: &str, visible: bool, value: i16, min: i16, max: i16, proc_id: u16, ref_con: u32) -> Result<u32, Refusal> {
        if !self.windows.contains_key(&win) { return Err(Refusal(format!("NewControl: {win:#x} is not an open window"))); }
        let e = Refusal::mem("ControlRecord");
        let ctl = self.mm.new_handle(&mut self.mem, &mut self.heap, RECORD_SIZE, true).map_err(|e| Refusal(format!("NewControl: no memory for a ControlRecord (memFullErr {e})")))?;
        let p = self.mm.block(ctl).map(|b| b.ptr).ok_or_else(|| Refusal("NewControl: the new handle has no block".into()))?;
        let head = self.mem.read_u32(win + WR_CONTROL_LIST).map_err(&e)?;
        let t = title.chars().take(255).map(|c| c as u8).collect::<Vec<u8>>();
        self.mem.write_u32(p + C_NEXT, head).map_err(&e)?;
        self.mem.write_u32(p + C_OWNER, win).map_err(&e)?;
        for (i, v) in [rect.top, rect.left, rect.bottom, rect.right].into_iter().enumerate() { self.mem.write_u16(p + C_RECT + 2 * i as u32, v as u16).map_err(&e)?; }
        self.mem.write_u8(p + C_VIS, visible as u8).map_err(&e)?;
        for (off, v) in [(C_VALUE, value), (C_MIN, min), (C_MAX, max)] { self.mem.write_u16(p + off, v as u16).map_err(&e)?; }
        self.mem.write_u32(p + C_REF_CON, ref_con).map_err(&e)?;
        self.mem.write_u8(p + C_TITLE, t.len() as u8).map_err(&e)?;
        self.mem.write_bytes(p + C_TITLE + 1, &t).map_err(&e)?;
        self.mem.write_u32(win + WR_CONTROL_LIST, ctl).map_err(&e)?;
        self.tr.procs.insert(ctl, proc_id);
        match row_of(proc_id) {
            None => self.log.push(format!("[control] procID {proc_id} has no row in the Control Manager table: the control exists in guest memory and is not shown")),
            Some(r) if r.role == Role::Radio => self.log.push(format!("[control] {} has no kit role yet (choice is reserved): the control exists in guest memory and is not shown", r.name)),
            Some(_) => {}
        }
        self.sync_control(ctl)?;
        Ok(ctl)
    }

    /// `DisposeControl`: unlink, retire its projection (unless the whole window is going), free the record.
    pub fn dispose_control(&mut self, ctl: u32, retire: bool) -> Result<(), Refusal> {
        let f = self.control_facts(ctl)?;
        let e = Refusal::mem("controlList");
        let next = self.mem.read_u32(self.control_rec(ctl, "DisposeControl")? + C_NEXT).map_err(&e)?;
        if self.mem.read_u32(f.owner + WR_CONTROL_LIST).map_err(&e)? == ctl {
            self.mem.write_u32(f.owner + WR_CONTROL_LIST, next).map_err(&e)?;
        } else {
            for prev in self.window_controls(f.owner)? {
                let pp = self.control_rec(prev, "DisposeControl")?;
                if self.mem.read_u32(pp + C_NEXT).map_err(&e)? == ctl { self.mem.write_u32(pp + C_NEXT, next).map_err(&e)?; break; }
            }
        }
        if retire { self.sync_projection(f.owner, Obj::Control(ctl), None)?; }
        self.tr.procs.remove(&ctl);
        self.tr.tracks.remove(&ctl);
        self.dispose_handle(ctl).map_err(|e| Refusal(format!("DisposeControl: memory manager error {e}")))
    }

    pub fn kill_controls(&mut self, win: u32, retire: bool) -> Result<(), Refusal> {
        for c in self.window_controls(win)? { self.dispose_control(c, retire)?; }
        Ok(())
    }

    /// A native intent on a control: write the state the gesture leaves, then the input the original OS would have seen.
    pub(crate) fn control_intent(&mut self, ctl: u32, intent: Intent) -> Result<(), Refusal> {
        let f = self.control_facts(ctl)?;
        let row = row_of(f.proc_id).ok_or_else(|| Refusal(format!("Intent: procID {} has no row in the Control Manager table", f.proc_id)))?;
        if f.hilite >= INACTIVE { return Err(Refusal(format!("Intent: {} {ctl:#x} is inactive (contrlHilite {})", row.name, f.hilite))); }
        let g = gesture(row, &f, &intent)?;
        let (v, h) = click_point(row, &f);
        if let Some(item) = self.dialog_item_of(f.owner, ctl) { self.dialog_hit(f.owner, item); return self.restore_native(Obj::Control(ctl)); }
        let w = self.windows.get(&f.owner).ok_or_else(|| Refusal(format!("Intent: control {ctl:#x} has no open window")))?.rect;
        self.tr.tracks.insert(ctl, (g.part, g.value));
        self.mouse_event(MOUSE_DOWN, v.wrapping_add(w.top), h.wrapping_add(w.left), 0)
    }

    fn set_control_field(&mut self, ctl: u32, off: u32, v: i16) -> Result<(), Refusal> {
        let p = self.control_rec(ctl, "control")?;
        self.mem.write_u16(p + off, v as u16).map_err(Refusal::mem("ControlRecord"))
    }
}

// ---- the traps ----

fn done(log: String) -> Result<Done, Refusal> { Ok(Done { result: 0, log }) }
use crate::toolbox::point;

fn new_control<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let a = &c.args;
    let (rect, title) = (read_rect(&m.mem, a.get(1))?, read_pstring(&m.mem, a.get(2))?);
    let ctl = m.create_control(a.get(0), rect, &title, a.get(3) as u8 != 0, a.get(4) as i16, a.get(5) as i16, a.get(6) as i16, a.get(7) as u16, a.get(8))?;
    Ok(Done { result: ctl, log: format!("[control] NewControl \"{title}\" proc {} -> {ctl:#x}", a.get(7) as u16) })
}

/// `CNTL` resource: bounds (8), value (2), visible (1) + filler, max (2), min (2), procID (2), refCon (4), title (Pascal).
fn get_new_control<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let id = c.args.get(0) as u16 as i16;
    let Ok(h) = m.get_resource(*b"CNTL", id, false) else { return done(format!("[control] GetNewControl {id}: no CNTL resource")) };
    let ctl = m.control_from_cntl(h, id, c.args.get(1))?;
    Ok(Done { result: ctl, log: format!("[control] GetNewControl {id} -> {ctl:#x}") })
}

impl<U: GuestUi> Mac<U> {
    /// Build a control from the `CNTL` resource at handle `h` in window `win` (GetNewControl, and a DITL `resCtrl` item).
    pub(crate) fn control_from_cntl(&mut self, h: u32, id: i16, win: u32) -> Result<u32, Refusal> {
        let b = self.handle_bytes(h).unwrap_or_default();
        if b.len() < 23 || b.len() < 23 + b[22] as usize { return Err(Refusal(format!("GetNewControl {id}: CNTL resource is {} bytes, too short for its fields and title", b.len()))); }
        let be = |i: usize| i16::from_be_bytes([b[i], b[i + 1]]);
        let title: String = b[23..23 + b[22] as usize].iter().map(|&x| char::from(x)).collect();
        self.create_control(win, Rect { top: be(0), left: be(2), bottom: be(4), right: be(6) }, &title, b[10] != 0, be(8), be(14), be(12), be(16) as u16, u32::from_be_bytes([b[18], b[19], b[20], b[21]]))
    }
}

fn dispose_control<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    m.dispose_control(c.args.get(0), true)?;
    done(format!("[control] DisposeControl {:#x}", c.args.get(0)))
}
fn kill_controls<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    m.kill_controls(c.args.get(0), true)?;
    done(format!("[control] KillControls {:#x}", c.args.get(0)))
}

fn set_vis<U: GuestUi>(m: &mut Mac<U>, ctl: u32, vis: bool, name: &str) -> Result<Done, Refusal> {
    let p = m.control_rec(ctl, name)?;
    m.mem.write_u8(p + C_VIS, vis as u8).map_err(Refusal::mem("ControlRecord"))?;
    m.sync_control(ctl)?;
    done(format!("[control] {name} {ctl:#x}"))
}
fn show_control<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { set_vis(m, c.args.get(0), true, "ShowControl") }
fn hide_control<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { set_vis(m, c.args.get(0), false, "HideControl") }

/// Write one INTEGER field, keep `contrlValue` inside `min..=max` (the Control Manager clamps), resync.
fn put_field<U: GuestUi>(m: &mut Mac<U>, ctl: u32, off: u32, v: i16, name: &str) -> Result<Done, Refusal> {
    m.control_rec(ctl, name)?;
    m.set_control_field(ctl, off, v)?;
    let f = m.control_facts(ctl)?;
    if f.max >= f.min && (f.value < f.min || f.value > f.max) { m.set_control_field(ctl, C_VALUE, f.value.clamp(f.min, f.max))?; }
    m.sync_control(ctl)?;
    done(format!("[control] {name} {ctl:#x} {v}"))
}
fn set_value<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { put_field(m, c.args.get(0), C_VALUE, c.args.get(1) as i16, "SetControlValue") }
fn set_min<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { put_field(m, c.args.get(0), C_MIN, c.args.get(1) as i16, "SetControlMinimum") }
fn set_max<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { put_field(m, c.args.get(0), C_MAX, c.args.get(1) as i16, "SetControlMaximum") }

fn get_field<U: GuestUi>(m: &mut Mac<U>, ctl: u32, off: u32, name: &str) -> Result<Done, Refusal> {
    let p = m.control_rec(ctl, name)?;
    let v = m.mem.read_u16(p + off).map_err(Refusal::mem("ControlRecord"))?;
    Ok(Done { result: u32::from(v), log: format!("[control] {name} {ctl:#x} -> {}", v as i16) })
}
fn get_value<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { get_field(m, c.args.get(0), C_VALUE, "GetControlValue") }
fn get_min<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { get_field(m, c.args.get(0), C_MIN, "GetControlMinimum") }
fn get_max<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { get_field(m, c.args.get(0), C_MAX, "GetControlMaximum") }

fn hilite_control<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (ctl, state) = (c.args.get(0), c.args.get(1) as u8);
    let p = m.control_rec(ctl, "HiliteControl")?;
    m.mem.write_u8(p + C_HILITE, state).map_err(Refusal::mem("ControlRecord"))?;
    m.sync_control(ctl)?;
    done(format!("[control] HiliteControl {ctl:#x} {state}"))
}

impl<U: GuestUi> Mac<U> {
    /// Replace a control's title (SetControlTitle, and `SetDialogItemText` on a button item).
    pub(crate) fn set_control_title(&mut self, ctl: u32, title: &str) -> Result<(), Refusal> {
        let p = self.control_rec(ctl, "SetControlTitle")?;
        let t = title.chars().take(255).map(|c| c as u8).collect::<Vec<u8>>();
        let e = Refusal::mem("ControlRecord");
        self.mem.write_u8(p + C_TITLE, t.len() as u8).map_err(&e)?;
        self.mem.write_bytes(p + C_TITLE + 1, &t).map_err(&e)?;
        self.sync_control(ctl)
    }
}
fn set_title<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let t = read_pstring(&m.mem, c.args.get(1))?;
    m.set_control_title(c.args.get(0), &t)?;
    done(format!("[control] SetControlTitle {:#x} \"{t}\"", c.args.get(0)))
}
fn get_title<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let f = m.control_facts(c.args.get(0))?;
    let t: Vec<u8> = f.title.chars().map(|c| c as u8).collect();
    m.mem.write_u8(c.args.get(1), t.len() as u8).map_err(Refusal::mem("title"))?;
    m.mem.write_bytes(c.args.get(1) + 1, &t).map_err(Refusal::mem("title"))?;
    done(format!("[control] GetControlTitle {:#x} -> \"{}\"", c.args.get(0), f.title))
}

fn move_control<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (ctl, h, v) = (c.args.get(0), c.args.get(1) as i16, c.args.get(2) as i16);
    let r = m.control_facts(ctl)?.rect;
    set_rect(m, ctl, Rect { top: v, left: h, bottom: v + (r.bottom - r.top), right: h + (r.right - r.left) })?;
    done(format!("[control] MoveControl {ctl:#x} to ({v},{h})"))
}
fn size_control<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (ctl, w, h) = (c.args.get(0), c.args.get(1) as i16, c.args.get(2) as i16);
    let r = m.control_facts(ctl)?.rect;
    set_rect(m, ctl, Rect { top: r.top, left: r.left, bottom: r.top + h, right: r.left + w })?;
    done(format!("[control] SizeControl {ctl:#x} {w}x{h}"))
}
fn set_rect<U: GuestUi>(m: &mut Mac<U>, ctl: u32, r: Rect) -> Result<(), Refusal> {
    let p = m.control_rec(ctl, "control")?;
    for (i, v) in [r.top, r.left, r.bottom, r.right].into_iter().enumerate() { m.mem.write_u16(p + C_RECT + 2 * i as u32, v as u16).map_err(Refusal::mem("ControlRecord"))?; }
    m.sync_control(ctl)
}

/// `DrawControls` / `Draw1Control`: the kit draws projected controls; the call only checks its argument.
fn draw_controls<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let n = m.window_controls(c.args.get(0))?.len();
    done(format!("[control] DrawControls {:#x}: {n} control(s), drawn by the host's kit parts", c.args.get(0)))
}
fn draw_one<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    m.control_rec(c.args.get(0), "Draw1Control")?;
    done(format!("[control] Draw1Control {:#x}", c.args.get(0)))
}

/// `FindControl(thePoint, theWindow, VAR whichControl): INTEGER`: geometric over the window's list, local coordinates;
/// invisible and inactive controls are skipped (systemless' Control Manager does the same).
fn find_control<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let ((v, h), win) = (point(c.args.get(0)), c.args.get(1));
    let (mut part, mut hit) = (0u16, 0u32);
    for ctl in m.window_controls(win)? {
        let f = m.control_facts(ctl)?;
        let Some(row) = row_of(f.proc_id) else { continue };
        if !f.vis || f.hilite >= INACTIVE { continue; }
        let p = part_at(row, &f, v, h);
        if p != 0 { (part, hit) = (p, ctl); break; }
    }
    m.mem.write_u32(c.args.get(2), hit).map_err(Refusal::mem("FindControl result"))?;
    Ok(Done { result: u32::from(part), log: format!("[control] FindControl ({v},{h}) in {win:#x} -> part {part} control {hit:#x}") })
}

/// `TrackControl(theControl, startPt, actionProc): INTEGER`: see the module docs.
fn track_control<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (ctl, (v, h)) = (c.args.get(0), point(c.args.get(1)));
    let f = m.control_facts(ctl)?;
    let row = row_of(f.proc_id).ok_or_else(|| Refusal(format!("TrackControl: procID {} has no row in the Control Manager table", f.proc_id)))?;
    let under = if f.vis && f.hilite < INACTIVE { part_at(row, &f, v, h) } else { 0 };
    let pending = m.tr.tracks.remove(&ctl);
    let part = match pending { Some((p, _)) if under != 0 => p, _ => under };
    // a drag leaves its value in the record BEFORE TrackControl returns (the program reads it right after), but after FindControl saw the thumb where the user grabbed it
    if let (Some((_, Some(value))), true) = (pending, under != 0) { m.set_control_field(ctl, C_VALUE, value)?; }
    m.restore_native(Obj::Control(ctl))?; // the host showed the gesture's result optimistically; the guest's record is the truth
    m.sync_control(ctl)?;
    Ok(Done { result: u32::from(part), log: format!("[control] TrackControl {ctl:#x} ({v},{h}) -> part {part}{}", if pending.is_some() { " (native gesture)" } else { "" }) })
}

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    use Arg::{B, L, W};
    m.register(0xA954, &[L, L, L, B, W, W, W, W, L], Some(L), new_control::<U>)?;
    m.register(0xA9BE, &[W, L], Some(L), get_new_control::<U>)?;
    m.register(0xA955, &[L], None, dispose_control::<U>)?;
    m.register(0xA956, &[L], None, kill_controls::<U>)?;
    m.register(0xA957, &[L], None, show_control::<U>)?;
    m.register(0xA958, &[L], None, hide_control::<U>)?;
    m.register(0xA959, &[L, W, W], None, move_control::<U>)?;
    m.register(0xA95C, &[L, W, W], None, size_control::<U>)?;
    m.register(0xA95D, &[L, W], None, hilite_control::<U>)?;
    m.register(0xA95E, &[L, L], None, get_title::<U>)?;
    m.register(0xA95F, &[L, L], None, set_title::<U>)?;
    m.register(0xA960, &[L], Some(W), get_value::<U>)?;
    m.register(0xA961, &[L], Some(W), get_min::<U>)?;
    m.register(0xA962, &[L], Some(W), get_max::<U>)?;
    m.register(0xA963, &[L, W], None, set_value::<U>)?;
    m.register(0xA964, &[L, W], None, set_min::<U>)?;
    m.register(0xA965, &[L, W], None, set_max::<U>)?;
    m.register(0xA968, &[L, L, L], Some(W), track_control::<U>)?;
    m.register(0xA969, &[L], None, draw_controls::<U>)?;
    m.register(0xA96C, &[L, L, L], Some(W), find_control::<U>)?;
    m.register(0xA96D, &[L], None, draw_one::<U>)
}
