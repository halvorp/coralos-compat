//! The one C-ABI boundary between a Heirloom family engine PROCESS and its guest-OS library (`os-mac.library`, later
//! `os-ami.library`, `os-eka.library`): docs/heirloom/23-ENGINE-SPLIT.md, the owner's layer grid (2026-10-25) and doctrine
//! ONE-NAME-ONE-CONCERN-LOADED. The `heirloom-cpu-abi` shape: a flat, version-first `#[repr(C)]` table, an opaque instance,
//! callbacks with a context pointer, and no owning value crossing it.
//!
//! The engine opens two libraries - a `cpu-*.library` and an `os-*.library` - hands the CPU table to the OS library
//! ([`OsVTable::bind_cpu`]), creates a guest machine with an [`OsHost`] (where its outbound frames and canvas pixels go) and feeds it
//! heirloom-ui-abi wire frames ([`OsVTable::apply`]): the host's machine-session Controls and native Events in, the guest's UI calls,
//! Leases and Reports out. The engine never decodes the guest's UI and never sees a guest pointer.
//!
//! Version 1 is the machine face (the `GuestMachine` surface over frames). The Toolbox-below-the-glue face macppc.engine will need
//! (`dispatch(ctx, trap, frame)`, doc 23 2.7) is a later version's addition, made when its consumer exists.
#![no_std]

use core::ffi::c_void;
pub use heirloom_cpu_abi::CpuVTable;
/// The CPU contract the bound table follows (an engine opens its cpu-*.library at `cpu_abi::CPU_ABI_VERSION`).
pub use heirloom_cpu_abi as cpu_abi;

#[cfg(feature = "serve")]
extern crate alloc;
#[cfg(feature = "serve")]
pub mod serve;

pub const OS_ABI_VERSION: u32 = 1;
/// The guest family a table serves: the manifest's `(platform "<family>")` token, as a number.
pub const FAMILY_MAC68K: u32 = 1;
pub const FAMILY_AMI68K: u32 = 2;
pub const FAMILY_MACPPC: u32 = 3;
pub const FAMILY_EKAARM: u32 = 4;

/// `bind_cpu`: bound (or the same table again).
pub const BIND_OK: i32 = 0;
/// `bind_cpu`: the CPU table is null, too old, too small or another architecture.
pub const BIND_REFUSED: i32 = 1;
/// `bind_cpu`: a different CPU table is already bound in this library.
pub const BIND_ALREADY: i32 = 2;

/// `apply`: every frame was applied.
pub const APPLY_OK: i32 = 0;
/// `apply`: a frame did not decode, or is not one an engine takes (a UI call, a lease, a report); the frames before it were applied.
pub const APPLY_BAD_FRAME: i32 = -1;
/// `apply`: a null instance or a null batch with a nonzero length.
pub const APPLY_BAD_ARGS: i32 = -2;

/// Where a guest machine's outbound traffic goes: the engine's side. `emit` is called once per complete wire frame, in the order
/// the guest made the calls; `lease_canvas` copies one window's canvas into the shared region the host maps and returns the region
/// id its Lease frame names (0 = not leased; the host then shows no pixels for that present and says so).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct OsHost {
    pub ctx: *mut c_void,
    pub emit: extern "C" fn(ctx: *mut c_void, frame: *const u8, len: usize),
    pub lease_canvas: extern "C" fn(ctx: *mut c_void, win: u32, width: i32, height: i32, pixels: *const u32, count: usize) -> u32,
}

#[repr(C)]
pub struct OsInstance {
    _private: [u8; 0],
}

