//! The heirloom-cpu-68k test suite, run through cpu-68k.library's table: every test of the static crate's `mod tests`
//! ported with its program bytes and assertions unchanged (the API is the same only if these pass through the table), plus
//! the client's own: independent contexts, a saved context's register, binding refusals and the recent-PC ring.
extern crate std;
use std::sync::{Mutex, MutexGuard};
use std::{vec, vec::Vec};

use crate::*;
use heirloom_cpu_68k::vtable;
use heirloom_mem_flat32::GuestMem;

static TEST_LOCK: Mutex<()> = Mutex::new(());

/// The client binds one process-wide table and runs one process-wide instance: one test at a time.
fn lock() -> MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn bind_static() {
    assert!(matches!(bind(vtable()), Ok(()) | Err(BindRefusal::AlreadyBound)) || is_bound());
}

// ── Ported verbatim from heirloom-cpu-68k's tests ─────────────────────────────


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
    let _g = lock();
    bind_static();
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
    let _g = lock();
    bind_static();
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
    let _g = lock();
    bind_static();
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
    let _g = lock();
    bind_static();
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
    let _g = lock();
    bind_static();
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
    let _g = lock();
    bind_static();
    let mut h = aline_host(false);
    let st = run(&mut h, 0x1000, 0x8000, 1000);
    assert_eq!(h.seen, vec![(0xA913, 0x1000)]);
    assert!(st.halted, "vector 10 -> the ILLEGAL at 0x300 halted");
    assert_eq!(st.insns, 2, "moveq never ran");
}

// ── The client's own ─────────────────────────────────────────────────────────

/// A host whose every ILLEGAL records D0 and BLOCKS: the Amiga scheduler's shape (a guest call suspends, another runs).
struct Rec { mem: GuestMem, d0: Vec<u32> }
impl Host for Rec {
    fn mem(&mut self) -> &mut GuestMem { &mut self.mem }
    fn illegal(&mut self, cpu: &mut Cpu, _pc: u32) -> bool {
        self.d0.push(cpu.dreg(0));
        cpu.block();
        true
    }
}

#[test]
fn two_contexts_resume_independently() {
    let _g = lock();
    bind_static();
    let mut h = Rec { mem: GuestMem::new(1 << 20), d0: vec![] };
    // A: moveq #1,d0 ; ILLEGAL ; moveq #3,d0 ; ILLEGAL     B: moveq #2,d0 ; ILLEGAL
    h.mem.write_bytes(0x1000, &[0x70, 0x01, 0x4A, 0xFC, 0x70, 0x03, 0x4A, 0xFC]).unwrap();
    h.mem.write_bytes(0x2000, &[0x70, 0x02, 0x4A, 0xFC]).unwrap();
    let mut a = start(&mut h, 0x1000, 0x8000);
    let mut b = start(&mut h, 0x2000, 0x7000);
    assert!(resume(&mut h, &mut a, 1000).blocked);
    assert!(resume(&mut h, &mut b, 1000).blocked);
    assert!(resume(&mut h, &mut a, 1000).blocked);
    assert_eq!(h.d0, vec![1, 2, 3], "A continued after its first trap and never saw B's D0");
}

#[test]
fn set_reg_on_a_saved_context_is_seen_by_the_next_resume() {
    let _g = lock();
    bind_static();
    let mut h = Rec { mem: GuestMem::new(1 << 20), d0: vec![] };
    h.mem.write_u16(0x1000, 0x4AFC).unwrap();
    let mut ctx = start(&mut h, 0x1000, 0x8000);
    ctx.set_reg(REG_D0, 0x1234);
    assert!(resume(&mut h, &mut ctx, 1000).blocked);
    assert_eq!(h.d0, vec![0x1234]);
}

#[test]
fn recent_pcs_reports_the_last_instructions() {
    let _g = lock();
    bind_static();
    let mut h = H { mem: GuestMem::new(1 << 20), traps: vec![] };
    h.mem.write_bytes(0x1000, &[0x72, 0x05, 0x4A, 0xFC, 0x4E, 0xF9, 0, 0, 0x02, 0x00]).unwrap();
    h.mem.write_u16(0x200, 0x4AFC).unwrap();
    assert!(run(&mut h, 0x1000, 0x8000, 1000).halted);
    assert_eq!(recent_pcs().last(), Some(&0x200), "the final ILLEGAL is the newest entry");
}

#[test]
fn bind_refuses_a_wrong_table() {
    let _g = lock();
    let real = vtable();
    // Wrong arch
    let bad_arch = CpuVTable { arch_id: 2, ..*real };
    let leaked_arch = Box::leak(Box::new(bad_arch));
    match bind(leaked_arch) {
        Err(BindRefusal::Open(OpenRefusal::WrongArch { .. })) => {}
        other => panic!("expected WrongArch, got {:?}", other),
    }
    // Wrong version
    let bad_ver = CpuVTable { version: 1, ..*real };
    let leaked_ver = Box::leak(Box::new(bad_ver));
    match bind(leaked_ver) {
        Err(BindRefusal::Open(OpenRefusal::VersionTooOld { .. })) => {}
        other => panic!("expected VersionTooOld, got {:?}", other),
    }
}
