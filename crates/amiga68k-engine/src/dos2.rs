//! Registration of the second half of dos.library: variables, the DOS device list, patterns, ExAll and the small
//! services DOpus-class programs call. Vectors are registered by NAME (libtable::named), never by number.
use crate::libtable::LibraryBuilder;
use guestui_abi::GuestUi;

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.with(crate::vars::register::<U>).with(crate::doslist::register::<U>).with(crate::devproc::register::<U>).with(crate::dos_match::register::<U>)
}
