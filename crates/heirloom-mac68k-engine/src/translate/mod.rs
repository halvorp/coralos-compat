//! The Mac side of the translation layer (ruling 17, pieces 2, 3 and 8): the Toolbox's own UI objects, shown as semantic
//! controls through the ONE seam (`heirloom-ui-abi`), and native intents turned back into the exact input the original OS
//! produced. Everything is keyed on the TOOLKIT, never on an application:
//!
//! * [`controls`]: ONE `procID` (the CDEF) table plus ONE role x intent row table (the shared `heirloom-translate` rows), with the attribute map into the real `ControlRecord` in guest
//!   memory and the part-code geometry (`FindControl`/`TrackControl`).
//! * [`dialogs`]: ONE table, DITL item type -> role; push buttons, check boxes and `resCtrl` items reuse the control
//!   table, static and editable text are fields of their own.
//! * [`lists`]: the List Manager (`_Pack0`): a real `ListRec`, the selector dispatch, and the standard text LDEF shown as a native list.
//! * [`fallback`]: a control the kit has no part for is drawn into the content plane, never dropped.
//! * [`input`]: plain pointer and key input for content (a plane is fully usable before it has any binding).
//! * this file: the generic mapper both tables feed: project / update / retire an object, and the single stale-intent
//!   check. No role is ever inferred from what pixels look like (HEIRLOOM-ROLES-REQUIRE-A-CONTRACT): a role exists because a
//!   Toolbox record or resource says so.
//!
//! The guest's memory stays authoritative. A projected object is a VIEW of its record: after any Toolbox call that
//! changes the record the model is recomputed from guest memory and only the difference is pushed (a changed value is
//! `set_control`, a moved rect `set_control_rect`, a changed title or shape a new incarnation: remove + add, which is a new
//! generation, so the old ref is refused by name).
pub mod alerts;
pub mod controls;
pub mod dialog_items;
pub mod dialogs;
pub mod fallback;
pub mod input;
pub mod lists;
pub mod useritems;
pub mod filterproc;

use crate::events::Delivered;
use crate::machine::Mac;
use crate::pascal::Rect;
use crate::Refusal;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use heirloom_ui_abi::{CallbackToken, ControlModel, ControlValue, GuestUi, Intent, UiObjectRef};
use heirloom_translate::{Projected, Projections};

/// A Mac `Rect` (top, left, bottom, right) as the seam's rectangle (left, top, width, height).
pub(crate) fn guest_rect(r: Rect) -> heirloom_ui_abi::GuestRect {
    heirloom_ui_abi::GuestRect { left: i32::from(r.left), top: i32::from(r.top), width: i32::from(r.right) - i32::from(r.left), height: i32::from(r.bottom) - i32::from(r.top) }
}

/// What a projected token stands for in the guest. The token is always the guest object's own address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Obj {
    /// A `ControlHandle` (a Control Manager control, or a dialog's button / check box / `resCtrl` item).
    Control(u32),
    /// The `Handle` of a dialog's static or editable text item.
    Text(u32),
    /// A live single-line TextEdit `TEHandle` projected as an editable native field.
    TextEdit(u32),
    /// A declarative `plane_chrome` bound edit field (`PaletteRuleKind::EditField`) projected as an editable native field.
    BoundField(u32),
    /// The `ListHandle` of a List Manager list.
    List(u32),
}

impl Obj {
    pub fn handle(self) -> u32 { match self { Obj::Control(h) | Obj::Text(h) | Obj::TextEdit(h) | Obj::BoundField(h) | Obj::List(h) => h } }
}

/// What the engine adds to the shared registry's entry: which guest object, in which guest window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Meta { pub win: u32, pub obj: Obj }

