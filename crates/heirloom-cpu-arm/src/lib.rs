//! User-mode ARMv6 (ARM1136/ARM1176) interpreter for Heirloom's Symbian engine: A32, Thumb-1 (with the
//! BL/BLX pair) and VFPv2, plus the CP15 thread-ID registers user code reaches.
//!
//! The CPU knows nothing of an OS or of how guest memory is laid out: the host implements [`Memory`] and
//! drives [`Cpu::run`] / [`Cpu::step`]. An SVC (SWI) does not enter a kernel; it returns
//! [`Exit::Svc`] with the immediate, and the host (EKA2's exec-call dispatcher) resumes the CPU after it.
//! Anything this core does not execute (UNDEFINED, UNPREDICTABLE, privileged, a missing coprocessor) is
//! returned as a named [`TrapKind`] with the PC left on the instruction; nothing in here panics on guest input.
//!
//! Ported in part from HexRaysSA/rax (MIT, commit 526acfa) - see README.md for the file-by-file note.
#![no_std]

mod access;
mod alu;
mod a32;
mod cp15;
mod cpu;
mod loadstore;
mod media;
mod multiply;
mod softfloat;
mod thumb;
mod vfp;
mod vfp_xfer;

pub use cpu::{Config, Cpu, Unaligned};
pub use vfp::Vfp;

/// The kind of access that faulted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
    Fetch,
}

/// A memory access the host refused (unmapped, protection, ...). The host returns it from [`Memory`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fault {
    pub addr: u32,
    pub access: Access,
}

/// Guest memory, little-endian, implemented by the host.
///
/// The CPU only calls the 16-bit methods with even addresses and the 32-bit methods with 4-aligned addresses;
/// an unaligned access the architecture allows is composed from byte accesses by the CPU itself.
pub trait Memory {
    fn read_u8(&mut self, addr: u32) -> Result<u8, Fault>;
    fn read_u16(&mut self, addr: u32) -> Result<u16, Fault>;
    fn read_u32(&mut self, addr: u32) -> Result<u32, Fault>;
    fn write_u8(&mut self, addr: u32, value: u8) -> Result<(), Fault>;
    fn write_u16(&mut self, addr: u32, value: u16) -> Result<(), Fault>;
    fn write_u32(&mut self, addr: u32, value: u32) -> Result<(), Fault>;
    /// Instruction fetch (Thumb); the host may apply execute permission here.
    fn fetch_u16(&mut self, addr: u32) -> Result<u16, Fault>;
    /// Instruction fetch (A32).
    fn fetch_u32(&mut self, addr: u32) -> Result<u32, Fault>;
}

/// Why `run`/`step` returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exit {
    /// The instruction budget is used up; the CPU is between instructions.
    BudgetSpent,
    /// SVC/SWI with its immediate (24 bits in A32, 8 in Thumb). PC is already past the SVC.
    Svc(u32),
    /// BKPT with its immediate. PC is left on the BKPT.
    Bkpt(u16),
    /// WFI or WFE. PC is past the hint; the host may block the thread.
    Wait,
    /// The host's memory refused an access. PC is left on the instruction and no register was written.
    Fault { pc: u32, fault: Fault },
    /// An instruction this core does not execute. PC is left on it and no state was changed.
    Trap(Trap),
}

/// A named refusal to execute one instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trap {
    pub pc: u32,
    /// The instruction word (a Thumb halfword is zero-extended).
    pub insn: u32,
    pub thumb: bool,
    pub kind: TrapKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrapKind {
    /// UNDEFINED on ARMv6 (this includes ARMv6T2/ARMv7-only encodings, Thumb-2, and NEON).
    Undefined,
    /// UNPREDICTABLE on ARMv6: refused rather than guessed.
    Unpredictable,
    /// A privileged instruction or register access from user mode (SRS, RFE, SMC, SPSR, exception return, ...).
    Privileged,
    /// A data access whose alignment the configured [`Unaligned`] mode refuses.
    Unaligned { addr: u32, write: bool },
    /// An access to a coprocessor this core does not have.
    Coprocessor { cp: u8 },
    /// A CP15 register user mode cannot reach.
    Cp15 { read: bool, opc1: u8, crn: u8, crm: u8, opc2: u8 },
    /// A VFP system register user mode cannot reach (FPEXC, ...), or VFP is configured off.
    VfpSystemRegister { reg: u8 },
    /// A VFP operation raised a floating-point exception whose FPSCR trap-enable bit the guest set (bits are
    /// FPSCR's cumulative-flag layout: IOC=0, DZC=1, OFC=2, UFC=3, IXC=4, IDC=7). Untrapped exceptions only
    /// set the cumulative flags, as on hardware.
    VfpException { flags: u8 },
}
