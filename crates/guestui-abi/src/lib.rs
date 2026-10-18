//! guestui: FIRST DRAFT of the interface the retro engines call.
//!
//! # Status
//! Derived from `03-NATIVE-UI-GUESTUI.md` §4 (normalized model) of the CoralOS plan. These
//! are plain Rust types plus a [`GuestUi`] trait so engines can be written and tested now.
//! The REAL implementation maps these operations onto CoralOS `gadget.library` and
//! `coral-wm` and lives LATER, inside CoralOS (it is not in this repository). The planned
//! production ABI is a flat, version-first, caller-owned-storage vtable
//! (`query_requirements / init_context / plan_apply / apply / translate_event /
//! query_state`, plan §5); this trait is NOT that ABI and carries `String`/`Vec`. It
//! exists to fix the VOCABULARY (windows, menus, controls, events, named refusals).
//!
//! Records are pointer-free: no guest address ever crosses this boundary.
#![cfg_attr(not(test), no_std)]
extern crate alloc;
mod model;
mod recording;

pub use model::*;
pub use recording::{pixel_hash, RecordingGuestUi, UiCall};

/// What the engines call. One implementation per host (real CoralOS, recording, headless).
pub trait GuestUi {
    fn open_window(&mut self, model: WindowModel) -> Result<UiObjectRef, UiRefusal>;
    fn close_window(&mut self, win: UiObjectRef) -> Result<(), UiRefusal>;
    /// The guest retitled the window (Intuition SetWindowTitles).
    fn set_window_title(&mut self, win: UiObjectRef, title: alloc::string::String) -> Result<(), UiRefusal>;
    /// Replaces the window's menu strip (the plan's `MenuModel`).
    fn set_menu_strip(&mut self, win: UiObjectRef, menu: MenuModel) -> Result<(), UiRefusal>;
    fn clear_menu_strip(&mut self, win: UiObjectRef) -> Result<(), UiRefusal>;
    fn add_control(&mut self, win: UiObjectRef, control: ControlModel) -> Result<UiObjectRef, UiRefusal>;
    fn remove_control(&mut self, win: UiObjectRef, control: UiObjectRef) -> Result<(), UiRefusal>;
    /// The guest changed a control's state (a label's text, a checkbox, a slider level). A value the control's kind
    /// cannot hold is `UiRefusal::Unsupported`.
    fn set_control(&mut self, win: UiObjectRef, control: UiObjectRef, value: ControlValue) -> Result<(), UiRefusal>;
    /// Guest-DRAWN content: pixels a guest program drew itself (through its graphics library) into the window's
    /// content canvas. `width` x `height` is the whole canvas (the window's inner area); `damage` is the sub-rectangle
    /// that changed, in canvas pixels; `pixels` holds exactly `damage.width * damage.height` values, row-major,
    /// `0x00RRGGBB` (the engine resolved its own palette, so the host needs no palette semantics; a palette change
    /// arrives as a full-canvas damage). The first frame for a window is always the full canvas. One call per
    /// engine step per window, never per drawing primitive. Hosts without a canvas part refuse by name.
    fn present_content(&mut self, _win: UiObjectRef, _damage: GuestRect, _width: i32, _height: i32, _pixels: &[u32]) -> Result<(), UiRefusal> {
        Err(UiRefusal::Unsupported("guest-drawn content (no canvas part in this host)"))
    }
    /// Next native event for the guest, if any.
    fn poll_event(&mut self) -> Option<GuestUiEvent>;
}
