//! Normalized model types (plan §4: WindowModel, ControlModel, MenuModel, CallbackToken...).

use alloc::vec::Vec;
use alloc::string::String;
/// Launch-local object identity plus generation (a stale ref is refused, never aliased).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UiObjectRef { pub id: u32, pub generation: u32 }

/// Opaque adapter-owned continuation identity: lets the engine map an event back to its
/// guest object (e.g. a Gadget pointer) without the UI side ever seeing a guest address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CallbackToken(pub u64);

/// A rectangle in the GUEST coordinate domain (distinct from the native domain, which the
/// real implementation owns).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestRect { pub left: i32, pub top: i32, pub width: i32, pub height: i32 }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowRole { Document, Dialog, Utility }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowModel {
    pub title: String,
    pub rect: GuestRect,
    pub role: WindowRole,
    pub closable: bool,
    pub resizable: bool,
    pub token: CallbackToken,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    pub label: String,
    pub enabled: bool,
    /// Keyboard shortcut character (command-key meaning), if any.
    pub shortcut: Option<char>,
    /// `Some(checked)` for a checkable item.
    pub check: Option<bool>,
    pub sub_items: Vec<MenuItem>,
    pub token: CallbackToken,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Menu { pub title: String, pub enabled: bool, pub items: Vec<MenuItem> }

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MenuModel { pub menus: Vec<Menu> }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlKind {
    Button,
    Checkbox { checked: bool },
    String { text: String, max_len: u32 },
    Slider { min: i32, max: i32, value: i32 },
    /// Read-only text the guest updates (GadTools TEXT_KIND; `framed` = GTTX_Border's recessed box).
    Label { text: String, framed: bool },
    /// A two-state push button that stays down (an Intuition GACT_TOGGLESELECT boolean gadget).
    ToggleButton { on: bool },
    /// A scrollbar over `total` units of which `visible` show, the first shown being `top` (all in the guest's own units).
    /// `top` is at most `total - visible`; a `visible >= total` bar has nothing to scroll. The host draws the knob, the
    /// arrows are separate [`ControlKind::ArrowButton`]s when the guest has them.
    Scrollbar { orientation: Orientation, total: u32, visible: u32, top: u32 },
    /// A one-line text or integer field. `editable` false = shown but not typed into; `integer` = digits (and a leading
    /// minus) only; `max_len` counts characters.
    Field { text: String, max_len: u32, editable: bool, integer: bool },
    /// A static bevelled frame around or behind other controls (no interaction, no events).
    Frame { bevel: Bevel },
    /// A button that shows a direction arrow instead of a label (scrollbar arrows, spinners).
    ArrowButton { dir: ArrowDir },
    /// An invisible hit area: no visuals, but pointer press/release inside it are events (a gadget with nothing to draw).
    Hotspot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation { Horizontal, Vertical }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrowDir { Up, Down, Left, Right }

/// What a frame looks like, as a role (the host maps it onto its own kit parts).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bevel { Raised, Recessed, Ridge, DropBox }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlModel {
    pub kind: ControlKind,
    pub label: String,
    pub rect: GuestRect,
    pub enabled: bool,
    pub token: CallbackToken,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlValue { Pressed, Checked(bool), Text(String), Level(i32), Scroll { total: u32, visible: u32, top: u32 },
    /// Only in a `ControlDriven` `Up`: the gesture ended outside the control (no activation, nothing changed).
    Cancelled }

impl ControlModel {
    /// Apply `value` to this model the way a host applies `set_control`: the shared rule for what each kind can hold.
    /// A value the kind cannot hold is `UiRefusal::Unsupported`; a `Scroll` with `top` past the end is clamped to it.
    pub fn apply(&mut self, value: &ControlValue) -> Result<(), UiRefusal> {
        match (&mut self.kind, value) {
            (_, ControlValue::Pressed | ControlValue::Cancelled) => {}
            (ControlKind::Checkbox { checked }, ControlValue::Checked(b)) => *checked = *b,
            (ControlKind::ToggleButton { on }, ControlValue::Checked(b)) => *on = *b,
            (ControlKind::String { text, .. } | ControlKind::Label { text, .. } | ControlKind::Field { text, .. }, ControlValue::Text(t)) => *text = t.clone(),
            (ControlKind::Slider { min, max, value }, ControlValue::Level(l)) => {
                if l < min || l > max { return Err(UiRefusal::InvalidGeometry("slider level outside its range")); }
                *value = *l;
            }
            (ControlKind::Scrollbar { total: t, visible: v, top: p, .. }, ControlValue::Scroll { total, visible, top }) => {
                (*t, *v) = (*total, *visible);
                *p = (*top).min(total.saturating_sub(*visible));
            }
            _ => return Err(UiRefusal::Unsupported("a value the control's kind cannot hold")),
        }
        Ok(())
    }
}

/// Native events back to the guest. Tokens identify the guest-side object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuestUiEvent {
    CloseRequested { win: UiObjectRef, token: CallbackToken },
    ControlChanged { win: UiObjectRef, control: UiObjectRef, token: CallbackToken, value: ControlValue },
    MenuCommand { win: UiObjectRef, token: CallbackToken },
    Resized { win: UiObjectRef, rect: GuestRect },
    /// The pointer's primary button or motion, in the window's own coordinates (0,0 = its top-left corner). The smallest
    /// generic pointer input: a guest-drawn gadget (BOOPSI) needs it to be hit-tested and driven.
    Pointer { win: UiObjectRef, x: i32, y: i32, phase: PointerPhase },
    /// The user is driving a native control, with its two-ended gesture (what Intuition reports as GADGETDOWN / MOUSEMOVE /
    /// GADGETUP): `Down` = pressed (a button or arrow goes down, a scrollbar drag starts, a field takes focus); `Move` = the
    /// value changed while the gesture is held (a scrollbar knob drag, a field edit; `value` is the new value); `Up` =
    /// released / committed (a button release inside it, a scrollbar drag ends, a field's Return; `value` is the final one,
    /// `Pressed` for a plain button). A toggle's `Up` carries its new `Checked`. Tokens are the guest's own.
    ControlDriven { win: UiObjectRef, control: UiObjectRef, token: CallbackToken, phase: PointerPhase, value: ControlValue },
}

/// What the primary pointer button / motion did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerPhase { Down, Move, Up }

/// Named refusals (the plan: refuse by name, never silently discard).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiRefusal {
    UnknownObject(UiObjectRef),
    StaleGeneration(UiObjectRef),
    InvalidGeometry(&'static str),
    Unsupported(&'static str),
}
impl core::fmt::Display for UiRefusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            UiRefusal::UnknownObject(r) => write!(f, "unknown UI object {}", r.id),
            UiRefusal::StaleGeneration(r) => write!(f, "stale UI object {} (generation {})", r.id, r.generation),
            UiRefusal::InvalidGeometry(w) => write!(f, "invalid geometry: {w}"),
            UiRefusal::Unsupported(w) => write!(f, "unsupported: {w}"),
        }
    }
}
impl core::error::Error for UiRefusal {}
