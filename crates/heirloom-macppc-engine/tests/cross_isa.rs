//! Cross-ISA calls (Mixed Mode, PowerPC -> 68k): `CallUniversalProc` of 68k code runs it on the machine's 68k CPU through the one callback
//! machine, marshalled by the procInfo; the 68k routine's Toolbox traps are the same Toolbox's. Which ISA comes from Mixed Mode's rule
//! (a routine descriptor's records; any other pointer is a 68k ProcPtr), never from the code bytes. Negative controls: a stack the
//! procInfo does not describe, an unimplemented trap inside the routine, a convention not built, all named.
use heirloom_common::Step;
use heirloom_mac68k_engine::mixed_mode::{new_fat_routine_descriptor, new_routine_descriptor, ISA_M68K};
use heirloom_macppc_engine::cpu::{Cpu, Host};
use heirloom_macppc_engine::{MacPpc, PpcHost};
use heirloom_ui_abi::RecordingGuestUi;

type M = MacPpc<RecordingGuestUi>;
const TV: u32 = 0x0003_0000;
const CODE: u32 = 0x0003_1000;
const TOC: u32 = 0x0003_8000;
const M68K: u32 = 0x0003_2000;
const SP: u32 = 0x0004_0000;
const CALLER_LR: u32 = 0x0005_5554;
const A5: u32 = 0x0001_2340;

fn setup(code68: &[u16]) -> (M, Cpu, u32) {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    m.mac.mem.write_u32(TV, CODE).unwrap();
    m.mac.mem.write_u32(TV + 4, TOC).unwrap();
    for (i, w) in code68.iter().enumerate() { m.mac.mem.write_u16(M68K + 2 * i as u32, *w).unwrap(); }
    m.mac.mem.write_u32(heirloom_mac68k_engine::lowmem::CURRENT_A5, A5).unwrap();
    m.bind("CallUniversalProc");
    let id = m.glue.intern(&mut m.mac, "CFM", "CallUniversalProc").unwrap();
    let stub = m.glue.entry(id).unwrap().stub;
    let mut cpu = Cpu::new(0x5000, SP);
    cpu.gpr[2] = 0x6000; cpu.lr = CALLER_LR; cpu.gpr[14] = 0xABCD; cpu.gpr[31] = 0x3131;
    (m, cpu, stub)
}

/// Call through the glue with r3.. = `args`, then step the 68k CPU until the call is over (or faults).
fn call(m: &mut M, cpu: &mut Cpu, stub: u32, args: &[u32]) -> Step {
    cpu.gpr[3..3 + args.len()].copy_from_slice(args);
    assert!(m.native(cpu, stub));
    if m.fault().is_some() { return Step::Fault(m.fault().unwrap().into()); }
    let mut last = Step::BudgetSpent;
    for _ in 0..100 {
        if !m.m68k_running() { break; }
        last = m.step_68k(cpu, 1000).0;
        if matches!(last, Step::Fault(_)) { break; }
    }
    last
}

#[test]
fn a_plain_procptr_is_68k_code_and_a_pascal_function_returns_a5_through_its_result_space() {
    // Photoshop 7.0's routine: MOVE.L A5,4(SP); RTS - a Pascal function of no parameters returning A5 (procInfo 0x30)
    let (mut m, mut cpu, stub) = setup(&[0x2F4D, 0x0004, 0x4E75]);
    let step = call(&mut m, &mut cpu, stub, &[M68K, 0x30]);
    assert!(matches!(step, Step::BudgetSpent), "{step:?} {:?}", m.fault());
    assert!(!m.m68k_running() && m.callback_depth() == 0);
    assert_eq!((cpu.pc, cpu.gpr[3]), (CALLER_LR, A5), "the caller resumes with A5 = CurrentA5 as the result");
    assert_eq!((cpu.gpr[1], cpu.gpr[2], cpu.gpr[14], cpu.gpr[31]), (SP, 0x6000, 0xABCD, 0x3131), "every PowerPC register put back");
    assert_eq!(m.mac.mem.read_u32(0).unwrap(), 0, "the 68k reset's vector words at 0 and 4 are put back (low memory is the program's)");
}

