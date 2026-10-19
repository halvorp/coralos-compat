//! A REQUEST: a short modal question with a row of buttons (a system alert / message box). Generic: no engine's types cross
//! this boundary. The engine formats the text and decides which button number means what to the guest; the host shows a
//! native alert and answers with the index of the button the user chose (`GuestUiEvent::RequestAnswered`). The task that
//! asked is parked by the ENGINE (its scheduler) until the answer arrives; the host and the other tasks keep running.
use alloc::string::String;
use alloc::vec::Vec;
use crate::model::{UiObjectRef, UiRefusal};

/// What to ask. `buttons` is left to right as the guest listed them (at least one).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestModel {
    pub title: String,
    /// The message, lines separated by `\n`.
    pub text: String,
    pub buttons: Vec<String>,
    /// The button Return chooses (index into `buttons`), if any.
    pub default: Option<u32>,
    /// The button Escape / closing the alert chooses (index into `buttons`), if any.
    pub cancel: Option<u32>,
}

impl RequestModel {
    pub fn validate(&self) -> Result<(), UiRefusal> {
        if self.buttons.is_empty() { return Err(UiRefusal::InvalidGeometry("a request needs at least one button")); }
        let n = self.buttons.len() as u32;
        if self.default.is_some_and(|d| d >= n) || self.cancel.is_some_and(|c| c >= n) { return Err(UiRefusal::InvalidGeometry("request default/cancel button past its buttons")); }
        Ok(())
    }
}

/// A live request (same identity-plus-generation rule as every other seam object).
pub type RequestRef = UiObjectRef;
