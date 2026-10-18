//! Which libraries OpenLibrary builds with bridged functions (the rest of a known library's vectors are named refusals).
use crate::libtable::LibraryBuilder;
use guestui_abi::GuestUi;

/// The vector table for the library `name` (as the generated `lvo_names` spells it): its implemented functions
/// registered, every other known vector left to [`LibraryBuilder::install`] as a named refusal.
pub fn library<U: GuestUi>(name: &'static str, base: u32) -> LibraryBuilder<U> {
    let b = LibraryBuilder::new(name, base);
    match name {
        "lowlevel.library" => crate::lowlevel::register(b),
        "icon.library" => crate::icon::register(b),
        _ => b,
    }
}
