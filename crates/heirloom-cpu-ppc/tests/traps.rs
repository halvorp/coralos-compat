//! Named traps, the native-glue hook, sc, run control and independence of guests. The positive case sits next to
//! every refusal.
mod common;
use common::*;
use heirloom_mem_flat32::GuestMem;
use heirloom_cpu_ppc::{resume, Cpu, Host, Reg, Trap};

#[test]
fn unimplemented_opcode_groups_are_named_not_panics() {
    for (word, needle) in [
        (0x1000_0000u32, "AltiVec"),          // opcode 4 (vector / paired single)
        (0xE860_0000, "64-bit"),              // opcode 58 (ld)
        (0x0000_0000, "zero word"),
        (0x7C60_0066, "reserved opcode-31"),  // xo 51: not an instruction
        (0x7C60_00CE, "AltiVec load/store"),  // lvx, xo 103
    ] {
        let (c, _, st) = run_words(&[word], |_, _| {});
        match st.trap {
            Some(Trap::Unimplemented { pc, word: w, what }) => {
                assert_eq!((pc, w), (CODE, word));
                assert!(what.contains(needle), "{what:?} lacks {needle:?}");
            }
            t => panic!("{word:#x}: {t:?}"),
        }
        assert_eq!(c.pc, CODE, "pc is left at the faulting instruction");
    }
    // CONTROL: a defined instruction in the same slot space runs.
    let (_, _, st) = run_words(&[asm("add 3,4,5")], |_, _| {});
    assert!(st.trap.is_none());
}

#[test]
fn supervisor_instructions_are_privileged_traps() {
    for word in [0x7C60_00A6u32 /* mfmsr */, 0x7C60_0124 /* mtmsr */, 0x4C00_0064 /* rfi */] {
        let (_, _, st) = run_words(&[word], |_, _| {});
        assert!(matches!(st.trap, Some(Trap::Privileged { word: w, .. }) if w == word), "{word:#x}: {st:?}");
    }
    // CONTROL: the user-mode SPR move runs (the PVR/SDR1 refusals are in coverage.rs).
    let (_, _, st) = run_words(&[asm("mflr 3")], |_, _| {});
    assert!(st.trap.is_none());
}

#[test]
fn a_trap_leaves_registers_as_they_were_before_the_instruction() {
    let (c, _, st) = prog(&["li 3,5", "lwz 3,0(4)"], |c, _| { c.gpr[4] = 0x30_0000; });
    assert!(matches!(st.trap, Some(Trap::BusFault { .. })));
    assert_eq!(c.gpr[3], 5, "the faulting load did not clobber rT");
    assert_eq!(c.pc, CODE + 4);
    // and it can be resumed after the host repairs the cause
    let mut v = vm();
    v.mem.write_u32(CODE, asm("lwz 3,0(4)")).unwrap();
    v.mem.write_u32(CODE + 4, asm("blr")).unwrap();
    v.mem.write_u32(0x2000, 0xABCD).unwrap();
    let mut cpu = Cpu::new(CODE, STACK);
    cpu.lr = EXIT; cpu.bind_native(EXIT); cpu.gpr[4] = 0x30_0000;
    assert!(resume(&mut v, &mut cpu, 10).trap.is_some());
    cpu.gpr[4] = 0x2000;
    let st = resume(&mut v, &mut cpu, 10);
    assert!(st.halted && st.trap.is_none());
    assert_eq!(cpu.gpr[3], 0xABCD);
}

#[test]
fn trap_text_is_stable() {
    let t = Trap::Unimplemented { pc: 0x1000, word: 0x1000_0000, what: "AltiVec / paired-single (opcode 4)" };
    assert_eq!(t.to_string(), "unimplemented AltiVec / paired-single (opcode 4) (0x10000000) at 0x00001000");
    let t = Trap::BusFault { pc: 4, addr: 0xFFFF_FFFE, write: true };
    assert_eq!(t.to_string(), "bus fault: store to 0xfffffffe at 0x00000004");
    assert_eq!(Trap::NativeUnhandled { addr: 0x9000 }.to_string(), "native glue at 0x00009000 not handled by the host");
}

