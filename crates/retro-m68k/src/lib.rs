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
//! [`Host`] passed to [`run`]. A-line words ($Axxx, the classic Mac traps) go to [`Host::aline`].
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
pub const REG_PC: u32 = 16;
const REG_SR: u32 = 17;

/// Which 68k the core emulates. The model decides the instruction set AND the exception stack frame: a 68000 / 68010
/// pushes a 6- / 8-byte frame (68010: with a format word), a 68020 pushes the 8-byte frame with a format / vector word, so a
/// handler that adjusts the stack by the frame size (`addq.w #2,sp ; rts` after a TRAP) only works on the model it was
/// written for. The values are Musashi's `M68K_CPU_TYPE_*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CpuModel {
    /// Original 68000 (Mac 128K / 512K / Plus / SE, Amiga 1000..2000): 6-byte exception frame, no 68020 instructions.
    M68000 = 1,
    /// 68010: 8-byte frame with the format/vector word, VBR.
    M68010 = 2,
    /// 68020 (the default of every engine here).
    #[default]
    M68020 = 4,
}

/// The guest side the CPU runs against.
pub trait Host {
    fn mem(&mut self) -> &mut GuestMem;
    /// Called on every ILLEGAL ($4AFC). `pc` is the address of that opcode. Return true if
    /// handled (execution continues at the next word); false lets Musashi raise the
    /// illegal-instruction exception.
    fn illegal(&mut self, cpu: &mut Cpu, pc: u32) -> bool;
    /// Called on every A-line opcode ($Axxx: the classic Mac Toolbox/OS trap words). `pc` is the address of the
    /// opcode word. Return true if handled: execution continues at the next word, with the stack exactly as the
    /// handler left it (no exception frame is built, so the trap's stack arguments are still on top). False raises
    /// the 68k line-1010 exception as before. The default declines, so hosts that never see A-line words (Amiga) need
    /// nothing.
    fn aline(&mut self, _cpu: &mut Cpu, _opcode: u16, _pc: u32) -> bool { false }
}

/// Register access and run control, valid only inside a callback.
pub struct Cpu { halted: bool, blocked: bool, insns: u64, bus_faults: u32, detached: bool, touched: core::cell::Cell<bool> }

