//! Musashi 68k core, wrapped.
//!
//! # ONE GUEST PER PROCESS
//! Musashi keeps the whole CPU in C globals. There is exactly one CPU per process:
//! [`run`] takes a process-wide lock, so two guests in one process are serialised (never
//! concurrent), and their registers are NOT preserved between runs (each `run` resets the
//! CPU). This matches the CoralOS plan of one worker process per guest application; a
//! multi-guest-per-process design would need a patched core. Do not work around it here.
//!
//! The core calls back into Rust for memory (`m68k_read_memory_*`) and for the ILLEGAL
//! opcode ($4AFC), which engines use as the library-call trap. Both go through the
//! [`Host`] passed to [`run`].
#![cfg_attr(not(test), no_std)]
extern crate alloc;
use alloc::vec;
use alloc::vec::Vec;
use retro_guest_mem::GuestMem;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

// no_std: a spin lock replaces std::sync::Mutex (Musashi is one process-wide CPU anyway).
static CPU_LOCK: AtomicBool = AtomicBool::new(false);
struct Guard;
impl Guard {
    fn take() -> Guard {
        while CPU_LOCK.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() { core::hint::spin_loop(); }
        Guard
    }
}
impl Drop for Guard { fn drop(&mut self) { CPU_LOCK.store(false, Ordering::Release); } }

extern "C" {
    fn m68k_init();
    fn m68k_set_cpu_type(t: u32);
    fn m68k_pulse_reset();
    fn m68k_execute(cycles: i32) -> i32;
    fn m68k_end_timeslice();
    fn m68k_get_reg(ctx: *mut u8, r: u32) -> u32;
    fn m68k_set_reg(r: u32, v: u32);
    fn m68k_context_size() -> u32;
    fn m68k_get_context(dst: *mut u8) -> u32;
    fn m68k_set_context(src: *mut u8);
}
pub const REG_D0: u32 = 0;
pub const REG_A0: u32 = 8;
const REG_PC: u32 = 16;
const CPU_68020: u32 = 4;

/// The guest side the CPU runs against.
pub trait Host {
    fn mem(&mut self) -> &mut GuestMem;
    /// Called on every ILLEGAL ($4AFC). `pc` is the address of that opcode. Return true if
    /// handled (execution continues at the next word); false lets Musashi raise the
    /// illegal-instruction exception.
    fn illegal(&mut self, cpu: &mut Cpu, pc: u32) -> bool;
}

/// Register access and run control, valid only inside a callback.
pub struct Cpu { halted: bool, blocked: bool, insns: u64, bus_faults: u32 }