#[repr(C)]
pub struct OsVTable {
    pub version: u32,
    /// The byte size of the table the library was built with.
    pub size: u32,
    pub family: u32,
    /// Bind the CPU library's table this library's guest runs on. Call once, before `create`; see BIND_*.
    pub bind_cpu: extern "C" fn(cpu: *const CpuVTable) -> i32,
    /// A fresh guest machine of machine profile `profile` (UTF-8, `len` bytes; empty = the family's default) whose traffic goes to
    /// `host` (copied). Null on refusal, with the reason (UTF-8) in `why` (at most `why_cap` bytes; `*why_len` = bytes written).
    pub create: extern "C" fn(profile: *const u8, len: usize, host: *const OsHost, why: *mut u8, why_cap: usize, why_len: *mut usize) -> *mut OsInstance,
    pub destroy: extern "C" fn(*mut OsInstance),
    /// Apply a batch of concatenated wire frames in order; see APPLY_*.
    pub apply: extern "C" fn(*mut OsInstance, frames: *const u8, len: usize) -> i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenRefusal {
    NullTable,
    VersionTooOld { found: u32, required: u32 },
    TableTooSmall { found: u32, required: u32 },
    WrongFamily { found: u32, required: u32 },
}

impl OpenRefusal {
    pub fn name(&self) -> &'static str {
        match self {
            OpenRefusal::NullTable => "null-table",
            OpenRefusal::VersionTooOld { .. } => "version-too-old",
            OpenRefusal::TableTooSmall { .. } => "table-too-small",
            OpenRefusal::WrongFamily { .. } => "wrong-family",
        }
    }
}

/// Check a possibly foreign table pointer (the `coral_lib_open` result) before any of its functions is called, without forming a
/// reference to the whole table first.
///
/// # Safety
/// `p` must be null or point to readable, initialised memory holding at least the table's three leading `u32`s.
pub unsafe fn check_ptr(p: *const OsVTable, required_version: u32, family: u32) -> Result<(), OpenRefusal> {
    if p.is_null() {
        return Err(OpenRefusal::NullTable);
    }
    // SAFETY: the caller guarantees the three header words are readable.
    let (version, size, found) = unsafe {
        (
            core::ptr::addr_of!((*p).version).read_unaligned(),
            core::ptr::addr_of!((*p).size).read_unaligned(),
            core::ptr::addr_of!((*p).family).read_unaligned(),
        )
    };
    if version < required_version {
        return Err(OpenRefusal::VersionTooOld { found: version, required: required_version });
    }
    let required = core::mem::size_of::<OsVTable>() as u32;
    if size < required {
        return Err(OpenRefusal::TableTooSmall { found: size, required });
    }
    if found != family {
        return Err(OpenRefusal::WrongFamily { found, required: family });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    extern "C" fn bind(_: *const CpuVTable) -> i32 { BIND_OK }
    extern "C" fn create(_: *const u8, _: usize, _: *const OsHost, _: *mut u8, _: usize, _: *mut usize) -> *mut OsInstance { core::ptr::null_mut() }
    extern "C" fn destroy(_: *mut OsInstance) {}
    extern "C" fn apply(_: *mut OsInstance, _: *const u8, _: usize) -> i32 { APPLY_OK }

    fn table(version: u32, size: u32, family: u32) -> OsVTable {
        OsVTable { version, size, family, bind_cpu: bind, create, destroy, apply }
    }

    #[test]
    fn the_header_is_checked_before_any_entry() {
        let full = core::mem::size_of::<OsVTable>() as u32;
        let ok = table(OS_ABI_VERSION, full, FAMILY_MAC68K);
        assert_eq!(unsafe { check_ptr(&ok, OS_ABI_VERSION, FAMILY_MAC68K) }, Ok(()));
        assert_eq!(unsafe { check_ptr(core::ptr::null(), 1, FAMILY_MAC68K) }, Err(OpenRefusal::NullTable));
        let old = table(0, full, FAMILY_MAC68K);
        assert_eq!(unsafe { check_ptr(&old, 1, FAMILY_MAC68K) }, Err(OpenRefusal::VersionTooOld { found: 0, required: 1 }));
        let small = table(1, 12, FAMILY_MAC68K);
        assert_eq!(unsafe { check_ptr(&small, 1, FAMILY_MAC68K) }, Err(OpenRefusal::TableTooSmall { found: 12, required: full }));
        let amiga = table(1, full, FAMILY_AMI68K);
        assert_eq!(unsafe { check_ptr(&amiga, 1, FAMILY_MAC68K) }, Err(OpenRefusal::WrongFamily { found: FAMILY_AMI68K, required: FAMILY_MAC68K }));
    }
}
