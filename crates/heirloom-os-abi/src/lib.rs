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
//! Version 1 is the machine face (the `GuestMachine` surface over frames) for the one family in the header. Version 2 (2026-10-30
//! ruling, Astra + Argon: /build/patches/h81/macppc-engine-split.consult.md) appends a MULTI-FAMILY machine face: one library may
//! serve more than one family (os-mac.library: mac68k AND macppc, one Toolbox), each family names the CPU contract it runs on
//! ([`OsVTable::cpu_abi_for_family`]), a PowerPC family binds a cpu-ppc.library table (CPU ABI v3, [`ppc::PpcVTable`]) through
//! [`OsVTable::bind_cpu_ppc`], and [`OsVTable::create_for_family`] makes a machine of a named family. The v1 entries keep their
//! meaning for the header's family. A trap-dispatch face (doc 23 2.7) is not part of it: the macppc personality reaches into the
//! Toolbox's state, not through a call surface, so it lives in the same library.
#![no_std]

use core::ffi::c_void;
pub use heirloom_cpu_abi::CpuVTable;
/// The CPU contract the bound table follows (an engine opens its cpu-*.library at `cpu_abi::CPU_ABI_VERSION`).
pub use heirloom_cpu_abi as cpu_abi;
/// The PowerPC CPU contract (CPU ABI v3) a PowerPC family binds.
pub use heirloom_cpu_abi::ppc;

#[cfg(feature = "serve")]
extern crate alloc;
#[cfg(feature = "serve")]
pub mod serve;

pub const OS_ABI_VERSION: u32 = 2;
/// The first version with the multi-family entries ([`OsVTable::cpu_abi_for_family`] and after): a family other than the header's is
/// only served from this version on.
pub const OS_ABI_MULTI_FAMILY: u32 = 2;

/// `cpu_abi_for_family`: the CPU contract a family runs on - a [`CpuVTable`] (CPU ABI v2, `bind_cpu`).
pub const CPU_CONTRACT_V2: u32 = 2;
/// `cpu_abi_for_family`: a [`ppc::PpcVTable`] (CPU ABI v3, the flat-RAM PowerPC family, `bind_cpu_ppc`).
pub const CPU_CONTRACT_PPC: u32 = 3;
/// `cpu_abi_for_family`: the family is served (`*contract` and `*min_version` written).
pub const FAMILY_SERVED: i32 = 0;
/// `cpu_abi_for_family`: this library serves no such family.
pub const FAMILY_NOT_SERVED: i32 = 1;
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
///
/// `read_stream` (OS ABI v2) reads a STREAM file of the staged volume (heirloom-volume `Kind::Stream`: a big install file whose
/// bytes stay in storage, e.g. Warcraft III's 401 MB War3.mpq): up to `cap` bytes of the file at `locator` (UTF-8, `locator_len`
/// bytes, relative to the storage root the engine was given) from `offset` into `dst`; the count read, or a negative number when
/// the engine refused the locator or its storage failed (the guest's file layer reports an I/O error).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct OsHost {
    pub ctx: *mut c_void,
    pub emit: extern "C" fn(ctx: *mut c_void, frame: *const u8, len: usize),
    pub lease_canvas: extern "C" fn(ctx: *mut c_void, win: u32, width: i32, height: i32, pixels: *const u32, count: usize) -> u32,
    pub read_stream: extern "C" fn(ctx: *mut c_void, locator: *const u8, locator_len: usize, offset: u64, dst: *mut u8, cap: usize) -> isize,
}

/// A host's `read_stream` that reads nothing: for a host with no storage (a test, a family without streams). Every read is refused.
pub extern "C" fn no_streams(_: *mut c_void, _: *const u8, _: usize, _: u64, _: *mut u8, _: usize) -> isize {
    -1
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
    // ---- version 2 (OS_ABI_MULTI_FAMILY): appended only ----
    /// Whether this library serves `family`, and if so the CPU contract its machines run on (`CPU_CONTRACT_*`) and the lowest
    /// version of that contract's table it binds; see FAMILY_*.
    pub cpu_abi_for_family: extern "C" fn(family: u32, contract: *mut u32, min_version: *mut u32) -> i32,
    /// Bind a cpu-ppc.library table (checked with `ppc::check_ppc`) for the families whose contract is CPU_CONTRACT_PPC; see BIND_*.
    /// A library with no such family refuses it.
    pub bind_cpu_ppc: extern "C" fn(cpu: *const ppc::PpcVTable) -> i32,
    /// `create` for a named family this library serves (`create` is this with the header's family). `destroy` and `apply` take an
    /// instance of any family.
    pub create_for_family: extern "C" fn(family: u32, profile: *const u8, len: usize, host: *const OsHost, why: *mut u8, why_cap: usize, why_len: *mut usize) -> *mut OsInstance,
}

/// The byte size of a version-1 table (the four header words' entries up to `apply`).
pub const OS_TABLE_V1_SIZE: u32 = core::mem::offset_of!(OsVTable, cpu_abi_for_family) as u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenRefusal {
    NullTable,
    VersionTooOld { found: u32, required: u32 },
    TableTooSmall { found: u32, required: u32 },
    WrongFamily { found: u32, required: u32 },
    /// A multi-family table that does not serve the family asked for (`header` = the family the table names first).
    FamilyNotServed { header: u32, required: u32 },
}

impl OpenRefusal {
    pub fn name(&self) -> &'static str {
        match self {
            OpenRefusal::NullTable => "null-table",
            OpenRefusal::VersionTooOld { .. } => "version-too-old",
            OpenRefusal::TableTooSmall { .. } => "table-too-small",
            OpenRefusal::WrongFamily { .. } => "wrong-family",
            OpenRefusal::FamilyNotServed { .. } => "family-not-served",
        }
    }
}

