//! The ONE generic mapper (ruling 17, piece 2): an engine's toolkit table says which guest object is which [`ControlModel`];
//! this registry makes the host match it and keeps the single stale-intent rule. It knows no toolkit: the engine's per-object
//! metadata `K` (a guest window, an object kind) rides along untouched. Every engine (amiga68k, mac68k, macppc through it)
//! projects, updates, retires and resolves intents through here, so the three copies of the diff and of the generation check
//! that existed before cannot drift apart.
//!
//! * [`Projections::sync`]: make the host's control match `want` (`None` = not shown). Same kind of control: push only what
//!   changed (`set_control_rect`, `set_control_enabled`, `set_control` for the value). Different shape (another kind, a new
//!   label or fixed parameter): retire and add, which is a NEW generation, so an intent aimed at the old one is refused by name.
//! * [`check_ref`]: an intent must carry the generation of the control ref and name the live incarnation of its token.
use guestui_abi::{ControlKind, ControlModel, ControlValue, GuestUi, UiObjectRef, UiRefusal};
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;

/// One live projection: the host's window and control, what the host was last told, and the engine's own metadata.
#[derive(Debug, Clone)]
pub struct Projected<K> { pub window: UiObjectRef, pub native: UiObjectRef, pub meta: K, pub model: ControlModel }

/// The value a model's state is, when its kind has one (what `set_control` carries).
pub fn state(k: &ControlKind) -> Option<ControlValue> {
    Some(match k {
        ControlKind::Checkbox { checked } => ControlValue::Checked(*checked),
        ControlKind::ToggleButton { on } => ControlValue::Checked(*on),
        ControlKind::ScrollBar { total, visible, top, .. } => ControlValue::Scroll { total: *total, visible: *visible, top: *top },
        ControlKind::Field { text, .. } | ControlKind::Label { text, .. } | ControlKind::String { text, .. } => ControlValue::Text(text.clone()),
        ControlKind::Slider { value, .. } => ControlValue::Level(*value),
        _ => return None,
    })
}

/// Same kind of control with the same fixed parameters: the difference can be pushed instead of rebuilding the part.
pub fn same_shape(a: &ControlModel, b: &ControlModel) -> bool {
    use ControlKind as K;
    a.label == b.label && match (&a.kind, &b.kind) {
        (K::ScrollBar { vertical: x, .. }, K::ScrollBar { vertical: y, .. }) => x == y,
        (K::Field { max_len: x, editable: e, integer: i, .. }, K::Field { max_len: y, editable: f, integer: j, .. }) => (x, e, i) == (y, f, j),
        (K::String { max_len: x, .. }, K::String { max_len: y, .. }) => x == y,
        (K::Label { framed: x, .. }, K::Label { framed: y, .. }) => x == y,
        (K::Group { title: t, style: s }, K::Group { title: u, style: v }) => (t, s) == (u, v),
        (K::ArrowButton { dir: x }, K::ArrowButton { dir: y }) => x == y,
        (K::Slider { min: a0, max: a1, .. }, K::Slider { min: b0, max: b1, .. }) => (a0, a1) == (b0, b1),
        (x, y) => core::mem::discriminant(x) == core::mem::discriminant(y),
    }
}

/// The one stale-intent rule. `live` is the (window, control) the token is projected as now, if it is projected at all.
pub fn check_ref(token: u64, window: UiObjectRef, control: UiObjectRef, generation: u32, live: Option<(UiObjectRef, UiObjectRef)>) -> Result<(), String> {
    if generation != control.generation {
        return Err(format!("Intent: generation {generation} does not match control ref {}/{} (token {token:#x}, stale)", control.id, control.generation));
    }
    let Some((w, n)) = live else { return Err(format!("Intent: token {token:#x} is not a live control (retired, or never shown)")) };
    if n != control {
        return Err(format!("Intent: stale control ref {}/{}: token {token:#x} is control {}/{} now", control.id, control.generation, n.id, n.generation));
    }
    if w != window { return Err(format!("Intent: control {} does not belong to window {}", control.id, window.id)); }
    Ok(())
}

