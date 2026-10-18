//! The Mac side of the translation layer (ruling 17, pieces 2, 3 and 8): the Toolbox's own UI objects, shown as semantic
//! controls through the ONE seam (`guestui-abi`), and native intents turned back into the exact input the original OS
//! produced. Everything is keyed on the TOOLKIT, never on an application:
//!
//! * [`controls`]: ONE table, `procID` (the CDEF) -> role, with the attribute map into the real `ControlRecord` in guest
//!   memory and the part-code geometry (`FindControl`/`TrackControl`).
//! * [`dialogs`]: ONE table, DITL item type -> role; push buttons, check boxes and `resCtrl` items reuse the control
//!   table, static and editable text are fields of their own.
//! * [`input`]: plain pointer and key input for content (a plane is fully usable before it has any binding).
//! * this file: the generic mapper both tables feed: project / update / retire an object, and the single stale-intent
//!   check. No role is ever inferred from what pixels look like (RETRO-ROLES-REQUIRE-A-CONTRACT): a role exists because a
//!   Toolbox record or resource says so.
//!
//! The guest's memory stays authoritative. A projected object is a VIEW of its record: after any Toolbox call that
//! changes the record the model is recomputed from guest memory and only the difference is pushed (a changed value is
//! `set_control`, a moved rect `set_control_rect`, a changed title or shape a new incarnation: remove + add, which is a new
//! generation, so the old ref is refused by name).
pub mod controls;
pub mod dialogs;
pub mod input;

use crate::events::Delivered;
use crate::machine::Mac;
use crate::Refusal;
use alloc::collections::BTreeMap;
use alloc::format;
use guestui_abi::{CallbackToken, ControlKind, ControlModel, ControlValue, GuestUi, Intent, UiObjectRef};

/// What a projected token stands for in the guest. The token is always the guest object's own address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Obj {
    /// A `ControlHandle` (a Control Manager control, or a dialog's button / check box / `resCtrl` item).
    Control(u32),
    /// The `Handle` of a dialog's static or editable text item.
    Text(u32),
}

impl Obj {
    pub fn handle(self) -> u32 { match self { Obj::Control(h) | Obj::Text(h) => h } }
}

/// One live projection: which guest object, in which guest window, is which native control.
#[derive(Debug, Clone)]
pub struct Projected { pub win: u32, pub window: UiObjectRef, pub native: UiObjectRef, pub obj: Obj, pub model: ControlModel }

#[derive(Default)]
pub struct Translate {
    live: BTreeMap<u64, Projected>,
    /// `ControlHandle` -> its `procID` (a ControlRecord holds only a CDEF handle, so the id lives host-side).
    pub procs: BTreeMap<u32, u16>,
    /// A native gesture waiting for its `TrackControl`: control -> (the part code the user's release is in, the value the drag leaves).
    pub tracks: BTreeMap<u32, (u16, Option<i16>)>,
    pub dlg: dialogs::State,
    pub input: input::State,
}

impl Translate {
    /// The live projection of `obj`, if it has one.
    pub fn projected(&self, obj: Obj) -> Option<&Projected> { self.live.get(&(obj.handle() as u64)).filter(|p| p.obj == obj) }
    pub fn live_count(&self) -> usize { self.live.len() }
}

/// The value a model's state is, when its kind has one (what `set_control` carries).
fn state(k: &ControlKind) -> Option<ControlValue> {
    Some(match k {
        ControlKind::Checkbox { checked } => ControlValue::Checked(*checked),
        ControlKind::ScrollBar { total, visible, top, .. } => ControlValue::Scroll { total: *total, visible: *visible, top: *top },
        ControlKind::Field { text, .. } | ControlKind::Label { text, .. } => ControlValue::Text(text.clone()),
        _ => return None,
    })
}

/// Same kind of control with the same fixed parameters: the difference can be pushed instead of rebuilding the part.
fn same_shape(a: &ControlModel, b: &ControlModel) -> bool {
    a.label == b.label && match (&a.kind, &b.kind) {
        (ControlKind::ScrollBar { vertical: x, .. }, ControlKind::ScrollBar { vertical: y, .. }) => x == y,
        (ControlKind::Field { max_len: x, editable: e, integer: i, .. }, ControlKind::Field { max_len: y, editable: f, integer: j, .. }) => (x, e, i) == (y, f, j),
        (ControlKind::Label { framed: x, .. }, ControlKind::Label { framed: y, .. }) => x == y,
        (x, y) => core::mem::discriminant(x) == core::mem::discriminant(y),
    }
}