/// Check a possibly foreign table pointer (the `coral_lib_open` result) before any of its functions is called, without forming a
/// reference to the whole table first. A table of version `required_version` or later, at least as large as that version's entries
/// (the v1 entries for version 1, the whole table from version 2), serving `family`: the family its header names, or - a version-2
/// table only - another family its `cpu_abi_for_family` says it serves. A version-1 table serves its header's family alone.
///
/// # Safety
/// `p` must be null or point to readable, initialised memory holding at least the table's three leading `u32`s, and the `size`
/// bytes its header states.
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
    let multi = version >= OS_ABI_MULTI_FAMILY;
    let required = if multi { core::mem::size_of::<OsVTable>() as u32 } else { OS_TABLE_V1_SIZE };
    if size < required {
        return Err(OpenRefusal::TableTooSmall { found: size, required });
    }
    if found == family {
        return Ok(());
    }
    if !multi {
        return Err(OpenRefusal::WrongFamily { found, required: family });
    }
    // SAFETY: a version-2 table of at least the full size: the entry is present and readable.
    let serves = unsafe { core::ptr::addr_of!((*p).cpu_abi_for_family).read_unaligned() };
    let (mut contract, mut min) = (0u32, 0u32);
    match serves(family, &mut contract, &mut min) {
        FAMILY_SERVED => Ok(()),
        _ => Err(OpenRefusal::FamilyNotServed { header: found, required: family }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    extern "C" fn bind(_: *const CpuVTable) -> i32 { BIND_OK }
    extern "C" fn create(_: *const u8, _: usize, _: *const OsHost, _: *mut u8, _: usize, _: *mut usize) -> *mut OsInstance { core::ptr::null_mut() }
    extern "C" fn destroy(_: *mut OsInstance) {}
    extern "C" fn apply(_: *mut OsInstance, _: *const u8, _: usize) -> i32 { APPLY_OK }
    /// Serves the PowerPC Mac beside its header's family; nothing else.
    extern "C" fn serves_ppc(family: u32, contract: *mut u32, min: *mut u32) -> i32 {
        if family != FAMILY_MACPPC { return FAMILY_NOT_SERVED; }
        // SAFETY: the test passes valid pointers.
        unsafe { (*contract, *min) = (CPU_CONTRACT_PPC, 1) };
        FAMILY_SERVED
    }
    extern "C" fn bind_ppc(_: *const ppc::PpcVTable) -> i32 { BIND_OK }
    extern "C" fn create_for(_: u32, _: *const u8, _: usize, _: *const OsHost, _: *mut u8, _: usize, _: *mut usize) -> *mut OsInstance { core::ptr::null_mut() }

    fn table(version: u32, size: u32, family: u32) -> OsVTable {
        OsVTable { version, size, family, bind_cpu: bind, create, destroy, apply, cpu_abi_for_family: serves_ppc, bind_cpu_ppc: bind_ppc, create_for_family: create_for }
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
        assert_eq!(unsafe { check_ptr(&small, 1, FAMILY_MAC68K) }, Err(OpenRefusal::TableTooSmall { found: 12, required: OS_TABLE_V1_SIZE }));
        let amiga = table(1, OS_TABLE_V1_SIZE, FAMILY_AMI68K);
        assert_eq!(unsafe { check_ptr(&amiga, 1, FAMILY_MAC68K) }, Err(OpenRefusal::WrongFamily { found: FAMILY_AMI68K, required: FAMILY_MAC68K }));
    }

    #[test]
    fn a_version_one_table_is_its_header_family_alone_and_needs_only_its_own_entries() {
        // A v1 library (built before the multi-family entries) still opens for its own family at version 1 ...
        let v1 = table(1, OS_TABLE_V1_SIZE, FAMILY_MAC68K);
        assert_eq!(unsafe { check_ptr(&v1, 1, FAMILY_MAC68K) }, Ok(()));
        // ... is too old for an engine that needs version 2 ...
        assert_eq!(unsafe { check_ptr(&v1, OS_ABI_VERSION, FAMILY_MAC68K) }, Err(OpenRefusal::VersionTooOld { found: 1, required: 2 }));
        // ... and never serves another family, even one a v2 table of the same library would (its entries are not there to ask).
        assert_eq!(unsafe { check_ptr(&v1, 1, FAMILY_MACPPC) }, Err(OpenRefusal::WrongFamily { found: FAMILY_MAC68K, required: FAMILY_MACPPC }));
    }

    #[test]
    fn a_version_two_table_serves_the_families_it_says_and_no_other() {
        let full = core::mem::size_of::<OsVTable>() as u32;
        let mac = table(OS_ABI_VERSION, full, FAMILY_MAC68K);
        assert_eq!(unsafe { check_ptr(&mac, OS_ABI_VERSION, FAMILY_MACPPC) }, Ok(()));
        assert_eq!(unsafe { check_ptr(&mac, OS_ABI_VERSION, FAMILY_EKAARM) }, Err(OpenRefusal::FamilyNotServed { header: FAMILY_MAC68K, required: FAMILY_EKAARM }));
        // A v2 header with a v1 size is refused before its v2 entries are read.
        let short = table(OS_ABI_VERSION, OS_TABLE_V1_SIZE, FAMILY_MAC68K);
        assert_eq!(unsafe { check_ptr(&short, OS_ABI_VERSION, FAMILY_MACPPC) }, Err(OpenRefusal::TableTooSmall { found: OS_TABLE_V1_SIZE, required: full }));
    }
}
