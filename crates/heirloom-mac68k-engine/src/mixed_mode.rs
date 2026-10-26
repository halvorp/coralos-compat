//! Mixed Mode routine descriptors: the ONE parser, the constructors and the resolver (docs/MACPPC-PLUMBING.md, "Mixed Mode";
//! the ruling in docs/consults/2026-10-20-mixed-mode.md).
//!
//! A Universal Procedure Pointer is, for PowerPC code, the address of a RoutineDescriptor in guest memory (Inside Macintosh:
//! PowerPC System Software, "Mixed Mode Manager"; MixedMode.h). Big-endian, 2-byte packed:
//!
//! | offset | field | |
//! |---|---|---|
//! | 0 | `goMixedModeTrap: u16` | `$AAFE`, the `_MixedModeMagic` trap word (68k code that calls the UPP traps into Mixed Mode) |
//! | 2 | `version: i8` | 7 (`kRoutineDescriptorVersion`) |
//! | 3 | `routineDescriptorFlags: u8` | `kSelectorsAreIndexable` = 1 |
//! | 4 | `reserved1: u32`, 8 `reserved2: u8` | |
//! | 9 | `selectorInfo: u8` | |
//! | 10 | `routineCount: u16` | the INDEX of the last record: 0 = one record |
//! | 12 | `routineRecords[routineCount + 1]` | 20 bytes each, below |
//!
//! A RoutineRecord: +0 `procInfo: u32`, +4 `reserved1: i8`, +5 `ISA: u8` (`kM68kISA` 0, `kPowerPCISA` 1), +6 `routineFlags: u16`,
//! +8 `procDescriptor: u32` (a PowerPC record: the TVector; a 68k record: the code), +12 `reserved2: u32`, +16 `selector: u32`.
//!
//! The constructors build what the Universal Interfaces' `BUILD_ROUTINE_DESCRIPTOR` does: one record, flags
//! `kProcDescriptorIsAbsolute | kFragmentIsPrepared | kUseNativeISA`. A descriptor the program built itself (a static one in
//! its data section) is as valid as an allocated one: the allocation ledger says who may FREE a block, never what may be called.
use crate::machine::Mac;
use heirloom_mem_flat32::GuestMem;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_ui_abi::GuestUi;

pub const GO_MIXED_MODE_TRAP: u16 = 0xAAFE;
pub const ROUTINE_DESCRIPTOR_VERSION: u8 = 7;
pub const HEADER_LEN: u32 = 12;
pub const RECORD_LEN: u32 = 20;
/// `kM68kISA`, `kPowerPCISA`.
pub const ISA_M68K: u8 = 0;
pub const ISA_PPC: u8 = 1;
/// `kUseNativeISA`: the routine runs on the ISA the record names (the only flag the constructors set; 0 is absolute + prepared).
pub const USE_NATIVE_ISA: u16 = 0x0004;
/// `kProcDescriptorIsRelative`, `kFragmentNeedsPreparing`: forms this engine does not resolve (named refusals).
pub const PROC_DESCRIPTOR_IS_RELATIVE: u16 = 0x0001;
pub const FRAGMENT_NEEDS_PREPARING: u16 = 0x0002;
/// The most records one descriptor may carry here: a fat descriptor has two; a selector-dispatched one (a manager's dispatch
/// table) may have more. A bound on the parse, not on the guest.
pub const MAX_RECORDS: u16 = 256;

/// One routine record, decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Record { pub proc_info: u32, pub isa: u8, pub flags: u16, pub proc_descriptor: u32, pub selector: u32 }

/// A decoded descriptor (owned values: nothing borrows guest memory).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Descriptor { pub at: u32, pub flags: u8, pub selector_info: u8, pub records: Vec<Record> }