#[test]
fn pascal_parameters_go_left_to_right_and_the_callee_pops_them() {
    // FUNCTION f(a: INTEGER; b: LONGINT): LONGINT = a + b
    // MOVEA.L (SP)+,A0; MOVE.L (SP)+,D0; MOVE.W (SP)+,D1; EXT.L D1; ADD.L D1,D0; MOVE.L D0,(SP); JMP (A0)
    let code = [0x205F, 0x201F, 0x321F, 0x48C1, 0xD081, 0x2E80, 0x4ED0];
    let (mut m, mut cpu, stub) = setup(&code);
    let step = call(&mut m, &mut cpu, stub, &[M68K, 0x3B0, (-2i32) as u32, 100]);
    assert!(matches!(step, Step::BudgetSpent), "{step:?}");
    assert_eq!(cpu.gpr[3], 98);
    // NEGATIVE CONTROL: the same code under a procInfo with one 4-byte parameter pops two bytes more than were pushed: named, not ignored
    let (mut m, mut cpu, stub) = setup(&code);
    let step = call(&mut m, &mut cpu, stub, &[M68K, 0xF0, 100]);
    assert!(matches!(&step, Step::Fault(f) if f.contains("XISA-68K-STACK")), "{step:?}");
}

#[test]
fn a_68k_routine_descriptor_runs_a_c_function_whose_result_is_d0() {
    // long f(long x, short y) = x - y: MOVE.L 4(SP),D0; MOVE.W 8(SP),D1; EXT.L D1; SUB.L D1,D0; RTS
    let (mut m, mut cpu, stub) = setup(&[0x202F, 0x0004, 0x322F, 0x0008, 0x48C1, 0x9081, 0x4E75]);
    let upp = new_routine_descriptor(&mut m.mac, M68K, 0x2F1, ISA_M68K).unwrap();
    let step = call(&mut m, &mut cpu, stub, &[upp, 0x2F1, 1000, 7]);
    assert!(matches!(step, Step::BudgetSpent), "{step:?}");
    assert_eq!((cpu.pc, cpu.gpr[3]), (CALLER_LR, 993));
    // a FAT descriptor (68k + PowerPC) runs its PowerPC record: the caller's own ISA, no mode switch
    let (mut m, mut cpu, stub) = setup(&[0x4E75]);
    let fat = new_fat_routine_descriptor(&mut m.mac, M68K, TV, 0x2F1).unwrap();
    cpu.gpr[3..7].copy_from_slice(&[fat, 0x2F1, 1000, 7]);
    assert!(m.native(&mut cpu, stub));
    assert!(!m.m68k_running());
    assert_eq!((cpu.pc, cpu.gpr[3], cpu.gpr[4]), (CODE, 1000, 7));
}

#[test]
fn the_68k_routines_traps_are_served_by_the_same_toolbox() {
    // FUNCTION f: Ptr = NewPtr(16): MOVEQ #16,D0; _NewPtr; MOVE.L A0,4(SP); RTS
    let (mut m, mut cpu, stub) = setup(&[0x7010, 0xA11E, 0x2F48, 0x0004, 0x4E75]);
    let step = call(&mut m, &mut cpu, stub, &[M68K, 0x30]);
    assert!(matches!(step, Step::BudgetSpent), "{step:?}");
    let p = cpu.gpr[3];
    assert!(p != 0 && m.mac.mem.write_bytes(p, &[0; 16]).is_ok(), "a real block of guest memory: {p:#x}");
}

#[test]
fn what_cannot_run_is_refused_by_name() {
    // an unimplemented trap inside the routine stops the call by name (here `_MixedModeMagic`: PowerPC nested in 68k is not built)
    // (NOP first: a pointer whose first word is $AAFE IS a routine descriptor by Mixed Mode's rule)
    let (mut m, mut cpu, stub) = setup(&[0x4E71, 0xAAFE, 0x4E75]);
    let step = call(&mut m, &mut cpu, stub, &[M68K, 0x0]);
    assert!(matches!(&step, Step::Fault(f) if f.contains("XISA-68K-STOPPED")), "{step:?}");
    assert!(m.mac.refused.iter().any(|r| r.contains("unimplemented")), "{:?}", m.mac.refused);
    // a convention not built is refused BEFORE anything changes
    let (mut m, mut cpu, stub) = setup(&[0x4E75]);
    let before = (cpu.gpr, cpu.lr);
    cpu.gpr[3..5].copy_from_slice(&[M68K, 0x2]);
    let before = (cpu.gpr, before.1);
    m.native(&mut cpu, stub);
    assert!(m.fault().is_some_and(|f| f.contains("CallUniversalProc REFUSED") && f.contains("kRegisterBased")), "{:?}", m.fault());
    assert!(!m.m68k_running() && m.callback_depth() == 0);
    assert_eq!((cpu.gpr, cpu.lr), before, "no register changed");
}
