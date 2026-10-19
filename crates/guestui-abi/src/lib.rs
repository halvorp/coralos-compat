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
mod event;
mod model;
mod plane;
mod recording;

pub use event::*;
pub use model::*;
pub use plane::*;
pub use recording::{pixel_hash, RecordingGuestUi, UiCall};

/// What an icon depicts, in generic terms (a host maps each to its own artwork).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum IconRole {
    /// A mounted volume with a root directory (Workbench WBDISK).
    Disk,
    /// A removable or secondary volume (the engine asks `Disk`; reserved for hosts/engines that can tell).
    Volume,
    /// A directory (WBDRAWER).
    Drawer,
    /// An executable (WBTOOL).
    Tool,
    /// A document (WBPROJECT).
    Project,
    /// The trashcan (WBGARBAGE).
    Trash,
    /// A device (WBDEVICE).
    Device,
    /// A boot/Kickstart disk (WBKICK).
    Kick,
}

/// What the engines call. One implementation per host (real CoralOS, recording, headless).
pub trait GuestUi: 'static {
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
    /// The guest moved or resized a control (a relative gadget after its window resized, a re-laid-out BOOPSI gadget). The rect
    /// is in the same window coordinates as `add_control`'s. Hosts without it refuse by name.
    fn set_control_rect(&mut self, _win: UiObjectRef, _control: UiObjectRef, _rect: GuestRect) -> Result<(), UiRefusal> {
        Err(UiRefusal::Unsupported("moving a control (no set_control_rect in this host)"))
    }
    /// The guest enabled or disabled (ghosted) a control after creating it (GA_Disabled).
    fn set_control_enabled(&mut self, _win: UiObjectRef, _control: UiObjectRef, _enabled: bool) -> Result<(), UiRefusal> {
        Err(UiRefusal::Unsupported("enabling a control (no set_control_enabled in this host)"))
    }
    /// Guest-DRAWN content: pixels a guest program drew itself (through its graphics library) into the window's
    /// content canvas. `width` x `height` is the whole canvas (the window's inner area); `damage` is the sub-rectangle
    /// that changed, in canvas pixels; `pixels` holds exactly `damage.width * damage.height` values, row-major,
    /// `0x00RRGGBB` (the engine resolved its own palette, so the host needs no palette semantics; a palette change
    /// arrives as a full-canvas damage). The first frame for a window is always the full canvas. One call per
    /// engine step per window, never per drawing primitive. Hosts without a canvas part refuse by name.
    fn present_content(&mut self, _win: UiObjectRef, _damage: GuestRect, _width: i32, _height: i32, _pixels: &[u32]) -> Result<(), UiRefusal> {
        Err(UiRefusal::Unsupported("guest-drawn content (no canvas part in this host)"))
    }
    /// Guest drawing as a DISPLAY LIST (ruling 17): a content plane the engine could not map to controls, as recorded ops
    /// with theme-role pens. `plane_damage.rect` is the region to repaint and `ops` everything drawn there; the host
    /// replays it with the kit's own typography and theme. Hosts without plane replay refuse by name (the engine then
    /// falls back to `present_content`).
    fn present_plane(&mut self, _win: UiObjectRef, _plane_damage: Plane) -> Result<(), UiRefusal> {
        Err(UiRefusal::Unsupported("guest content planes (no display-list replay in this host)"))
    }
    /// A host-drawn icon for a generic object `role`, `width` x `height` pixels, row-major `0xAARRGGBB` (alpha >= 0x80 is
    /// opaque, below it transparent), exactly `width * height` values. `None` = this host has no artwork for the role
    /// (the engine then draws its own plain placeholder and logs that). Engines own no icon artwork: it lives host-side.
    fn icon_image(&mut self, _role: IconRole, _width: u32, _height: u32) -> Option<alloc::vec::Vec<u32>> { None }
    /// Next native event for the guest, if any.
    fn poll_event(&mut self) -> Option<GuestUiEvent>;
}