impl Cpu {
    pub fn dreg(&self, n: u32) -> u32 { debug_assert!(n < 8); unsafe { m68k_get_reg(core::ptr::null_mut(), REG_D0 + n) } }
    pub fn areg(&self, n: u32) -> u32 { debug_assert!(n < 8); unsafe { m68k_get_reg(core::ptr::null_mut(), REG_A0 + n) } }
    pub fn set_d0(&mut self, v: u32) { unsafe { m68k_set_reg(REG_D0, v) } }
    /// Set address register `n` (A7 = the active stack pointer). Only inside a Host callback, like every accessor here.
    pub fn set_areg(&mut self, n: u32, v: u32) { debug_assert!(n < 8); unsafe { m68k_set_reg(REG_A0 + n, v) } }
    pub fn pc(&self) -> u32 { unsafe { m68k_get_reg(core::ptr::null_mut(), REG_PC) } }
    /// Stop after the current instruction.
    pub fn halt(&mut self) { self.halted = true; unsafe { m68k_end_timeslice() } }
    /// Stop after the current instruction WITHOUT ending the program: the trapped call is left unfinished, and the next
    /// [`resume`] continues at the instruction after the trap (the library slot's RTS), so the call returns then with
    /// whatever D0 the host set before resuming. This is how a guest `Wait` with nothing pending suspends.
    pub fn block(&mut self) { self.blocked = true; unsafe { m68k_end_timeslice() } }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunStats {
    pub insns: u64,
    /// Guest accesses outside RAM (read as 0 / dropped). A bridge that validates its pointers keeps this 0.
    pub bus_faults: u32,
    /// True if stopped by [`Cpu::halt`], false if the instruction cap was hit.
    pub halted: bool,
    /// True if stopped by [`Cpu::block`]: the guest is suspended mid-call and the next [`resume`] continues it.
    pub blocked: bool,
}

type Active = Option<(*mut (dyn Host + 'static), *mut Cpu)>;
struct ActiveCell(UnsafeCell<Active>);
// SAFETY: only touched while CPU_LOCK is held (run) or from Musashi callbacks that run inside run().
unsafe impl Sync for ActiveCell {}
static ACTIVE: ActiveCell = ActiveCell(UnsafeCell::new(None));

fn with<R>(f: impl FnOnce(&mut dyn Host, &mut Cpu) -> R) -> R {
    let (h, c) = unsafe { *ACTIVE.0.get() }.expect("Musashi callback outside retro_m68k::run");
    // SAFETY: set by `run` for exactly the duration of m68k_execute on this thread.
    unsafe { f(&mut *h, &mut *c) }
}

/// One guest CPU's saved registers (Musashi's context). Musashi has ONE live CPU per process, so each guest machine
/// keeps its own context and [`resume`] swaps it in under CPU_LOCK: two machines (or two parallel tests) can never
/// run on, or reset, each other's registers between calls.
pub struct CpuContext(Vec<u8>);

/// Reset a CPU (a 68020) to start at `entry` with stack `sp`, executing nothing, and return its context. Address 4
/// doubles as the reset PC vector and AbsExecBase, so it is saved and restored around the reset.
pub fn start<H: Host>(host: &mut H, entry: u32, sp: u32) -> CpuContext { start_with(host, entry, sp, &[]) }

/// [`start`], then set the given registers (`(REG_*, value)`, D0..D7 = 0..7, A0..A7 = 8..15) in the new context:
/// an AmigaDOS CLI program is entered with A0 = its argument string and D0 = that string's length.
pub fn start_with<H: Host>(host: &mut H, entry: u32, sp: u32, regs: &[(u32, u32)]) -> CpuContext {
    let _g = Guard::take();
    let saved4 = host.mem().read_u32(4).unwrap_or(0);
    let _ = host.mem().write_u32(0, sp);
    let _ = host.mem().write_u32(4, entry);
    let mut cpu = Cpu { halted: false, blocked: false, insns: 0, bus_faults: 0 };
    // The reset reads the vectors through the memory callbacks, so ACTIVE must be set for it too.
    activate(host, &mut cpu, |_| unsafe {
        m68k_init();
        m68k_set_cpu_type(CPU_68020);
        m68k_pulse_reset();
    });
    let _ = host.mem().write_u32(4, saved4);
    for &(r, v) in regs { unsafe { m68k_set_reg(r, v) } }
    let mut ctx = CpuContext(vec![0u8; unsafe { m68k_context_size() } as usize]);
    unsafe { m68k_get_context(ctx.0.as_mut_ptr()) };
    ctx
}

/// Execute the machine whose registers are `ctx` until [`Cpu::halt`], [`Cpu::block`] or `max_insns` (counted by the
/// per-instruction hook). No reset: a guest stopped by `block` continues where it stopped.
pub fn resume<H: Host>(host: &mut H, ctx: &mut CpuContext, max_insns: u64) -> RunStats {
    let _g = Guard::take();
    unsafe { m68k_set_context(ctx.0.as_mut_ptr()) };
    let mut cpu = Cpu { halted: false, blocked: false, insns: 0, bus_faults: 0 };
    activate(host, &mut cpu, |c| {
        while !c.halted && !c.blocked && c.insns < max_insns {
            unsafe { m68k_execute(1000) };
        }
    });
    unsafe { m68k_get_context(ctx.0.as_mut_ptr()) };
    RunStats { insns: cpu.insns, bus_faults: cpu.bus_faults, halted: cpu.halted, blocked: cpu.blocked }
}

/// Run from `entry` with initial stack `sp` until [`Cpu::halt`], [`Cpu::block`] or `max_insns`: [`start`] + [`resume`].
pub fn run<H: Host>(host: &mut H, entry: u32, sp: u32, max_insns: u64) -> RunStats {
    let mut ctx = start(host, entry, sp);
    resume(host, &mut ctx, max_insns)
}

/// Set ACTIVE to (`host`, `cpu`) for the duration of `f` (the only window in which Musashi may call back), then clear
/// it. Called with CPU_LOCK held.
fn activate<H: Host>(host: &mut H, cpu: &mut Cpu, f: impl FnOnce(&mut Cpu)) {
    let hp: *mut (dyn Host + '_) = host;
    // SAFETY: lifetime erased only while ACTIVE is set; cleared before returning.
    let hp: *mut (dyn Host + 'static) = unsafe { core::mem::transmute(hp) };
    let cp: *mut Cpu = cpu;
    unsafe { *ACTIVE.0.get() = Some((hp, cp)); }
    // SAFETY: Musashi reaches `cpu` only through ACTIVE inside `f`, never concurrently with `f`'s own use of it.
    f(unsafe { &mut *cp });
    unsafe { *ACTIVE.0.get() = None; }
}

#[no_mangle] pub extern "C" fn retro_instr_hook(_pc: u32) { with(|_, c| c.insns += 1) }
#[no_mangle] pub extern "C" fn retro_illg_hook(opcode: i32) -> i32 {
    // Only $4AFC is a bridge trap; the library slot's next word is an RTS.
    if opcode != 0x4AFC { return 0; }
    with(|h, c| {
        let pc = c.pc().wrapping_sub(2); // PC has advanced past the opcode
        h.illegal(c, pc) as i32
    })
}
fn rdm(addr: u32, len: u32) -> u32 {
    with(|h, c| {
        let m = h.mem();
        let r = match len { 1 => m.read_u8(addr).map(u32::from), 2 => m.read_u16(addr).map(u32::from), _ => m.read_u32(addr) };
        r.unwrap_or_else(|_| { c.bus_faults += 1; 0 })
    })
}
fn wrm(addr: u32, len: u32, v: u32) {
    with(|h, c| {
        let m = h.mem();
        let r = match len { 1 => m.write_u8(addr, v as u8), 2 => m.write_u16(addr, v as u16), _ => m.write_u32(addr, v) };
        if r.is_err() { c.bus_faults += 1 }
    })
}
#[no_mangle] pub extern "C" fn m68k_read_memory_8(a: u32) -> u32 { rdm(a, 1) }
#[no_mangle] pub extern "C" fn m68k_read_memory_16(a: u32) -> u32 { rdm(a, 2) }
#[no_mangle] pub extern "C" fn m68k_read_memory_32(a: u32) -> u32 { rdm(a, 4) }
#[no_mangle] pub extern "C" fn m68k_write_memory_8(a: u32, v: u32) { wrm(a, 1, v) }
#[no_mangle] pub extern "C" fn m68k_write_memory_16(a: u32, v: u32) { wrm(a, 2, v) }
#[no_mangle] pub extern "C" fn m68k_write_memory_32(a: u32, v: u32) { wrm(a, 4, v) }
#[no_mangle] pub extern "C" fn m68k_read_immediate_16(a: u32) -> u32 { rdm(a, 2) }

#[cfg(test)]
mod tests {
    use super::*;

    struct H { mem: GuestMem, traps: Vec<u32> }
    impl Host for H {
        fn mem(&mut self) -> &mut GuestMem { &mut self.mem }
        fn illegal(&mut self, cpu: &mut Cpu, pc: u32) -> bool {
            self.traps.push(pc);
            if pc == 0x200 { cpu.halt(); } else { cpu.set_d0(0x77); }
            true
        }
    }

    /// moveq #5,d1 ; ILLEGAL (trap, returns d0=0x77) ; ILLEGAL at $200 halts.
    #[test]
    fn executes_and_traps_illegal() {
        let mut h = H { mem: GuestMem::new(1 << 20), traps: vec![] };
        h.mem.write_bytes(0x1000, &[0x72, 0x05, 0x4A, 0xFC, 0x4E, 0xF9, 0, 0, 0x02, 0x00]).unwrap(); // moveq; illegal; jmp $200
        h.mem.write_u16(0x200, 0x4AFC).unwrap();
        let st = run(&mut h, 0x1000, 0x8000, 1000);
        assert_eq!(h.traps, vec![0x1002, 0x200]);
        assert!(st.halted);
        assert_eq!(st.bus_faults, 0);
    }

    #[test]
    fn wild_access_counts_a_bus_fault_not_a_crash() {
        let mut h = H { mem: GuestMem::new(1 << 20), traps: vec![] };
        // move.l $F00000.l,d0 (outside 1 MiB) ; illegal at $200 via jmp
        h.mem.write_bytes(0x1000, &[0x20, 0x39, 0x00, 0xF0, 0x00, 0x00, 0x4E, 0xF9, 0, 0, 0x02, 0x00]).unwrap();
        h.mem.write_u16(0x200, 0x4AFC).unwrap();
        let st = run(&mut h, 0x1000, 0x8000, 1000);
        assert_eq!(st.bus_faults, 1);
    }
}