/// Why a UPP cannot be resolved; every variant names the address it is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MixedModeError {
    /// Guest memory could not be read at this address (the descriptor runs off the end, or the UPP is wild).
    Unreadable(u32),
    /// Not a routine descriptor: no `$AAFE` at the UPP. Here a raw 68k procedure pointer (what a 68k program passes).
    NotADescriptor(u32),
    /// The version byte is not 7.
    Version { at: u32, version: u8 },
    /// More records than [`MAX_RECORDS`].
    TooManyRecords { at: u32, count: u32 },
    /// No record this engine can run: the 68k-only and relative/unprepared forms (NEXT-NEEDED until cross-ISA calls exist).
    NoRunnableRecord { at: u32, why: String },
    /// `DisposeRoutineDescriptor` of a block the constructors did not make (or made and already freed).
    NotOwned(u32),
    /// The Memory Manager had no room for a new descriptor.
    NoMemory,
}

impl core::fmt::Display for MixedModeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unreadable(a) => write!(f, "routine descriptor at {a:#x}: guest memory unreadable"),
            Self::NotADescriptor(a) => write!(f, "UPP {a:#x} is not a routine descriptor (no $AAFE): a raw 68k procedure"),
            Self::Version { at, version } => write!(f, "routine descriptor at {at:#x}: version {version}, not 7"),
            Self::TooManyRecords { at, count } => write!(f, "routine descriptor at {at:#x}: {count} records (the parse allows {MAX_RECORDS})"),
            Self::NoRunnableRecord { at, why } => write!(f, "routine descriptor at {at:#x}: {why}"),
            Self::NotOwned(a) => write!(f, "DisposeRoutineDescriptor({a:#x}): not a descriptor NewRoutineDescriptor made (or freed already)"),
            Self::NoMemory => write!(f, "NewRoutineDescriptor: no memory for the descriptor"),
        }
    }
}

fn rd16(mem: &GuestMem, at: u32) -> Result<u16, MixedModeError> { mem.read_u16(at).map_err(|_| MixedModeError::Unreadable(at)) }
fn rd32(mem: &GuestMem, at: u32) -> Result<u32, MixedModeError> { mem.read_u32(at).map_err(|_| MixedModeError::Unreadable(at)) }
fn rd8(mem: &GuestMem, at: u32) -> Result<u8, MixedModeError> { mem.read_u8(at).map_err(|_| MixedModeError::Unreadable(at)) }

/// THE parser. Every read is bounds-checked by guest memory; record offsets are computed with checked arithmetic.
pub fn parse(mem: &GuestMem, at: u32) -> Result<Descriptor, MixedModeError> {
    if rd16(mem, at)? != GO_MIXED_MODE_TRAP { return Err(MixedModeError::NotADescriptor(at)); }
    let version = rd8(mem, at + 2)?;
    if version != ROUTINE_DESCRIPTOR_VERSION { return Err(MixedModeError::Version { at, version }); }
    let flags = rd8(mem, at + 3)?;
    let selector_info = rd8(mem, at + 9)?;
    let last = rd16(mem, at + 10)?;
    let count = u32::from(last) + 1;
    if last >= MAX_RECORDS { return Err(MixedModeError::TooManyRecords { at, count }); }
    let mut records = Vec::with_capacity(count as usize);
    for i in 0..count {
        let r = at.checked_add(HEADER_LEN + i * RECORD_LEN).ok_or(MixedModeError::Unreadable(at))?;
        records.push(Record { proc_info: rd32(mem, r)?, isa: rd8(mem, r + 5)?, flags: rd16(mem, r + 6)?, proc_descriptor: rd32(mem, r + 8)?, selector: rd32(mem, r + 16)? });
    }
    Ok(Descriptor { at, flags, selector_info, records })
}

/// The record a PowerPC caller runs: the PowerPC record of a one-record or fat descriptor whose procedure pointer is absolute
/// and prepared. A 68k-only descriptor, a relative or unprepared record, and a selector-dispatched descriptor are named
/// refusals (owner heirloom-macppc-engine; delete-when cross-ISA calls and selector dispatch have a witness).
pub fn ppc_record(d: &Descriptor) -> Result<Record, MixedModeError> {
    if d.records.len() > 2 || d.flags & 1 != 0 {
        return Err(MixedModeError::NoRunnableRecord { at: d.at, why: format!("a selector-dispatched descriptor ({} records, flags {:#x}) is not resolved yet", d.records.len(), d.flags) });
    }
    let Some(r) = d.records.iter().find(|r| r.isa == ISA_PPC) else {
        return Err(MixedModeError::NoRunnableRecord { at: d.at, why: String::from("68k code only: running 68k code from PowerPC is NEXT-NEEDED (cross-ISA calls)") });
    };
    if r.flags & (PROC_DESCRIPTOR_IS_RELATIVE | FRAGMENT_NEEDS_PREPARING) != 0 {
        return Err(MixedModeError::NoRunnableRecord { at: d.at, why: format!("a relative or unprepared PowerPC record (routineFlags {:#x}) is not resolved yet", r.flags) });
    }
    if r.proc_descriptor == 0 { return Err(MixedModeError::NoRunnableRecord { at: d.at, why: String::from("a PowerPC record with a NIL procedure") }); }
    Ok(*r)
}