#[derive(Default)]
pub struct Translate {
    /// The generic mapper's registry (heirloom-translate): token (the guest object's address) -> live projection.
    live: Projections<Meta>,
    /// `ControlHandle` -> its `procID` (a ControlRecord holds only a CDEF handle, so the id lives host-side).
    pub procs: BTreeMap<u32, u16>,
    /// A pop-up control's MENU resource ID, its MenuHandle (`PopupPrivateData.mHandle`), and the initial `titleWidth`
    /// (`maxValue` passed to `NewControl` before `make_popup` replaced `contrlMax` with the item count).
    pub popups: BTreeMap<u32, (i16, u32, i16)>,
    /// Port origin `(owner_win, portRect.left, portRect.top)` active when `Draw1Control` painted a view-relative control
    /// (MacApp/PowerPlant/Toolbox views that call `SetOrigin(-x, -y); Draw1Control(ctl)` with view-local `contrlRect`).
    pub drawn_origin: BTreeMap<u32, (u32, i16, i16)>,
    /// `TEHandle`s marked interactive (`TEFeatureFlag(teFOutlineHilite)` or `TEActivate`).
    pub te_editable: BTreeSet<u32>,
    /// Live single-line `TEHandle`s projected as native editable fields: `te_handle -> (owner_win, window_pixel_rect)`.
    pub te_boxes: BTreeMap<u32, (u32, Rect)>,
    /// Declarative `plane_chrome` bound edit fields: `token -> (owner_win, rect, current_text)`.
    pub bound_fields: BTreeMap<u32, (u32, heirloom_ui_abi::GuestRect, alloc::string::String)>,
    /// A native gesture waiting for its `TrackControl`: control -> (the part code the user's release is in, the value the drag leaves).
    /// It lives until that call, or until the next input supersedes it ([`Translate::supersede`]).
    pub tracks: BTreeMap<u32, (u16, Option<i16>)>,
    /// The List Manager lists: ListHandle -> the cells and selection the engine holds (the ListRec itself is guest memory).
    pub lists: BTreeMap<u32, lists::ListState>,
    pub dlg: dialogs::State,
    pub input: input::State,
    /// Loaded declarative `CDEF` bindings (`HEIRLOOM-APP-CHROME-NEEDS-A-BINDING`, keyed strictly by resource id/len/hash).
    pub cdef_bindings: alloc::vec::Vec<heirloom_bind::ControlDefBinding>,
    /// Loaded declarative in-plane chrome bindings (`HEIRLOOM-APP-CHROME-NEEDS-A-BINDING`, keyed strictly by resource type/id/len/hash).
    pub plane_chrome_bindings: alloc::vec::Vec<heirloom_bind::PlaneChromeBinding>,
    /// Shared Handle to a minimal 68k Pascal CDEF stub (`long CDEF(short varCode, ControlHandle ctl, short msg, long param)`
    /// returning 0) stored in `ControlRecord.contrlDefProc` (+24) so programs invoking `(*ctl)->contrlDefProc` directly
    /// (e.g. offscreen drawing via `CallUniversalProc`) never call NIL.
    pub std_cdef: u32,
}

impl Translate {
    /// The live projection of `obj`, if it has one.
    pub fn projected(&self, obj: Obj) -> Option<&Projected<Meta>> { self.live.get(obj.handle() as u64).filter(|p| p.meta.obj == obj) }
    pub fn live_count(&self) -> usize { self.live.len() }
    /// New input ends any native gesture still waiting for a `TrackControl` that never came.
    pub(crate) fn supersede(&mut self) { self.tracks.clear(); }
}

impl<U: GuestUi> Mac<U> {
    /// Make the projection of `obj` (in guest window `win`) match `want`: `None` = it should not be shown (hidden, or
    /// no role); `Some` = this model. A window that is not on screen yet shows nothing now: [`Mac::project_window`] runs
    /// when it opens. This is the only place a control is added, changed or removed on the host.
    pub(crate) fn sync_projection(&mut self, win: u32, obj: Obj, want: Option<ControlModel>) -> Result<(), Refusal> {
        let token = obj.handle() as u64;
        // a token is a guest address: an entry of ANOTHER object kind at it is a dead object's projection (a reused handle)
        if self.tr.live.get(token).is_some_and(|p| p.meta.obj != obj) { self.tr.live.retire(&mut self.ui, token).map_err(Refusal::ui)?; }
        let window = self.windows.get(&win).and_then(|w| w.native);
        self.tr.live.sync(&mut self.ui, window, token, Meta { win, obj }, want).map_err(Refusal::ui)
    }

    /// Put the guest's value back on the native control: the user's gesture changed the part optimistically (a check box
    /// toggles under the pointer) but on a Mac the PROGRAM changes the value, so until it does the guest record is the truth.
    pub(crate) fn restore_native(&mut self, obj: Obj) -> Result<(), Refusal> {
        if self.tr.projected(obj).is_none() { return Ok(()); }
        self.tr.live.restore(&mut self.ui, obj.handle() as u64).map_err(Refusal::ui)
    }

