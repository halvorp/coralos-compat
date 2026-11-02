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

pub const OS_ABI_VERSION: u32 = 3;
/// The first version with the multi-family entries ([`OsVTable::cpu_abi_for_family`] and after): a family other than the header's is
/// only served from this version on.
pub const OS_ABI_MULTI_FAMILY: u32 = 2;
/// The first version with [`OsVTable::attach_gl`], [`OsVTable::perf`] and [`OsVTable::set_code_mem`]: a machine's guest GL can
/// reach the engine's GL worker, its stage clock can be read by the engine process, and its CPU can run in the engine's executable
/// memory.
pub const OS_ABI_GL: u32 = 3;

/// `attach_gl`: the machine's guest GL now goes to the host's [`GlHost`].
pub const GL_ATTACH_OK: i32 = 0;
/// `attach_gl`: this machine's family has no guest GL (its OS offers none), or it already has a worker.
pub const GL_ATTACH_REFUSED: i32 = 1;
/// `attach_gl`: a null instance or a null host.
pub const GL_ATTACH_BAD_ARGS: i32 = 2;

/// `set_code_mem`: the machine's CPU will run tier 2 (the JIT) in the engine's executable memory.
pub const CODE_MEM_OK: i32 = 0;
/// `set_code_mem`: this machine's family has no JIT (its CPU contract carries no executable memory).
pub const CODE_MEM_REFUSED: i32 = 1;
/// `set_code_mem`: a null instance or a null record.
pub const CODE_MEM_BAD_ARGS: i32 = 2;

/// `perf`: one finished stage-clock line was written.
pub const PERF_LINE: i32 = 0;
/// `perf`: the stage clock runs (switched on by this call if it was off) and has no finished line yet.
pub const PERF_NONE: i32 = 1;
/// `perf`: this machine's family has no stage clock, or a null instance / a zero TSC rate.
pub const PERF_REFUSED: i32 = 2;
/// [`OsVTable::cpu_prof`]: the record was filled.
pub const CPU_PROF_OK: i32 = 0;
/// This machine's CPU keeps no such counters (not a PowerPC machine), or the instance is unknown.
pub const CPU_PROF_REFUSED: i32 = 1;
/// A null instance or record.
pub const CPU_PROF_BAD_ARGS: i32 = 2;

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

/// The engine's transport to its GL worker (heirloom-gl.engine, the engine process's own `gl.req` / `gl.evt` session; doc 23 slice
/// 4.3): `transact` sends one request batch (whole heirloom-gl-abi frames) and returns the reply's GL frames - `*reply_len` bytes at
/// the returned pointer, lent until the next call. A null pointer (or a zero length for a request that needs an answer) = the worker
/// is gone or its ring failed: the guest's GL call is refused, never waited on.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GlHost {
    pub ctx: *mut c_void,
    pub transact: extern "C" fn(ctx: *mut c_void, request: *const u8, len: usize, reply_len: *mut usize) -> *const u8,
}

impl GlHost {
    /// One request through the host: the reply's bytes, lent until the next call (the `&mut` borrow); empty = the worker is gone.
    pub fn transact(&mut self, request: &[u8]) -> &[u8] {
        let mut n = 0usize;
        let p = (self.transact)(self.ctx, request.as_ptr(), request.len(), &mut n);
        if p.is_null() || n == 0 {
            return &[];
        }
        // SAFETY: the host lends `n` readable bytes at `p` until its next transact, which needs this `&mut self` again.
        unsafe { core::slice::from_raw_parts(p, n) }
    }
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
    // ---- version 3 (OS_ABI_GL): appended only ----
    /// Give a machine its GL worker: the guest's GL batches go to `gl` (copied) from now on. See GL_ATTACH_*. A machine with no
    /// worker keeps its family's no-GL behaviour (a Mac's AGL offers no pixel format).
    pub attach_gl: extern "C" fn(inst: *mut OsInstance, gl: *const GlHost) -> i32,
    /// The machine's stage clock (doc 22 phase 5, the `[HEIRLOOM-PERF ...]` line): the first call switches it on (the library reads
    /// the CPU's own time-stamp counter); each call writes at most one finished line (UTF-8, at most `cap` bytes at `line`,
    /// `*line_len` = bytes written), formatted at `tsc_per_us` TSC ticks per microsecond (the ENGINE measures the rate). See PERF_*.
    pub perf: extern "C" fn(inst: *mut OsInstance, tsc_per_us: u64, line: *mut u8, cap: usize, line_len: *mut usize) -> i32,
    /// Executable memory for the machine's CPU (CPU ABI v3 [`ppc::PpcCodeMem`], copied; its `ctx` and functions must outlive the
    /// instance): a PowerPC machine runs tier 2 in it from now on (its named input `cpu-tier` can still force another tier).
    /// See CODE_MEM_*.
    pub set_code_mem: extern "C" fn(inst: *mut OsInstance, mem: *const ppc::PpcCodeMem) -> i32,
    /// APPENDED 2026-11-02 (h86), present when the table's `size` > [`OS_TABLE_V3_SIZE`] (an engine checks before calling; the
    /// version stays 3): the machine's CPU counters as cpu-ppc.library's `read_prof` last gave them (where the instructions ran:
    /// tier 2 / tier 1 / tier 0, the executable-memory calls), into the engine's record under [`ppc::PpcProf`]'s size contract
    /// (the caller sets `size`; 0 = the first version's fields). See CPU_PROF_*.
    pub cpu_prof: extern "C" fn(inst: *mut OsInstance, out: *mut ppc::PpcProf) -> i32,
}

