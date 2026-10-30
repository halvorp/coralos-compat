//! os-mac.library before bind_cpu (its own test binary: the CPU binding is process-wide): no machine is made, and the refusal
//! names the missing CPU - the library never falls back to a CPU it was not given, and never reaches an unbound one.
#![cfg(feature = "os-library")]

use core::ffi::c_void;
use heirloom_mac68k_engine::os_vtable;
use heirloom_os_abi::*;

extern "C" fn emit(_: *mut c_void, _: *const u8, _: usize) {}
extern "C" fn lease(_: *mut c_void, _: u32, _: i32, _: i32, _: *const u32, _: usize) -> u32 { 0 }

#[test]
fn an_unbound_library_makes_no_machine_and_names_the_missing_cpu() {
    let host = OsHost { ctx: core::ptr::null_mut(), emit, lease_canvas: lease, read_stream: no_streams };
    let (mut why, mut n) = ([0u8; 128], 0usize);
    let inst = (os_vtable().create)(core::ptr::null(), 0, &host, why.as_mut_ptr(), why.len(), &mut n);
    assert!(inst.is_null());
    let said = String::from_utf8_lossy(&why[..n]);
    assert!(said.contains("no CPU is bound"), "{said}");
}
