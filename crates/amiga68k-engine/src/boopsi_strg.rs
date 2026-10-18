//! strgclass: a gadgetclass whose instance data is a `struct StringInfo`. (Skeleton; filled in below.)
use crate::{machine::Amiga, Refusal};
use guestui_abi::GuestUi;
pub fn strg<U: GuestUi>(a: &mut Amiga<U>, cl: u32, o: u32, msg: u32) -> Result<u32, Refusal> { crate::boopsi_native::dispatch_super(a, cl, o, msg) }
