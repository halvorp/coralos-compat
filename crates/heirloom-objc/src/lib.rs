//! The Apple Objective-C 2 runtime for the osxx86 personality, written clean room from the published metadata layouts.
//!
//! * [`abi`]: the `class_t` / `class_ro_t` / `method_t` / `ivar_t` / `category_t` layouts;
//! * [`runtime`]: selectors, the class registry, realization of an image's metadata (superclass-first, instance variables
//!   moved when the superclass differs from what the compiler assumed), method lookup with `+initialize`, instances with
//!   reference counts, weak slots, autorelease pools;
//! * [`msg`]: the `objc_msgSend` family as assembly trampolines;
//! * [`nsobject`]: the native root class;
//! * [`libobjc`]: the symbol table a Mach-O app's imports from `libobjc.A.dylib` bind to.
//!
//! Frameworks (strings, collections, AppKit) are classes registered on top of this runtime, either as native Rust classes via
//! [`runtime::register_native_class`] or as Objective-C code in Mach-O libraries the loader maps and registers like the app.
//! One guest runs per process; the registry is a process-wide singleton.
#![cfg_attr(not(test), no_std)]
extern crate alloc;

pub mod abi;
pub mod libobjc;
pub mod msg;
pub mod nsobject;
pub mod runtime;
#[cfg(test)]
mod tests;

pub use runtime::{call_load_methods, register_image, ImageMeta};

/// The address of the `objc_msgSend` trampoline (tests and tools call it directly).
pub fn msgsend_address() -> usize {
    msg::osxobjc_msgsend as *const () as usize
}
