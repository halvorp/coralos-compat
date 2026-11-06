//! The native seed classes of the osxx86 personality and the adapter that publishes them through [`heirloom_ui_abi::GuestUi`].
//!
//! A guest that opens a window with `NSWindow`, adds controls to its content view and runs `[NSApp run]` reaches the host only
//! through the UI seam: windows become `open_window`, controls become `add_control` models with a token and a generation, host
//! events come back from `poll_event` and are delivered as the Cocoa messages the app expects (the button's action sent to its
//! target). Nothing here draws a pixel. These are the classes the smallest proof apps need; the frameworks a real app loads
//! (docs/doctrine OSXX86-FRAMEWORKS-ARE-GUEST-CODE-OVER-A-NATIVE-SEAM) register over the same runtime and publish through the
//! same seam.
#![cfg_attr(not(test), no_std)]
extern crate alloc;

mod app;
mod controls;
pub mod state;
pub mod string;
pub mod view;
pub mod window;
#[cfg(test)]
mod tests;

pub use state::{begin, end, AppHost};

/// Registers every class this crate provides (idempotent); `NSObject` must already be installed.
pub fn install() {
    string::install();
    view::install();
    window::install();
    app::install();
    controls::install();
}

/// The address a framework symbol binds to: the data symbols (`_NSApp`, `___CFConstantStringClassReference`), the registered
/// classes and the runtime's own entry points. None for anything this crate does not provide.
pub fn symbol(name: &str) -> Option<usize> {
    match name {
        "_NSApp" => Some(app::nsapp_cell()),
        "___CFConstantStringClassReference" => {
            let c = heirloom_objc::runtime::class_named("NSConstantString");
            (!c.is_null()).then_some(c as usize)
        }
        _ => heirloom_objc::libobjc::symbol(name),
    }
}
