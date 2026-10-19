//! Which libraries OpenLibrary builds with bridged functions (the rest of a known library's vectors are named refusals).
use crate::libtable::LibraryBuilder;
use guestui_abi::GuestUi;

/// The vector table for the library `name` (as the generated `lvo_names` spells it): its implemented functions
/// registered, every other known vector left to [`LibraryBuilder::install`] as a named refusal.
pub fn library<U: GuestUi>(name: &'static str, base: u32) -> LibraryBuilder<U> {
    let b = LibraryBuilder::new(name, base);
    match name {
        "lowlevel.library" => crate::lowlevel::register(b),
        "utility.library" => crate::utility::register(b).with(crate::utility_date::register::<U>).with(crate::utility_tags::register::<U>),
        "icon.library" => crate::icon::register(b),
        "locale.library" => crate::locale::register(b),
        "mathieeesingbas.library" => crate::mathieee::register_single(b),
        "mathieeedoubbas.library" => crate::mathieee::register_double(b),
        "workbench.library" => crate::workbench::register(b),
        "asl.library" => crate::asl::register(b),
        "iffparse.library" => crate::iff::register(b),
        "commodities.library" => crate::commodities::register(b),
        "timer.device" => crate::timer::register(b),
        "aros.library" => b.version(41), // posixc.library asks for 41; ArosInquireA stays a named refusal until a program needs it
        _ => b,
    }
}