    /// The host already shows `v` (the user typed it): take it as the shown state so the next sync does not echo it back.
    pub(crate) fn adopt_native_state(&mut self, obj: Obj, v: &ControlValue) {
        if self.tr.projected(obj).is_some() { self.tr.live.adopt(obj.handle() as u64, v); }
    }

    /// Forget every projection of a window that is going away (the host retires its controls with the window).
    pub(crate) fn forget_window_projections(&mut self, win: u32) {
        self.tr.live.forget_where(|p| p.meta.win == win);
        self.tr.drawn_origin.retain(|_, &mut (w, _, _)| w != win);
        self.tr.te_boxes.retain(|_, &mut (w, _)| w != win);
        self.tr.bound_fields.retain(|_, &mut (w, _, _)| w != win);
    }

    /// A window just got its native window: show every control and item it already holds.
    pub(crate) fn project_window(&mut self, win: u32) -> Result<(), Refusal> {
        self.project_controls(win)?;
        self.project_lists(win)?;
        self.project_dialog_text(win)
    }

    pub(crate) fn deliver_intent(&mut self, win: UiObjectRef, control: UiObjectRef, token: CallbackToken, generation: u32, intent: Intent) -> Result<Delivered, Refusal> {
        let obj = self.tr.live.resolve(token.0, win, control, generation).map_err(Refusal)?.meta.obj;
        match obj {
            Obj::Control(h) => self.control_intent(h, intent)?,
            Obj::Text(h) => self.text_intent(h, intent)?,
            Obj::TextEdit(h) => self.te_intent(h, intent)?,
            Obj::BoundField(h) => self.bound_field_intent(h, intent)?,
            Obj::List(h) => self.list_intent(h, intent)?,
        }
        Ok(Delivered::Queued)
    }

    /// Route native edits on a declarative `plane_chrome` bound edit field back to the guest window via click + keystrokes.
    pub(crate) fn bound_field_intent(&mut self, tok: u32, intent: Intent) -> Result<(), Refusal> {
        let Some((win, rect, _)) = self.tr.bound_fields.get(&tok).cloned() else {
            return Err(Refusal(format!("Intent: bound field {tok:#x} is not projected")));
        };
        let is_commit = matches!(intent, Intent::CommitText(_));
        match intent {
            Intent::Press => {
                if let Some(w) = self.windows.get(&win).map(|w| w.rect) {
                    let gv = w.top.wrapping_add((rect.top + rect.height / 2) as i16);
                    let gh = w.left.wrapping_add((rect.left + rect.width / 2) as i16);
                    self.mouse_event(crate::events::MOUSE_DOWN, gv, gh, 0)?;
                    self.mouse_event(crate::events::MOUSE_UP, gv, gh, 0)?;
                }
                Ok(())
            }
            Intent::Drag(ControlValue::Text(t)) | Intent::CommitText(t) => {
                if let Some(e) = self.tr.bound_fields.get_mut(&tok) { e.2 = t.clone(); }
                self.adopt_native_state(Obj::BoundField(tok), &ControlValue::Text(t.clone()));
                if is_commit {
                    if let Some(w) = self.windows.get(&win).map(|w| w.rect) {
                        let gv = w.top.wrapping_add((rect.top + rect.height / 2) as i16);
                        let gh = w.left.wrapping_add((rect.left + rect.width / 2) as i16);
                        self.mouse_event(crate::events::MOUSE_DOWN, gv, gh, 0)?;
                        self.mouse_event(crate::events::MOUSE_UP, gv, gh, 0)?;
                    }
                    for ch in t.bytes().filter(|&b| (0x20..0x7F).contains(&b)) {
                        self.post(crate::events::KEY_DOWN, u32::from(ch), 0, 0);
                    }
                    self.post(crate::events::KEY_DOWN, (0x24u32 << 8) | 0x0D, 0, 0);
                }
                Ok(())
            }
            other => Err(Refusal(format!("Intent {other:?} has no meaning for a bound edit field"))),
        }
    }
}

/// Register the Control Manager, Dialog Manager and List Manager traps.
pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    controls::register(m)?;
    dialogs::register(m)?;
    dialog_items::register(m)?;
    useritems::register(m)?;
    alerts::register(m)?;
    lists::register(m)
}
