//! The UI seam of this engine: everything it shows goes through `heirloom_ui_abi::GuestUi`, nothing else. The process holds
//! the host's implementation behind [`EngineUi`] (a `GuestUi` plus `as_any`, so a host can take its concrete type back, e.g. the
//! runner prints a `RecordingGuestUi`'s calls); there is no UI state in the engine other than the window it opened.
use alloc::boxed::Box;
use core::any::Any;
use alloc::string::String;
use heirloom_ui_abi::{ControlModel, ControlValue, GuestUi, GuestRect, UiObjectRef, UiRefusal, WindowModel};

pub trait EngineUi {
    fn open_window(&mut self, model: WindowModel) -> Result<UiObjectRef, UiRefusal>;
    /// The guest retitled the window (`GuestUi::set_window_title`).
    fn set_window_title(&mut self, win: UiObjectRef, title: String) -> Result<(), UiRefusal>;
    /// Add a control to `win` (`GuestUi::add_control`); the host's handle for it comes back.
    fn add_control(&mut self, win: UiObjectRef, control: ControlModel) -> Result<UiObjectRef, UiRefusal>;
    fn remove_control(&mut self, win: UiObjectRef, control: UiObjectRef) -> Result<(), UiRefusal>;
    /// The guest changed a control's state (`GuestUi::set_control`).
    fn set_control(&mut self, win: UiObjectRef, control: UiObjectRef, value: ControlValue) -> Result<(), UiRefusal>;
    fn set_control_rect(&mut self, win: UiObjectRef, control: UiObjectRef, rect: GuestRect) -> Result<(), UiRefusal>;
    fn as_any(&self) -> &dyn Any;
    /// Give the concrete box back (`EkaArm` lends its UI to the process for a slice and takes it back).
    fn into_any(self: Box<Self>) -> Box<dyn Any>;
}

impl<U: GuestUi> EngineUi for U {
    fn open_window(&mut self, model: WindowModel) -> Result<UiObjectRef, UiRefusal> {
        GuestUi::open_window(self, model)
    }
    fn set_window_title(&mut self, win: UiObjectRef, title: String) -> Result<(), UiRefusal> {
        GuestUi::set_window_title(self, win, title)
    }
    fn add_control(&mut self, win: UiObjectRef, control: ControlModel) -> Result<UiObjectRef, UiRefusal> {
        GuestUi::add_control(self, win, control)
    }
    fn remove_control(&mut self, win: UiObjectRef, control: UiObjectRef) -> Result<(), UiRefusal> {
        GuestUi::remove_control(self, win, control)
    }
    fn set_control(&mut self, win: UiObjectRef, control: UiObjectRef, value: ControlValue) -> Result<(), UiRefusal> {
        GuestUi::set_control(self, win, control, value)
    }
    fn set_control_rect(&mut self, win: UiObjectRef, control: UiObjectRef, rect: GuestRect) -> Result<(), UiRefusal> {
        GuestUi::set_control_rect(self, win, control, rect)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

/// The host objects of a process: its file view and its UI. Both start empty; a missing one is a named refusal at the
/// call that needs it, never a default.
#[derive(Default)]
pub struct Host {
    pub drive: Option<crate::drive::Drive>,
    pub ui: Option<Box<dyn EngineUi>>,
    /// The guest-resident `CEikonEnv` returned by `CCoeEnv::Static()`, allocated on first use.
    pub env: Option<u32>,
    /// The application window `BaseConstructL` opened.
    pub window: Option<UiObjectRef>,
}

impl crate::loader::Process {
    pub fn set_drive(&mut self, drive: crate::drive::Drive) {
        self.host.drive = Some(drive);
    }
    pub fn set_ui<U: GuestUi>(&mut self, ui: U) {
        self.host.ui = Some(Box::new(ui));
    }
    /// Hand the process a UI that is already boxed (no move of the `U` itself).
    pub fn set_ui_boxed(&mut self, ui: Box<dyn EngineUi>) {
        self.host.ui = Some(ui);
    }
    /// Take the UI back out.
    pub fn take_ui(&mut self) -> Option<Box<dyn EngineUi>> {
        self.host.ui.take()
    }
    /// The UI implementation as its concrete type, if that is what was set.
    pub fn ui_as<T: Any>(&self) -> Option<&T> {
        self.host.ui.as_ref()?.as_any().downcast_ref::<T>()
    }
}
