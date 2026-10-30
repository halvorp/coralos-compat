//! PowerPC 32-bit USER-MODE interpreter: the 601/603/G3 user instruction set that classic Mac OS applications use.
//!
//! No MMU, no supervisor state, no AltiVec, no paired singles. Big-endian guest memory is a
//! [`heirloom_mem_flat32::GuestMem`] reached through [`Host`], the same bus abstraction as heirloom-cpu-68k. The API mirrors it:
//! a resumable [`CpuContext`] ([`Cpu`]), [`resume`] with an instruction budget returning [`RunStats`], and a host
//! hook for native glue ([`Host::native`], bound with [`Cpu::bind_native`]) so an engine can put Toolbox imports at
//! guest addresses. Unlike Musashi the interpreter has no process-global state: many guests can coexist.
//!
//! Faults are named [`Trap`]s with `pc` left at the faulting instruction; nothing here panics on guest input.
//!
//! Execution tiers ([`Exec`], [`Tier`]): tier 0 ([`resume`]) fetches and decodes every instruction and is the
//! reference; tier 1 (the default of [`Exec`]) runs pre-decoded blocks cached per guest pc and invalidated by the
//! write generation of their 4 KiB code page (heirloom-mem-flat32's code watch: every write, from the guest or the host,
//! and `icbi`), leaving the same architectural state as tier 0 at every exit; [`Tier::Lockstep`] runs both on every
//! block and stops at the first difference. Measurements: docs/PPC-SPEED.md.
//!
//! Deliberate deviations (each is a named refusal or documented here, none silent):
//! - FP arithmetic is round-to-nearest-even only; with FPSCR[RN] != 0 an arithmetic FP instruction is a named
//!   `Unimplemented` trap (fctiw honours RN). XX/FI/FR and UX are not tracked; OX, ZX and the VX* flags are.
//! - FP exceptions are never delivered as interrupts; FPSCR[VE] and [ZE] suppress the result write on an invalid
//!   operation / zero divide (PEM: the target FPR is unchanged); an enabled OE or UE still writes the UNWRAPPED
//!   result (the architecture writes the result with its exponent adjusted by -192/-1536 or +192/+1536: not done),
//!   and XE only sets FEX. FPSCR[NI] (non-IEEE mode) is ignored.
//! - fres/frsqrte return the correctly rounded reciprocal, more accurate than the hardware estimate.
//! - fsqrt/fsqrts are executed although the 603/G3 omit them (the architecture defines them).
//! - Integer divide by zero / overflow: PEM leaves the result undefined; we return 0xFFFFFFFF (divwu) and
//!   0 or -1 by the dividend's sign (divw), setting OV when OE=1.
//!
//! References: PowerPC Microprocessor Family: The Programming Environments Manual for 32-bit Processors (PEM,
//! MPCFPE32B), cited per instruction family in the module headers. No emulator code was copied.
#![cfg_attr(not(test), no_std)]
extern crate alloc;

pub mod cpu;
mod exec;
pub mod fields;
mod fpscr;
mod int;
mod branch;
mod mem;
mod fpu;
mod fpmem;
mod sys;
mod x31;
mod names;
pub mod census;
mod tier;
mod t1;
mod t1cache;
mod t1op;
mod t1run;
/// cpu-ppc.library's flat table (heirloom-cpu-abi::ppc): what its coral_lib_open returns.
pub mod table;
#[cfg(test)]
mod table_tests;
pub use t1::{Divergence, LockstepStats};
pub use t1cache::CacheStats;
pub use tier::{Exec, Prof, Tier};
pub use cpu::{Cpu, CpuContext, Reg, RunStats, Trap, XER_CA, XER_OV, XER_SO};
use heirloom_mem_flat32::GuestMem;

/// The guest side the CPU runs against.
pub trait Host {
    fn mem(&mut self) -> &mut GuestMem;
    /// Execution reached an address bound with [`Cpu::bind_native`]. Return true if handled: the guest then returns
    /// through LR (as if the glue were a `blr`) unless the handler called [`Cpu::set_pc`]. False stops the run with
    /// [`Trap::NativeUnhandled`].
    fn native(&mut self, _cpu: &mut Cpu, _addr: u32) -> bool { false }
    /// An `sc` instruction. Return true if handled: execution continues after it (or at [`Cpu::set_pc`]'s target).
    /// False stops the run with [`Trap::Syscall`].
    fn syscall(&mut self, _cpu: &mut Cpu) -> bool { false }
}

/// Execute `cpu` until [`Cpu::halt`], [`Cpu::block`], a [`Trap`], or `budget` instructions. A guest stopped by
/// `block` continues where it stopped.
pub fn resume<H: Host>(host: &mut H, cpu: &mut Cpu, budget: u64) -> RunStats {
    cpu.clear_blocked();
    let mut n = 0u64;
    let mut trap = None;
    while n < budget && !cpu.halted() && !cpu.blocked() {
        if let Err(t) = exec::step(host, cpu) { trap = Some(t); break; }
        n += 1;
    }
    RunStats { insns: n, halted: cpu.halted(), blocked: cpu.blocked(), trap }
}

/// A fresh CPU at `entry` with stack `sp`, run to a stop: [`Cpu::new`] + [`resume`].
pub fn run<H: Host>(host: &mut H, entry: u32, sp: u32, budget: u64) -> (Cpu, RunStats) {
    let mut cpu = Cpu::new(entry, sp);
    let st = resume(host, &mut cpu, budget);
    (cpu, st)
}
