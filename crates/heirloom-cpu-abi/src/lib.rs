#![no_std]

/// 2 (2026-10-27): [`CpuStop`] carries the bus-fault count in its own field and a stall is its own kind
/// ([`STOP_STALL`]), and the table gains the context and diagnostics entries an engine needs to drive the CPU
/// through the table alone (one saved context per guest task; the recent-PC ring). Version 1 had no consumer.
pub const CPU_ABI_VERSION: u32 = 2;
pub const ARCH_M68K: u32 = 1;
pub const ARCH_PPC: u32 = 2;
pub const ARCH_ARM: u32 = 3;

pub const STOP_BUDGET: u32 = 1;
pub const STOP_TRAP: u32 = 2;
pub const STOP_FAULT: u32 = 3;
pub const STOP_HALT: u32 = 4;
/// The CPU stopped making progress: a slice retired no instruction (68k STOP, or a double-fault halt). `pc` is where.
pub const STOP_STALL: u32 = 5;

/// Bus callbacks supplied to a CPU library.
///
/// A callback return value of `0` indicates success; a nonzero value indicates
/// a bus fault.
#[repr(C)]
pub struct CpuBusCallbacks {
    pub ctx: *mut core::ffi::c_void,
    pub read8: extern "C" fn(ctx: *mut core::ffi::c_void, addr: u32, out: *mut u32) -> i32,
    pub read16: extern "C" fn(ctx: *mut core::ffi::c_void, addr: u32, out: *mut u32) -> i32,
    pub read32: extern "C" fn(ctx: *mut core::ffi::c_void, addr: u32, out: *mut u32) -> i32,
    pub write8: extern "C" fn(ctx: *mut core::ffi::c_void, addr: u32, value: u32) -> i32,
    pub write16: extern "C" fn(ctx: *mut core::ffi::c_void, addr: u32, value: u32) -> i32,
    pub write32: extern "C" fn(ctx: *mut core::ffi::c_void, addr: u32, value: u32) -> i32,
    pub on_trap: extern "C" fn(ctx: *mut core::ffi::c_void, pc: u32, opcode_or_trap: u32) -> i32,
}

#[repr(C)]
pub struct CpuInstance {
    _private: [u8; 0],
}

#[repr(C)]
pub struct CpuStop {
    pub kind: u32,
    pub pc: u32,
    /// STOP_TRAP: 1 = the guest blocked mid-call (the trap callback returned 3). Otherwise 0.
    pub detail: u32,
    /// Accesses the bus callbacks refused during this run (each read as 0 / dropped). Reported with ANY kind; a run
    /// that ended for another reason and also faulted says both.
    pub bus_faults: u32,
    pub insns: u64,
}

#[repr(C)]
pub struct CpuVTable {
    pub version: u32,
    /// The byte size of the table used by the library.
    pub size: u32,
    pub arch_id: u32,
    pub create: extern "C" fn(sub_arch: u32, bus: *const CpuBusCallbacks) -> *mut CpuInstance,
    pub destroy: extern "C" fn(*mut CpuInstance),
    pub reset: extern "C" fn(*mut CpuInstance),
    pub run: extern "C" fn(*mut CpuInstance, budget: u64, stop: *mut CpuStop) -> i32,
    pub interrupt: extern "C" fn(*mut CpuInstance, level: u32) -> i32,
    pub get_reg: extern "C" fn(*mut CpuInstance, reg: u32, out: *mut u64) -> i32,
    pub set_reg: extern "C" fn(*mut CpuInstance, reg: u32, value: u64) -> i32,
    pub invalidate_code: extern "C" fn(*mut CpuInstance, addr: u32, len: u32),
    /// Bytes one saved CPU context takes (the buffer size for `save_context` / `load_context`). 0 = not supported.
    pub context_size: extern "C" fn(*mut CpuInstance) -> u32,
    /// Copy the instance's current registers into `dst` (`len` >= `context_size`): 0, or -1 (bad instance, short or null
    /// buffer, no context yet). The bytes are opaque and valid only for the same library.
    pub save_context: extern "C" fn(*mut CpuInstance, dst: *mut u8, len: u32) -> i32,
    /// Make `src` (a buffer `save_context` filled) the instance's current registers: 0, or -1. The next `run` continues from it.
    pub load_context: extern "C" fn(*mut CpuInstance, src: *const u8, len: u32) -> i32,
    /// The addresses of the most recently executed instructions, oldest first, into `out` (at most `cap`): how many were
    /// written. Diagnostics only; 0 when the library keeps no ring.
    pub recent_pcs: extern "C" fn(*mut CpuInstance, out: *mut u32, cap: u32) -> u32,
}