fn write_record(mem: &mut GuestMem, at: u32, r: &Record) -> Result<(), MixedModeError> {
    let e = |_| MixedModeError::Unreadable(at);
    mem.write_u32(at, r.proc_info).map_err(e)?;
    mem.write_u8(at + 4, 0).map_err(e)?;
    mem.write_u8(at + 5, r.isa).map_err(e)?;
    mem.write_u16(at + 6, r.flags).map_err(e)?;
    mem.write_u32(at + 8, r.proc_descriptor).map_err(e)?;
    mem.write_u32(at + 12, 0).map_err(e)?;
    mem.write_u32(at + 16, r.selector).map_err(e)
}

/// Allocate and fill a descriptor with `records` (1 or 2); the block is recorded as the constructors' to free.
fn build<U: GuestUi>(m: &mut Mac<U>, records: &[Record]) -> Result<u32, MixedModeError> {
    let len = HEADER_LEN + RECORD_LEN * records.len() as u32;
    let at = m.mm.new_ptr(&mut m.mem, &mut m.heap, len, true).map_err(|_| MixedModeError::NoMemory)?;
    let e = |_| MixedModeError::Unreadable(at);
    m.mem.write_u16(at, GO_MIXED_MODE_TRAP).map_err(e)?;
    m.mem.write_u8(at + 2, ROUTINE_DESCRIPTOR_VERSION).map_err(e)?;
    m.mem.write_u16(at + 10, records.len() as u16 - 1).map_err(e)?;
    for (i, r) in records.iter().enumerate() { write_record(&mut m.mem, at + HEADER_LEN + RECORD_LEN * i as u32, r)?; }
    m.routine_descriptors.insert(at);
    Ok(at)
}

/// `NewRoutineDescriptor(theProc, theProcInfo, theISA)`: one record, absolute, prepared, native ISA.
pub fn new_routine_descriptor<U: GuestUi>(m: &mut Mac<U>, proc: u32, proc_info: u32, isa: u8) -> Result<u32, MixedModeError> {
    build(m, &[Record { proc_info, isa, flags: USE_NATIVE_ISA, proc_descriptor: proc, selector: 0 }])
}

/// `NewFatRoutineDescriptor(theM68kProc, thePowerPCProc, theProcInfo)`: two records, 68k first (routineCount = 1).
pub fn new_fat_routine_descriptor<U: GuestUi>(m: &mut Mac<U>, m68k: u32, ppc: u32, proc_info: u32) -> Result<u32, MixedModeError> {
    build(m, &[
        Record { proc_info, isa: ISA_M68K, flags: USE_NATIVE_ISA, proc_descriptor: m68k, selector: 0 },
        Record { proc_info, isa: ISA_PPC, flags: USE_NATIVE_ISA, proc_descriptor: ppc, selector: 0 },
    ])
}

/// `DisposeRoutineDescriptor(theUPP)`: frees a block the constructors made, once. NIL is a no-op (as DisposePtr(NIL)).
pub fn dispose_routine_descriptor<U: GuestUi>(m: &mut Mac<U>, upp: u32) -> Result<(), MixedModeError> {
    if upp == 0 { return Ok(()); }
    if !m.routine_descriptors.remove(&upp) { return Err(MixedModeError::NotOwned(upp)); }
    m.mm.dispose_ptr(&mut m.heap, upp).map_err(|_| MixedModeError::NotOwned(upp))
}
