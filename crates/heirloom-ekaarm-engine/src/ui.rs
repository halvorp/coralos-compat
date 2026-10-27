//! The UI seam of this engine: everything it shows goes through `heirloom_ui_abi::GuestUi`, nothing else. The process holds
//! the host's implementation behind [`EngineUi`] (a `GuestUi` plus `as_any`, so a host can take its concrete type back, e.g. the
//! runner prints a `RecordingGuestUi`'s calls); there is no UI state in the engine other than the window it opened.
use alloc::boxed::Box;
use core::any::Any;
use heirloom_ui_abi::{GuestUi, UiObjectRef, UiRefusal, WindowModel};

pub trait EngineUi {
    fn open_window(&mut self, model: WindowModel) -> Result<UiObjectRef, UiRefusal>;
    fn as_any(&self) -> &dyn Any;
    /// Give the concrete box back (`EkaArm` lends its UI to the process for a slice and takes it back).
    fn into_any(self: Box<Self>) -> Box<dyn Any>;
}

impl<U: GuestUi> EngineUi for U {
    fn open_window(&mut self, model: WindowModel) -> Result<UiObjectRef, UiRefusal> {
        GuestUi::open_window(self, model)
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