/// A "Toolbox" host: bound addresses implement host functions that read args from r3/r4 and return in r3.
struct Tb { mem: GuestMem, calls: Vec<u32>, redirect: Option<u32>, block_on: Option<u32> }
impl Host for Tb {
    fn mem(&mut self) -> &mut GuestMem { &mut self.mem }
    fn native(&mut self, cpu: &mut Cpu, addr: u32) -> bool {
        self.calls.push(addr);
        match addr {
            0x9000 => { cpu.gpr[3] = cpu.gpr[3].wrapping_add(cpu.gpr[4]); true }          // "Add(a, b)"
            0x9004 => { if let Some(t) = self.redirect { cpu.set_pc(t); } true }           // jumps elsewhere
            0x9008 => { if self.block_on == Some(addr) { cpu.block(); self.block_on = None; } cpu.gpr[3] = 99; true }
            0xF000 => { cpu.halt(); true }
            _ => false,                                                                     // 0x900C etc.: declined
        }
    }
}
fn tb(words: &[u32]) -> (Tb, Cpu) {
    let mut t = Tb { mem: GuestMem::new(1 << 20), calls: vec![], redirect: None, block_on: None };
    for (i, w) in words.iter().enumerate() { t.mem.write_u32(CODE + 4 * i as u32, *w).unwrap(); }
    let mut c = Cpu::new(CODE, STACK);
    c.lr = EXIT;
    for a in [0x9000, 0x9004, 0x9008, 0x900C, EXIT] { c.bind_native(a); }
    (t, c)
}
fn bl_to(from: u32, to: u32) -> u32 { 0x4800_0001 | (to.wrapping_sub(from) & 0x03FF_FFFC) }

#[test]
fn native_hook_runs_the_host_function_and_returns_through_lr() {
    // mflr 31 ; li 3,40 ; li 4,2 ; bl 0x9000 ; mtlr 31 ; blr
    let (mut t, mut c) = tb(&[asm("mflr 31"), asm("li 3,40"), asm("li 4,2"), bl_to(CODE + 12, 0x9000), asm("mtlr 31"), asm("blr")]);
    let st = resume(&mut t, &mut c, 100);
    assert!(st.halted && st.trap.is_none(), "{st:?}");
    assert_eq!(c.gpr[3], 42, "the host computed 40 + 2");
    assert_eq!(t.calls, vec![0x9000, EXIT], "native hook entered once, then the exit glue");
    assert_eq!(st.insns, 8, "six instructions plus two native calls (the Add glue and the exit glue), one tick each");
}

#[test]
fn a_native_handler_may_redirect_and_may_block() {
    let prog = [asm("mflr 31"), bl_to(CODE + 4, 0x9004), asm("li 3,1"), asm("li 5,5"), asm("mtlr 31"), asm("blr")];
    let (mut t, mut c) = tb(&prog);
    t.redirect = Some(CODE + 12); // skip "li 3,1"
    let st = resume(&mut t, &mut c, 100);
    assert!(st.halted, "{st:?}");
    assert_eq!((c.gpr[3], c.gpr[5]), (0, 5), "set_pc replaced the implicit blr: li 3,1 was skipped");
    // CONTROL: without a redirect the glue returns through LR (the next instruction) and li 3,1 runs.
    let (mut t, mut c) = tb(&prog);
    t.redirect = None;
    let st = resume(&mut t, &mut c, 100);
    assert!(st.halted);
    assert_eq!((c.gpr[3], c.gpr[5]), (1, 5));
    // block(): the run stops mid-call, resumable, and continues after the glue.
    let (mut t, mut c) = tb(&[asm("mflr 31"), bl_to(CODE + 4, 0x9008), asm("addi 3,3,1"), asm("mtlr 31"), asm("blr")]);
    t.block_on = Some(0x9008);
    let st = resume(&mut t, &mut c, 100);
    assert!(st.blocked && !st.halted && st.trap.is_none(), "{st:?}");
    assert_eq!(c.pc, CODE + 8, "the call completed (returned through LR) before the guest was suspended");
    let st = resume(&mut t, &mut c, 100);
    assert!(st.halted, "{st:?}");
    assert_eq!(c.gpr[3], 100);
}