impl Cpu {
    /// A `Cpu` that belongs to no running 68k: for a host that drives a Toolbox handler on behalf of ANOTHER processor
    /// (the PowerPC personality calling the same Rust handlers). It has no registers: reads give 0, writes are dropped (both recorded, see [`Cpu::touched_registers`]),
    /// and [`Cpu::halt`] / [`Cpu::block`] only record the request ([`Cpu::is_halted`], [`Cpu::is_blocked`]) without
    /// touching Musashi's global timeslice, so it can never disturb a 68k machine running in the same process.
    pub fn detached() -> Cpu { Cpu { halted: false, blocked: false, insns: 0, bus_faults: 0, detached: true, touched: core::cell::Cell::new(false) } }
    /// True if a register was read or written on a detached `Cpu`: the value was a placeholder (0 / dropped), so the
    /// caller must refuse the call by name rather than trust what the handler computed from it.
    pub fn touched_registers(&self) -> bool { self.touched.get() }
    /// True if [`Cpu::halt`] was called on this `Cpu` (meaningful on a detached one; a running 68k reports through RunStats).
    pub fn is_halted(&self) -> bool { self.halted }
    /// True if [`Cpu::block`] was called on this `Cpu`.
    pub fn is_blocked(&self) -> bool { self.blocked }
    pub fn dreg(&self, n: u32) -> u32 { debug_assert!(n < 8); if self.detached { self.touched.set(true); return 0; } unsafe { m68k_get_reg(core::ptr::null_mut(), REG_D0 + n) } }
    pub fn areg(&self, n: u32) -> u32 { debug_assert!(n < 8); if self.detached { self.touched.set(true); return 0; } unsafe { m68k_get_reg(core::ptr::null_mut(), REG_A0 + n) } }
    pub fn set_d0(&mut self, v: u32) { if self.detached { self.touched.set(true); return; } unsafe { m68k_set_reg(REG_D0, v) } }
    pub fn set_dreg(&mut self, n: u32, v: u32) { debug_assert!(n < 8); if self.detached { self.touched.set(true); return; } unsafe { m68k_set_reg(REG_D0 + n, v) } }
    /// Set address register `n` (A7 = the active stack pointer). Only inside a Host callback, like every accessor here.
    pub fn set_areg(&mut self, n: u32, v: u32) { debug_assert!(n < 8); if self.detached { self.touched.set(true); return; } unsafe { m68k_set_reg(REG_A0 + n, v) } }
    pub fn pc(&self) -> u32 { if self.detached { self.touched.set(true); return 0; } unsafe { m68k_get_reg(core::ptr::null_mut(), REG_PC) } }
    /// Continue at `pc` instead of the instruction after the trapped word: how an A-line handler redirects the guest
    /// (the Segment Loader's `_LoadSeg` resumes at the jump-table entry it just patched). Only inside a Host callback.
    pub fn set_pc(&mut self, pc: u32) { if self.detached { self.touched.set(true); return; } unsafe { m68k_set_reg(REG_PC, pc) } }
    /// The status register (the condition codes are its low five bits).
    pub fn sr(&self) -> u32 { if self.detached { self.touched.set(true); return 0; } unsafe { m68k_get_reg(core::ptr::null_mut(), REG_SR) } }
    pub fn set_sr(&mut self, v: u32) { if self.detached { self.touched.set(true); return; } unsafe { m68k_set_reg(REG_SR, v) } }
    /// Stop after the current instruction.
    pub fn halt(&mut self) { self.halted = true; if !self.detached { unsafe { m68k_end_timeslice() } } }
    /// Forget a [`Cpu::halt`] / [`Cpu::block`] requested during this callback: the host is abandoning the trap it was handling
    /// (to run it again later), so what the abandoned run asked for must not stop the guest.
    pub fn cancel_stop(&mut self) { self.halted = false; self.blocked = false; }
    /// Stop after the current instruction WITHOUT ending the program: the trapped call is left unfinished, and the next
    /// [`resume`] continues at the instruction after the trap (the library slot's RTS), so the call returns then with
    /// whatever D0 the host set before resuming. This is how a guest `Wait` with nothing pending suspends.
    pub fn block(&mut self) { self.blocked = true; if !self.detached { unsafe { m68k_end_timeslice() } } }
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
    /// Some(pc) if the CPU stopped making progress: an execute slice retired no instruction (the 68k is in STOP, or
    /// halted itself after a double fault). Without this the run loop would spin forever on a CPU that never runs.
    pub stalled_at: Option<u32>,
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

impl CpuContext {
    /// Set register `r` (REG_D0.., REG_A0..) in this saved context: how a host completes a blocked call (the D0 a guest
    /// Wait returns) before resuming it.
    pub fn set_reg(&mut self, r: u32, v: u32) {
        let _g = Guard::take();
        unsafe {
            m68k_set_context(self.0.as_mut_ptr());
            m68k_set_reg(r, v);
            m68k_get_context(self.0.as_mut_ptr());
        }
    }
}

/// Read a Cpu field the callbacks may have written during the last m68k_execute (see `activate`).
macro_rules! live { ($p:expr, $f:ident) => { unsafe { core::ptr::read_volatile(core::ptr::addr_of!((*$p).$f)) } }; }

/// Reset a CPU (a 68020 unless [`start_model`] says otherwise) to start at `entry` with stack `sp`, executing nothing, and return its context. Address 4
/// doubles as the reset PC vector and AbsExecBase, so it is saved and restored around the reset.
pub fn start<H: Host>(host: &mut H, entry: u32, sp: u32) -> CpuContext { start_with(host, entry, sp, &[]) }

/// [`start`], then set the given registers (`(REG_*, value)`, D0..D7 = 0..7, A0..A7 = 8..15) in the new context:
/// an AmigaDOS CLI program is entered with A0 = its argument string and D0 = that string's length.
pub fn start_with<H: Host>(host: &mut H, entry: u32, sp: u32, regs: &[(u32, u32)]) -> CpuContext {
    start_model(host, entry, sp, regs, CpuModel::M68020)
}

/// [`start_with`] on the given CPU model (the model is part of the saved context, so [`resume`] needs no argument).
pub fn start_model<H: Host>(host: &mut H, entry: u32, sp: u32, regs: &[(u32, u32)], model: CpuModel) -> CpuContext {
    let _g = Guard::take();
    let saved4 = host.mem().read_u32(4).unwrap_or(0);
    let _ = host.mem().write_u32(0, sp);
    let _ = host.mem().write_u32(4, entry);
    let mut cpu = Cpu { halted: false, blocked: false, insns: 0, bus_faults: 0, detached: false, touched: core::cell::Cell::new(false) };
    // The reset reads the vectors through the memory callbacks, so ACTIVE must be set for it too.
    activate(host, &mut cpu, |_| unsafe {
        m68k_init();
        m68k_set_cpu_type(model as u32);
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
    let mut cpu = Cpu { halted: false, blocked: false, insns: 0, bus_faults: 0, detached: false, touched: core::cell::Cell::new(false) };
    let mut stalled = None;
    activate(host, &mut cpu, |p| {
        while !live!(p, halted) && !live!(p, blocked) && live!(p, insns) < max_insns {
            let before = live!(p, insns);
            unsafe { m68k_execute(1000) };
            if live!(p, insns) == before && !live!(p, halted) && !live!(p, blocked) {
                stalled = Some(unsafe { (*p).pc() });
                break;
            }
        }
    });
    unsafe { m68k_get_context(ctx.0.as_mut_ptr()) };
    RunStats { insns: cpu.insns, bus_faults: cpu.bus_faults, halted: cpu.halted, blocked: cpu.blocked, stalled_at: stalled }
}

/// Run from `entry` with initial stack `sp` until [`Cpu::halt`], [`Cpu::block`] or `max_insns`: [`start`] + [`resume`].
pub fn run<H: Host>(host: &mut H, entry: u32, sp: u32, max_insns: u64) -> RunStats {
    let mut ctx = start(host, entry, sp);
    resume(host, &mut ctx, max_insns)
}

/// Set ACTIVE to (`host`, `cpu`) for the duration of `f` (the only window in which Musashi may call back), then clear
/// it. Called with CPU_LOCK held.
fn activate<H: Host>(host: &mut H, cpu: &mut Cpu, f: impl FnOnce(*mut Cpu)) {
    let hp: *mut (dyn Host + '_) = host;
    // SAFETY: lifetime erased only while ACTIVE is set; cleared before returning.
    let hp: *mut (dyn Host + 'static) = unsafe { core::mem::transmute(hp) };
    let cp: *mut Cpu = cpu;
    unsafe { *ACTIVE.0.get() = Some((hp, cp)); }
    // `f` gets the RAW pointer, never a `&mut Cpu`: Musashi's callbacks write the Cpu through ACTIVE while `f` waits in
    // m68k_execute, and a `&mut` held across that call lets the optimiser assume the fields cannot change (2026-10-18:
    // in release builds the run loop never saw `insns` move, and a guest that ran fine in debug spun forever).
    f(cp);
    unsafe { *ACTIVE.0.get() = None; }
}

/// The addresses of the last [`RECENT`] instructions executed (a ring), for diagnostics: where a guest ran just before it
/// crashed or jumped somewhere it should not. Process-wide like Musashi itself; read it only from the thread that ran the guest.
pub const RECENT: usize = 16384;
static RING: [core::sync::atomic::AtomicU32; RECENT] = [const { core::sync::atomic::AtomicU32::new(0) }; RECENT];
static RING_AT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

/// The most recent instruction addresses, oldest first.
pub fn recent_pcs() -> alloc::vec::Vec<u32> {
    use core::sync::atomic::Ordering::Relaxed;
    let at = RING_AT.load(Relaxed);
    (0..RECENT).map(|i| RING[(at + i) % RECENT].load(Relaxed)).collect()
}

#[no_mangle] pub extern "C" fn retro_instr_hook(pc: u32) {
    use core::sync::atomic::Ordering::Relaxed;
    let at = RING_AT.load(Relaxed);
    RING[at % RECENT].store(pc, Relaxed);
    RING_AT.store((at + 1) % RECENT, Relaxed);
    with(|_, c| c.insns += 1)
}
#[no_mangle] pub extern "C" fn retro_illg_hook(opcode: i32) -> i32 {
    // Only $4AFC is a bridge trap; the library slot's next word is an RTS.
    if opcode != 0x4AFC { return 0; }
    with(|h, c| {
        let pc = c.pc().wrapping_sub(2); // PC has advanced past the opcode
        h.illegal(c, pc) as i32
    })
}
/// Musashi's line-1010 exception path (vendor/m68kcpu.h `m68ki_exception_1010`) asks the host first.
#[no_mangle] pub extern "C" fn retro_aline_hook(opcode: i32) -> i32 {
    with(|h, c| {
        let pc = c.pc().wrapping_sub(2); // PC has advanced past the opcode
        h.aline(c, opcode as u16, pc) as i32
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

    /// Records A-line words; `handle` decides. dc.w $A913 ; moveq #7,d1 ; dc.w $A9F4 (halts) -- control: a host that
    /// declines gets the line-1010 exception (vector 10 -> address 0x300, which halts through ILLEGAL).
    struct A { mem: GuestMem, seen: Vec<(u16, u32)>, handle: bool }
    impl Host for A {
        fn mem(&mut self) -> &mut GuestMem { &mut self.mem }
        fn illegal(&mut self, cpu: &mut Cpu, _pc: u32) -> bool { cpu.halt(); true }
        fn aline(&mut self, cpu: &mut Cpu, opcode: u16, pc: u32) -> bool {
            self.seen.push((opcode, pc));
            if opcode == 0xA9F4 { cpu.halt(); }
            self.handle
        }
    }
    fn aline_host(handle: bool) -> A {
        let mut h = A { mem: GuestMem::new(1 << 20), seen: vec![], handle };
        h.mem.write_bytes(0x1000, &[0xA9, 0x13, 0x72, 0x07, 0xA9, 0xF4]).unwrap();
        h.mem.write_u32(10 * 4, 0x300).unwrap(); // line-1010 vector
        h.mem.write_u16(0x300, 0x4AFC).unwrap();
        h
    }

    #[test]
    fn a_stopped_cpu_is_reported_not_spun_on() {
        // STOP #$2000 (supervisor after reset): the 68k waits for an interrupt that never comes. CONTROL: the same run
        // loop with a NOP program retires instructions until the cap.
        let mut h = aline_host(true);
        h.mem.write_bytes(0x1000, &[0x4E, 0x72, 0x20, 0x00]).unwrap();
        let st = run(&mut h, 0x1000, 0x8000, 1_000_000);
        assert_eq!(st.stalled_at, Some(0x1004), "the stop is reported with the pc after STOP");
        assert!(!st.halted && st.insns < 10, "{st:?}");
        let mut n = aline_host(true);
        for i in 0..64u32 { n.mem.write_u16(0x1000 + 2 * i, 0x4E71).unwrap(); }
        n.mem.write_u16(0x1080, 0x60FE).unwrap(); // bra.s self: runs forever, retiring instructions
        let st = run(&mut n, 0x1000, 0x8000, 5000);
        assert_eq!((st.stalled_at, st.insns >= 5000), (None, true), "{st:?}");
    }

    #[test]
    fn handled_aline_continues_at_the_next_word() {
        let mut h = aline_host(true);
        let st = run(&mut h, 0x1000, 0x8000, 1000);
        assert_eq!(h.seen, vec![(0xA913, 0x1000), (0xA9F4, 0x1004)], "pc is the address of the A-line word");
        assert!(st.halted);
        assert_eq!(st.insns, 3, "moveq ran after the handled trap; no exception frame was taken");
    }

    /// A handled A-line word may redirect the guest: dc.w $A913 ; moveq #7,d1 ; dc.w $A9F4 with the first trap's handler
    /// setting pc to the final trap (skipping the moveq). CONTROL: `handled_aline_continues_at_the_next_word` runs the
    /// same bytes without redirecting and executes the moveq (3 instructions, not 2).
    #[test]
    fn a_handler_can_redirect_the_pc() {
        struct R { mem: GuestMem }
        impl Host for R {
            fn mem(&mut self) -> &mut GuestMem { &mut self.mem }
            fn illegal(&mut self, cpu: &mut Cpu, _pc: u32) -> bool { cpu.halt(); true }
            fn aline(&mut self, cpu: &mut Cpu, opcode: u16, _pc: u32) -> bool {
                match opcode { 0xA913 => cpu.set_pc(0x1004), _ => { cpu.set_d0(cpu.dreg(1)); cpu.halt() } }
                true
            }
        }
        let mut h = R { mem: GuestMem::new(1 << 20) };
        h.mem.write_bytes(0x1000, &[0xA9, 0x13, 0x72, 0x07, 0xA9, 0xF4]).unwrap();
        let st = run(&mut h, 0x1000, 0x8000, 1000);
        assert!(st.halted);
        assert_eq!(st.insns, 2, "the moveq was skipped");
    }

    #[test]
    fn declined_aline_raises_the_1010_exception() {
        let mut h = aline_host(false);
        let st = run(&mut h, 0x1000, 0x8000, 1000);
        assert_eq!(h.seen, vec![(0xA913, 0x1000)]);
        assert!(st.halted, "vector 10 -> the ILLEGAL at 0x300 halted");
        assert_eq!(st.insns, 2, "moveq never ran");
    }
}