/// The registry: token -> live projection.
#[derive(Debug, Clone)]
pub struct Projections<K> { live: BTreeMap<u64, Projected<K>> }

impl<K> Default for Projections<K> { fn default() -> Self { Self { live: BTreeMap::new() } } }

impl<K: Clone> Projections<K> {
    pub fn get(&self, token: u64) -> Option<&Projected<K>> { self.live.get(&token) }
    pub fn get_mut(&mut self, token: u64) -> Option<&mut Projected<K>> { self.live.get_mut(&token) }
    pub fn len(&self) -> usize { self.live.len() }
    pub fn is_empty(&self) -> bool { self.live.is_empty() }
    pub fn iter(&self) -> impl Iterator<Item = (u64, &Projected<K>)> { self.live.iter().map(|(&t, p)| (t, p)) }

    /// Make the host's control for `token` match `want`. `window` is the host window it lives in: `None` = the guest window
    /// is not on screen yet, so a control to ADD waits ([`Projections::get`] says nothing is live) until the window opens.
    pub fn sync<U: GuestUi>(&mut self, ui: &mut U, window: Option<UiObjectRef>, token: u64, meta: K, want: Option<ControlModel>) -> Result<(), UiRefusal> {
        match (self.live.get(&token).cloned(), want) {
            (None, None) => Ok(()),
            (Some(_), None) => self.retire(ui, token),
            (None, Some(model)) => {
                let Some(window) = window else { return Ok(()) };
                let native = ui.add_control(window, model.clone())?;
                self.live.insert(token, Projected { window, native, meta, model });
                Ok(())
            }
            (Some(mut p), Some(model)) => {
                if !same_shape(&p.model, &model) {
                    self.retire(ui, token)?;
                    return self.sync(ui, window, token, meta, Some(model));
                }
                let (w, n) = (p.window, p.native);
                if p.model.rect != model.rect { ui.set_control_rect(w, n, model.rect)?; }
                if p.model.enabled != model.enabled { ui.set_control_enabled(w, n, model.enabled)?; }
                if let Some(v) = state(&model.kind).filter(|v| state(&p.model.kind).as_ref() != Some(v)) { ui.set_control(w, n, v)?; }
                p.model = model;
                p.meta = meta;
                self.live.insert(token, p);
                Ok(())
            }
        }
    }

    /// Remove the host control of `token` (a no-op when none is live).
    pub fn retire<U: GuestUi>(&mut self, ui: &mut U, token: u64) -> Result<(), UiRefusal> {
        match self.live.remove(&token) { Some(p) => ui.remove_control(p.window, p.native), None => Ok(()) }
    }

    /// Put the stored state back on the host control: the user's gesture changed the part optimistically, but the guest's
    /// record is the truth until the program changes it.
    pub fn restore<U: GuestUi>(&self, ui: &mut U, token: u64) -> Result<(), UiRefusal> {
        match self.live.get(&token).and_then(|p| state(&p.model.kind).map(|v| (p.window, p.native, v))) {
            Some((w, n, v)) => ui.set_control(w, n, v),
            None => Ok(()),
        }
    }

    /// The host already shows `v` (the user typed it): take it as the shown state so the next sync does not echo it back.
    pub fn adopt(&mut self, token: u64, v: &ControlValue) {
        if let Some(p) = self.live.get_mut(&token) { let _ = p.model.apply(v); }
    }

    /// Forget (without telling the host) every projection that satisfies `gone`: the host retires a closed window's controls itself.
    pub fn forget_where(&mut self, mut gone: impl FnMut(&Projected<K>) -> bool) { self.live.retain(|_, p| !gone(p)); }

    /// Remove the projection of `token` from the registry only (the host side is already gone), returning it.
    pub fn take(&mut self, token: u64) -> Option<Projected<K>> { self.live.remove(&token) }

    /// The live projection an intent names, or the named reason it is stale.
    pub fn resolve(&self, token: u64, window: UiObjectRef, control: UiObjectRef, generation: u32) -> Result<&Projected<K>, String> {
        check_ref(token, window, control, generation, self.live.get(&token).map(|p| (p.window, p.native)))?;
        Ok(&self.live[&token])
    }
}