#[test]
fn an_unhandled_native_address_is_a_named_trap_and_unbinding_works() {
    let (mut t, mut c) = tb(&[bl_to(CODE, 0x900C), asm("blr")]);
    let st = resume(&mut t, &mut c, 100);
    assert_eq!(st.trap, Some(Trap::NativeUnhandled { addr: 0x900C }));
    assert_eq!(c.pc, 0x900C);
    // After unbind the address is plain memory: executing zeros is the *other* named trap, not NativeUnhandled.
    assert!(c.unbind_native(0x900C));
    assert!(!c.is_native(0x900C));
    let st = resume(&mut t, &mut c, 100);
    assert!(matches!(st.trap, Some(Trap::Unimplemented { pc: 0x900C, .. })), "{st:?}");
}

#[test]
fn sc_is_handled_by_the_host_or_named() {
    let mut v = vm();
    v.sc_result = 0x1234;
    let words = [asm("li 0,7"), asm("sc"), asm("mr 5,3"), asm("blr")];
    for (i, w) in words.iter().enumerate() { v.mem.write_u32(CODE + 4 * i as u32, *w).unwrap(); }
    let mut c = Cpu::new(CODE, STACK);
    c.lr = EXIT; c.bind_native(EXIT);
    let st = resume(&mut v, &mut c, 100);
    assert!(st.halted && st.trap.is_none(), "{st:?}");
    assert_eq!((v.syscalls.clone(), c.gpr[5]), (vec![7], 0x1234), "host saw r0 = 7 and answered in r3");
    // CONTROL: a host without a syscall handler gets a named trap with pc at the sc.
    struct NoSc(GuestMem);
    impl Host for NoSc { fn mem(&mut self) -> &mut GuestMem { &mut self.0 } }
    let mut h = NoSc(GuestMem::new(1 << 16));
    h.0.write_u32(0x100, asm("sc")).unwrap();
    let mut c = Cpu::new(0x100, 0x8000);
    let st = resume(&mut h, &mut c, 10);
    assert_eq!(st.trap, Some(Trap::Syscall { pc: 0x100 }));
    // sc with the reserved bit clear is an invalid form, not a syscall
    h.0.write_u32(0x100, 0x4400_0000).unwrap();
    let mut c = Cpu::new(0x100, 0x8000);
    assert!(matches!(resume(&mut h, &mut c, 10).trap, Some(Trap::InvalidForm { .. })));
}

#[test]
fn the_budget_stops_a_spinning_guest_without_a_trap() {
    let mut v = vm();
    v.mem.write_u32(CODE, 0x4800_0000).unwrap(); // b .
    let mut c = Cpu::new(CODE, STACK);
    let st = resume(&mut v, &mut c, 1234);
    assert_eq!((st.insns, st.halted, st.blocked, st.trap), (1234, false, false, None));
    let st = resume(&mut v, &mut c, 10);
    assert_eq!(st.insns, 10, "resumable");
    assert_eq!(c.tb, 1244, "time base advanced per instruction");
}

#[test]
fn guests_are_independent_in_one_process() {
    // Unlike Musashi there is no global CPU: interleave two guests instruction by instruction.
    let (mut v1, mut v2) = (vm(), vm());
    for v in [&mut v1, &mut v2] {
        v.mem.write_u32(CODE, asm("addi 3,3,1")).unwrap();
        v.mem.write_u32(CODE + 4, 0x4BFF_FFFC).unwrap(); // b .-4
    }
    let (mut a, mut b) = (Cpu::new(CODE, STACK), Cpu::new(CODE, STACK));
    b.gpr[3] = 1000;
    for _ in 0..5 { resume(&mut v1, &mut a, 2); resume(&mut v2, &mut b, 4); }
    assert_eq!((a.gpr[3], b.gpr[3]), (5, 1010));
    assert_eq!(a.reg(Reg::Pc) % 4, 0);
}

#[test]
fn register_accessors_round_trip_and_mask() {
    let mut c = Cpu::new(0x2000, 0x9000);
    assert_eq!((c.reg(Reg::Pc), c.reg(Reg::Gpr(1))), (0x2000, 0x9000));
    c.set_reg(Reg::Xer, 0xFFFF_FFFF);
    assert_eq!(c.reg(Reg::Xer), 0xE000_007F);
    c.set_reg(Reg::Gpr(33), 5); // index wraps into 0..31 instead of panicking
    assert_eq!(c.gpr[1], 5);
    c.set_reg(Reg::Fpscr, 0x6000_0000);
    assert_eq!(c.reg(Reg::Fpscr), 0, "FEX/VX are derived");
}
