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

pub use cpu::Cpu;

mod table;
pub use table::vtable;

// The types the CPU contract publishes for ARM (heirloom-cpu-abi::arm): ONE definition, shared with the engine side
// (heirloom-cpu-arm-client) that drives cpu-arm.library without linking this core. They are this crate's API unchanged.
pub use heirloom_cpu_abi::arm::{Access, Config, Exit, Fault, Memory, Trap, TrapKind, Unaligned, Vfp};