impl<U: GuestUi> Mac<U> {
    /// Make the projection of `obj` (in guest window `win`) match `want`: `None` = it should not be shown (hidden, or
    /// no role); `Some` = this model. A window that is not on screen yet shows nothing now: [`Mac::project_window`] runs
    /// when it opens. This is the only place a control is added, changed or removed on the host.
    pub(crate) fn sync_projection(&mut self, win: u32, obj: Obj, want: Option<ControlModel>) -> Result<(), Refusal> {
        let token = obj.handle() as u64;
        let live = self.tr.live.get(&token).filter(|p| p.obj == obj).cloned();
        match (live, want) {
            (None, None) => Ok(()),
            (Some(p), None) => self.retire_projection(&p),
            (None, Some(model)) => {
                let Some(window) = self.windows.get(&win).and_then(|w| w.native) else { return Ok(()) };
                let native = self.ui.add_control(window, model.clone()).map_err(Refusal::ui)?;
                self.tr.live.insert(token, Projected { win, window, native, obj, model });
                Ok(())
            }
            (Some(mut p), Some(model)) => {
                if !same_shape(&p.model, &model) {
                    self.retire_projection(&p)?;
                    return self.sync_projection(win, obj, Some(model));
                }
                let (w, n) = (p.window, p.native);
                if p.model.rect != model.rect { self.ui.set_control_rect(w, n, model.rect).map_err(Refusal::ui)?; }
                if p.model.enabled != model.enabled { self.ui.set_control_enabled(w, n, model.enabled).map_err(Refusal::ui)?; }
                if let Some(v) = state(&model.kind).filter(|v| state(&p.model.kind).as_ref() != Some(v)) { self.ui.set_control(w, n, v).map_err(Refusal::ui)?; }
                p.model = model;
                self.tr.live.insert(token, p);
                Ok(())
            }
        }
    }

    fn retire_projection(&mut self, p: &Projected) -> Result<(), Refusal> {
        self.tr.live.remove(&(p.obj.handle() as u64));
        self.ui.remove_control(p.window, p.native).map_err(Refusal::ui)
    }

    /// Put the guest's value back on the native control: the user's gesture changed the part optimistically (a check box
    /// toggles under the pointer) but on a Mac the PROGRAM changes the value, so until it does the guest record is the truth.
    pub(crate) fn restore_native(&mut self, obj: Obj) -> Result<(), Refusal> {
        let Some(p) = self.tr.projected(obj).cloned() else { return Ok(()) };
        match state(&p.model.kind) { Some(v) => self.ui.set_control(p.window, p.native, v).map_err(Refusal::ui), None => Ok(()) }
    }

    /// The host already shows `v` (the user typed it): take it as the shown state so the next sync does not echo it back.
    pub(crate) fn adopt_native_state(&mut self, obj: Obj, v: &ControlValue) {
        if let Some(p) = self.tr.live.get_mut(&(obj.handle() as u64)).filter(|p| p.obj == obj) { let _ = p.model.apply(v); }
    }

    /// Forget every projection of a window that is going away (the host retires its controls with the window).
    pub(crate) fn forget_window_projections(&mut self, win: u32) { self.tr.live.retain(|_, p| p.win != win); }

    /// A window just got its native window: show every control and item it already holds.
    pub(crate) fn project_window(&mut self, win: u32) -> Result<(), Refusal> {
        self.project_controls(win)?;
        self.project_dialog_text(win)
    }

    /// The one stale-intent check: the intent must name the live incarnation of its token.
    fn resolve_intent(&self, win: UiObjectRef, control: UiObjectRef, token: CallbackToken, generation: u32) -> Result<Projected, Refusal> {
        if generation != control.generation {
            return Err(Refusal(format!("Intent: generation {generation} does not match control ref {}/{}", control.id, control.generation)));
        }
        let Some(p) = self.tr.live.get(&token.0) else {
            return Err(Refusal(format!("Intent: token {:#x} is not a live control (retired, or never shown)", token.0)));
        };
        if p.native != control {
            return Err(Refusal(format!("Intent: stale control ref {}/{}: token {:#x} is control {}/{} now", control.id, control.generation, token.0, p.native.id, p.native.generation)));
        }
        if p.window != win { return Err(Refusal(format!("Intent: control {} does not belong to window {}", control.id, win.id))); }
        Ok(p.clone())
    }

    pub(crate) fn deliver_intent(&mut self, win: UiObjectRef, control: UiObjectRef, token: CallbackToken, generation: u32, intent: Intent) -> Result<Delivered, Refusal> {
        let p = self.resolve_intent(win, control, token, generation)?;
        match p.obj {
            Obj::Control(h) => self.control_intent(h, intent)?,
            Obj::Text(h) => self.text_intent(h, intent)?,
        }
        Ok(Delivered::Queued)
    }
}

/// Register the Control Manager and Dialog Manager traps.
pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    controls::register(m)?;
    dialogs::register(m)
}
