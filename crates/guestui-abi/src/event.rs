//! Native events back to the guest: ONE role-level intent event for controls, plain pointer and key input for planes.
//!
//! An intent says what the user MEANT toward a role (activate, set a value, select a row ...), never which guest message
//! it becomes: the engine maps it to the exact guest input the original OS would have produced (ruling 17, piece 3).
use alloc::string::String;
use crate::model::{CallbackToken, ControlValue, GuestRect, UiObjectRef};

/// What the user meant toward a control, by role. The engine reads it against the control's kind; an intent the kind does
/// not have (a `SelectRow` on a button) is a named refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    /// A complete click on a button-like control (Mac `TrackControl` returning its part, a GadTools button's GADGETUP).
    Activate,
    /// A stateful control took a new value (checkbox, toggle, slider level, cycle choice).
    SetValue(ControlValue),
    /// A list row was chosen (index in the guest's own row order).
    SelectRow(u32),
    /// A scrollbar or list scrolled so `top` is its first shown unit (guest units).
    ScrollTo(u32),
    /// A text or integer field was committed (Return / focus loss): the final text.
    CommitText(String),
    /// The pointer went down on the control (starts a two-ended gesture).
    Press,
    /// The gesture ended; `inside` false = it ended outside the control (cancelled: nothing activates).
    Release { inside: bool },
    /// The value changed while the gesture is held (a scrollbar knob drag; a field edit): the new value.
    Drag(ControlValue),
}

/// What the pointer did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerPhase { Down, Move, Up }

/// Which pointer button. `None` = motion with no button held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerButton { None, Primary, Secondary, Middle }

/// Keyboard modifier bits (`Modifiers(Modifiers::SHIFT | Modifiers::CTRL)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers(pub u8);
impl Modifiers {
    pub const SHIFT: u8 = 1;
    pub const CTRL: u8 = 2;
    pub const ALT: u8 = 4;
    pub const META: u8 = 8;
    pub fn has(self, bit: u8) -> bool { self.0 & bit == bit }
}

/// Native events back to the guest. Tokens identify the guest-side object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuestUiEvent {
    CloseRequested { win: UiObjectRef, token: CallbackToken },
    MenuCommand { win: UiObjectRef, token: CallbackToken },
    Resized { win: UiObjectRef, rect: GuestRect },
    /// The user acted on a native control. `generation` is the generation of the incarnation of `token` the host
    /// addressed (= `control.generation`, as `add_control` returned it); an engine refuses by name an intent whose
    /// `control` / `generation` is not the live control of that token (a token can be reused after its control was
    /// removed, but never with the same generation).
    Intent { win: UiObjectRef, control: UiObjectRef, token: CallbackToken, generation: u32, intent: Intent },
    /// Pointer input in window coordinates (0,0 = the window's top-left): what a content plane receives. `clicks` is the
    /// multi-click count of a `Down` (1 single, 2 double ...), 0 for `Move`.
    Pointer { win: UiObjectRef, x: i32, y: i32, button: PointerButton, phase: PointerPhase, clicks: u8 },
    /// A key press or release for the window: `code` is the host's key code, `text` what it types (empty for a non-character key).
    Key { win: UiObjectRef, code: u32, text: String, modifiers: Modifiers, down: bool },
}

impl GuestUiEvent {
    /// The intent event for a control exactly as `add_control` returned it (the generation comes from the ref).
    pub fn intent(win: UiObjectRef, control: UiObjectRef, token: CallbackToken, intent: Intent) -> Self {
        GuestUiEvent::Intent { win, control, token, generation: control.generation, intent }
    }
    /// A primary-button pointer event with a click count of 1 for `Down`, else 0 (the common case).
    pub fn primary(win: UiObjectRef, x: i32, y: i32, phase: PointerPhase) -> Self {
        let button = if phase == PointerPhase::Move { PointerButton::None } else { PointerButton::Primary };
        GuestUiEvent::Pointer { win, x, y, button, phase, clicks: (phase == PointerPhase::Down) as u8 }
    }
}