/// The byte size of a version-1 table (the four header words' entries up to `apply`).
pub const OS_TABLE_V1_SIZE: u32 = core::mem::offset_of!(OsVTable, cpu_abi_for_family) as u32;
/// The byte size of a version-2 table (everything before `attach_gl`).
pub const OS_TABLE_V2_SIZE: u32 = core::mem::offset_of!(OsVTable, attach_gl) as u32;
/// The byte size of a version-3 table as first published (everything before `cpu_prof`): what [`check_ptr`] requires of a v3
/// table, so an engine built with `cpu_prof` still opens a library built before it.
pub const OS_TABLE_V3_SIZE: u32 = core::mem::offset_of!(OsVTable, cpu_prof) as u32;
/// Whether a table checked by [`check_ptr`] carries [`OsVTable::cpu_prof`].
pub fn has_cpu_prof(t: &OsVTable) -> bool { t.size as usize >= core::mem::offset_of!(OsVTable, cpu_prof) + core::mem::size_of::<usize>() }

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
/// (the v1 entries for version 1, the v2 entries for version 2, the v3 entries as first published - OS_TABLE_V3_SIZE - from
/// version 3: an entry appended since is used only when [`has_cpu_prof`]-style size checks say it is there), serving `family`: the family its header names, or - a version-2
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
    let required = if version >= OS_ABI_GL {
        OS_TABLE_V3_SIZE
    } else if multi {
        OS_TABLE_V2_SIZE
    } else {
        OS_TABLE_V1_SIZE
    };
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
    extern "C" fn attach(_: *mut OsInstance, _: *const GlHost) -> i32 { GL_ATTACH_REFUSED }
    extern "C" fn perf(_: *mut OsInstance, _: u64, _: *mut u8, _: usize, _: *mut usize) -> i32 { PERF_REFUSED }
    extern "C" fn code_mem(_: *mut OsInstance, _: *const ppc::PpcCodeMem) -> i32 { CODE_MEM_REFUSED }
    extern "C" fn cpu_prof(_: *mut OsInstance, _: *mut ppc::PpcProf) -> i32 { CPU_PROF_REFUSED }

    fn table(version: u32, size: u32, family: u32) -> OsVTable {
        OsVTable { version, size, family, bind_cpu: bind, create, destroy, apply, cpu_abi_for_family: serves_ppc, bind_cpu_ppc: bind_ppc, create_for_family: create_for, attach_gl: attach, perf, set_code_mem: code_mem, cpu_prof }
    }

    /// `cpu_prof` was appended to version 3: a v3 library built before it still opens (its size is OS_TABLE_V3_SIZE) and an
    /// engine sees it has no `cpu_prof` (never calls past the table's end); a table with it says so.
    #[test]
    fn a_v3_table_from_before_cpu_prof_still_opens_and_says_it_has_none() {
        let before = table(OS_ABI_VERSION, OS_TABLE_V3_SIZE, FAMILY_MAC68K);
        assert_eq!(unsafe { check_ptr(&before, OS_ABI_VERSION, FAMILY_MACPPC) }, Ok(()));
        assert!(!has_cpu_prof(&before));
        let now = table(OS_ABI_VERSION, core::mem::size_of::<OsVTable>() as u32, FAMILY_MAC68K);
        assert!(has_cpu_prof(&now));
        let short = table(OS_ABI_VERSION, OS_TABLE_V3_SIZE - 8, FAMILY_MAC68K);
        assert_eq!(unsafe { check_ptr(&short, OS_ABI_VERSION, FAMILY_MACPPC) }, Err(OpenRefusal::TableTooSmall { found: OS_TABLE_V3_SIZE - 8, required: OS_TABLE_V3_SIZE }));
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
        assert_eq!(unsafe { check_ptr(&v1, OS_ABI_VERSION, FAMILY_MAC68K) }, Err(OpenRefusal::VersionTooOld { found: 1, required: OS_ABI_VERSION }));
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
        assert_eq!(unsafe { check_ptr(&short, OS_ABI_VERSION, FAMILY_MACPPC) }, Err(OpenRefusal::TableTooSmall { found: OS_TABLE_V1_SIZE, required: OS_TABLE_V3_SIZE }));
    }

    #[test]
    fn a_version_three_table_must_carry_attach_gl_and_a_version_two_one_is_too_old_for_it() {
        let full = core::mem::size_of::<OsVTable>() as u32;
        // a v2 library (built before attach_gl) opens at version 2 with its own, smaller size ...
        let v2 = table(OS_ABI_MULTI_FAMILY, OS_TABLE_V2_SIZE, FAMILY_MAC68K);
        assert_eq!(unsafe { check_ptr(&v2, OS_ABI_MULTI_FAMILY, FAMILY_MACPPC) }, Ok(()));
        // ... is too old for an engine that attaches GL ...
        assert_eq!(unsafe { check_ptr(&v2, OS_ABI_GL, FAMILY_MAC68K) }, Err(OpenRefusal::VersionTooOld { found: 2, required: OS_ABI_GL }));
        // ... and a v3 header without the attach_gl entry is refused before it is read.
        let short = table(OS_ABI_GL, OS_TABLE_V2_SIZE, FAMILY_MAC68K);
        assert_eq!(unsafe { check_ptr(&short, OS_ABI_GL, FAMILY_MAC68K) }, Err(OpenRefusal::TableTooSmall { found: OS_TABLE_V2_SIZE, required: OS_TABLE_V3_SIZE }));
        // (a full table today is larger than a v3 one as first published: cpu_prof was appended)
        assert!(full > OS_TABLE_V3_SIZE);
    }
}