pub enum OpenRefusal {
    NullTable,
    VersionTooOld { found: u32, required: u32 },
    TableTooSmall { found: u32, required: u32 },
    WrongArch { found: u32, required: u32 },
}

fn check_header(
    version: u32,
    required_version: u32,
    size: impl FnOnce() -> u32,
    arch_id: impl FnOnce() -> u32,
    arch: u32,
) -> Result<(), OpenRefusal> {
    if version < required_version {
        return Err(OpenRefusal::VersionTooOld {
            found: version,
            required: required_version,
        });
    }

    let size = size();
    let required = core::mem::size_of::<CpuVTable>() as u32;
    if size < required {
        return Err(OpenRefusal::TableTooSmall {
            found: size,
            required,
        });
    }

    let found = arch_id();
    if found != arch {
        return Err(OpenRefusal::WrongArch {
            found,
            required: arch,
        });
    }

    Ok(())
}

pub fn check(table: &CpuVTable, required_version: u32, arch: u32) -> Result<(), OpenRefusal> {
    check_header(
        table.version,
        required_version,
        || table.size,
        || table.arch_id,
        arch,
    )
}

/// Checks a possibly foreign vtable pointer without forming a Rust reference to
/// the complete table.
///
/// # Safety
///
/// `p` must be null or address readable, initialized ABI-header memory. A
/// successful result also requires the caller to trust and satisfy the
/// advertised table extent before invoking function pointers.
pub unsafe fn check_ptr(
    p: *const CpuVTable,
    required_version: u32,
    arch: u32,
) -> Result<(), OpenRefusal> {
    if p.is_null() {
        return Err(OpenRefusal::NullTable);
    }

    let version = core::ptr::addr_of!((*p).version).read_unaligned();
    check_header(
        version,
        required_version,
        || unsafe { core::ptr::addr_of!((*p).size).read_unaligned() },
        || unsafe { core::ptr::addr_of!((*p).arch_id).read_unaligned() },
        arch,
    )
}

pub enum StopKind {
    Budget,
    Trap,
    Fault,
    Halt,
    Stall,
}

impl StopKind {
    pub fn from_raw(raw_kind: u32) -> Option<StopKind> {
        match raw_kind {
            STOP_BUDGET => Some(StopKind::Budget),
            STOP_TRAP => Some(StopKind::Trap),
            STOP_FAULT => Some(StopKind::Fault),
            STOP_HALT => Some(StopKind::Halt),
            STOP_STALL => Some(StopKind::Stall),
            _ => None,
        }
    }
}

pub mod regs {
    pub mod m68k {
        pub const D0: u32 = 0;
        pub const D1: u32 = 1;
        pub const D2: u32 = 2;
        pub const D3: u32 = 3;
        pub const D4: u32 = 4;
        pub const D5: u32 = 5;
        pub const D6: u32 = 6;
        pub const D7: u32 = 7;
        pub const A0: u32 = 8;
        pub const A1: u32 = 9;
        pub const A2: u32 = 10;
        pub const A3: u32 = 11;
        pub const A4: u32 = 12;
        pub const A5: u32 = 13;
        pub const A6: u32 = 14;
        pub const A7: u32 = 15;
        pub const PC: u32 = 16;
        pub const SR: u32 = 17;
    }

    pub mod ppc {
        /// GPR `n` (0..32); `None` for any other index (a register number is never guessed).
        pub const fn gpr(n: u32) -> Option<u32> {
            if n < 32 { Some(n) } else { None }
        }

        pub const PC: u32 = 32;
        pub const LR: u32 = 33;
        pub const CTR: u32 = 34;
        pub const CR: u32 = 35;
        pub const XER: u32 = 36;
        pub const MSR: u32 = 37;
    }

    pub mod arm {
        /// R`n` (0..16); `None` for any other index.
        pub const fn r(n: u32) -> Option<u32> {
            if n < 16 { Some(n) } else { None }
        }

        pub const CPSR: u32 = 16;
    }
}