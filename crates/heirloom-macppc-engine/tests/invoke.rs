//! `Invoke<Name>UPP`: the program's UPP is run through the callback machine and its result comes back to the caller.
use heirloom_ui_abi::RecordingGuestUi;
use heirloom_macppc_engine::{MacPpc, PpcHost};
use heirloom_mac68k_engine::mixed_mode::{new_routine_descriptor, ISA_M68K, ISA_PPC};
use heirloom_cpu_ppc::{Cpu, Host};

type M = MacPpc<RecordingGuestUi>;
const TV: u32 = 0x0003_0000;
const CODE: u32 = 0x0003_1000;
const TOC: u32 = 0x0003_8000;
const SP: u32 = 0x0004_0000;
const CALLER_LR: u32 = 0x0005_5554;

fn setup(name: &str) -> (M, Cpu, u32) {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    m.mac.mem.write_u32(TV, CODE).unwrap();
    m.mac.mem.write_u32(TV + 4, TOC).unwrap();
    m.bind(name);
    let id = m.glue.intern(&mut m.mac, "CFM", name).unwrap();
    let stub = m.glue.entry(id).unwrap().stub;
    let mut cpu = Cpu::new(0x5000, SP);
    cpu.gpr[2] = 0x6000; cpu.lr = CALLER_LR; cpu.gpr[14] = 0xABCD;
    (m, cpu, stub)
}

#[test]
fn invokemodalfilterupp_runs_the_programs_ppc_filter_and_returns_its_boolean_to_the_caller() {
    let (mut m, mut cpu, stub) = setup("InvokeModalFilterUPP");
    let upp = new_routine_descriptor(&mut m.mac, TV, 0x2C0, ISA_PPC).unwrap();
    cpu.gpr[3..7].copy_from_slice(&[0x1111, 0x2222, 0x3333, upp]);
    assert!(m.native(&mut cpu, stub));
    assert_eq!((cpu.pc, cpu.gpr[3], cpu.gpr[4], cpu.gpr[5], cpu.gpr[2]), (CODE, 0x1111, 0x2222, 0x3333, TOC), "the filter gets (dialog, event, itemHit)");
    assert_eq!(m.callback_depth(), 1);
    // the filter returns TRUE with junk in the upper bits and trashes a callee-saved register: the caller sees the Boolean and its own registers
    cpu.gpr[3] = 0x0000_0101; cpu.gpr[14] = 0xDEAD;
    let ret = cpu.lr;
    assert!(m.native(&mut cpu, ret));
    assert_eq!((cpu.pc, cpu.gpr[3], cpu.gpr[2], cpu.gpr[14]), (CALLER_LR, 1, 0x6000, 0xABCD));
    assert_eq!(m.callback_depth(), 0);
}

#[test]
fn a_plain_tvector_is_a_carbon_upp_and_the_refusals_are_by_name() {
    let (mut m, mut cpu, stub) = setup("InvokeModalFilterUPP");
    cpu.gpr[3..7].copy_from_slice(&[1, 2, 3, TV]);
    assert!(m.native(&mut cpu, stub));
    assert_eq!((cpu.pc, cpu.gpr[3]), (CODE, 1), "CarbonLib allows a plain function pointer");
    // NEGATIVE controls: a NULL UPP, and a descriptor for 68k code only, stop the program by name
    let (mut m, mut cpu, stub) = setup("InvokeModalFilterUPP");
    cpu.gpr[3..7].copy_from_slice(&[1, 2, 3, 0]);
    m.native(&mut cpu, stub);
    assert!(m.fault().is_some_and(|f| f.contains("InvokeModalFilterUPP") && f.contains("NULL")), "{:?}", m.fault());
    let (mut m, mut cpu, stub) = setup("InvokeModalFilterUPP");
    let only68k = new_routine_descriptor(&mut m.mac, 0x0001_2000, 0x2C0, ISA_M68K).unwrap();
    cpu.gpr[3..7].copy_from_slice(&[1, 2, 3, only68k]);
    m.native(&mut cpu, stub);
    assert!(m.fault().is_some_and(|f| f.contains("68k code only")), "{:?}", m.fault());
    // a void type and an OSStatus type keep r3 as the callee left it / pass it whole
    let (mut m, mut cpu, stub) = setup("InvokeEventHandlerUPP");
    cpu.gpr[3..7].copy_from_slice(&[7, 8, 9, TV]);
    m.native(&mut cpu, stub);
    cpu.gpr[3] = 0xFFFF_D8EE; // eventNotHandledErr-like
    let ret = cpu.lr;
    m.native(&mut cpu, ret);
    assert_eq!(cpu.gpr[3], 0xFFFF_D8EE);
}
