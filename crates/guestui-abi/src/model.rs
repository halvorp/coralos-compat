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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlModel {
    pub kind: ControlKind,
    pub label: String,
    pub rect: GuestRect,
    pub enabled: bool,
    pub token: CallbackToken,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlValue { Pressed, Checked(bool), Text(String), Level(i32) }

/// Native events back to the guest. Tokens identify the guest-side object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuestUiEvent {
    CloseRequested { win: UiObjectRef, token: CallbackToken },
    ControlChanged { win: UiObjectRef, control: UiObjectRef, token: CallbackToken, value: ControlValue },
    MenuCommand { win: UiObjectRef, token: CallbackToken },
    Resized { win: UiObjectRef, rect: GuestRect },
}

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
