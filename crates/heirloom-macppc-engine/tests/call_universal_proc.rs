//! `CallUniversalProc` from PowerPC: a 68k trap's
//! stub that the engine serves by value runs that trap's body; the program's own PowerPC routine runs through the callback machine; any
//! other target is refused by name.
use heirloom_ui_abi::RecordingGuestUi;
use heirloom_macppc_engine::{MacPpc, PpcHost};
use heirloom_mac68k_engine::mixed_mode::{new_routine_descriptor, ISA_PPC};
use heirloom_macppc_engine::cpu::{Cpu, Host};

type M = MacPpc<RecordingGuestUi>;
const TV: u32 = 0x0003_0000;
const CODE: u32 = 0x0003_1000;
const TOC: u32 = 0x0003_8000;
const SP: u32 = 0x0004_0000;
const CALLER_LR: u32 = 0x0005_5554;
const DEV: u32 = 0x0003_9000;

fn setup() -> (M, Cpu, u32) {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    m.mac.mem.write_u32(TV, CODE).unwrap();
    m.mac.mem.write_u32(TV + 4, TOC).unwrap();
    m.bind("CallUniversalProc");
    let id = m.glue.intern(&mut m.mac, "CFM", "CallUniversalProc").unwrap();
    let stub = m.glue.entry(id).unwrap().stub;
    let mut cpu = Cpu::new(0x5000, SP);
    cpu.gpr[2] = 0x6000; cpu.lr = CALLER_LR;
    (m, cpu, stub)
}

/// The stub `NGetTrapAddress(trap, ToolTrap)` hands out.
fn trap_stub(m: &mut M, trap: u16) -> u32 { heirloom_mac68k_engine::traptable::trap_address(&mut m.mac, 0xA746, u32::from(trap)).unwrap().0 }

#[test]
fn cursor_device_next_device_through_the_trap_stub_answers_an_empty_list() {
    let (mut m, mut cpu, stub) = setup();
    let at = trap_stub(&mut m, 0xAADB);
    assert_ne!(at, trap_stub(&mut m, 0xA89F), "the Cursor Device Manager is a served trap, not _Unimplemented");
    m.mac.mem.write_u32(DEV, 0xDEAD_BEEF).unwrap();
    // CursorDeviceNextDevice(&dev): procInfo 0x3E8 = D0-dispatched Pascal, OSErr, 4-byte selector, one 4-byte parameter
    cpu.gpr[3..7].copy_from_slice(&[at, 0x3E8, 0x0B, DEV]);
    assert!(m.native(&mut cpu, stub));
    assert_eq!((cpu.gpr[3], m.mac.mem.read_u32(DEV).unwrap()), (0, 0), "noErr and a NIL device: no cursor device is registered");
    assert!(m.fault().is_none(), "{:?}", m.fault());
    // Warcraft III registers its own device (and quits if it cannot): a real record, then found by NextDevice
    const NEW: u32 = DEV + 0x10;
    cpu.gpr[3..7].copy_from_slice(&[at, 0x3E8, 0x0C, NEW]);
    m.native(&mut cpu, stub);
    let d = m.mac.mem.read_u32(NEW).unwrap();
    assert_eq!(cpu.gpr[3], 0, "CursorDeviceNewDevice -> noErr");
    assert!(d != 0 && m.mac.mem.read_u32(d).unwrap() == 0, "a zeroed record at {d:#x}");
    cpu.gpr[3..7].copy_from_slice(&[at, 0x3E8, 0x0B, DEV]);
    m.native(&mut cpu, stub);
    assert_eq!(m.mac.mem.read_u32(DEV).unwrap(), d, "NextDevice(NIL) is the registered device");
    // a warp on it is accepted and named (the host owns the pointer); NEGATIVE CONTROL: a device that was never registered is paramErr
    cpu.gpr[3..9].copy_from_slice(&[at, 0x3FE8, 0x01, d, 320, 240]);
    m.native(&mut cpu, stub);
    assert_eq!(cpu.gpr[3], 0);
    assert!(m.mac.refused.iter().any(|r| r.contains("CursorDeviceMoveTo")), "{:?}", m.mac.refused);
    cpu.gpr[3..9].copy_from_slice(&[at, 0x3FE8, 0x01, 0x0003_A000, 320, 240]);
    m.native(&mut cpu, stub);
    assert_eq!(cpu.gpr[3] as i32, -50);
}

#[test]
fn a_powerpc_routine_runs_through_the_callback_machine_and_returns_its_result() {
    let (mut m, mut cpu, stub) = setup();
    let upp = new_routine_descriptor(&mut m.mac, TV, 0x3F1, ISA_PPC).unwrap();
    // procInfo 0x3F1: C stack-based, a 4-byte result, two 4-byte parameters
    cpu.gpr[3..7].copy_from_slice(&[upp, 0x3F1, 0x1111, 0x2222]);
    assert!(m.native(&mut cpu, stub));
    assert_eq!((cpu.pc, cpu.gpr[3], cpu.gpr[4], cpu.gpr[2]), (CODE, 0x1111, 0x2222, TOC), "the routine gets its two arguments");
    cpu.gpr[3] = 0x1234_5678;
    let ret = cpu.lr;
    assert!(m.native(&mut cpu, ret));
    assert_eq!((cpu.pc, cpu.gpr[3]), (CALLER_LR, 0x1234_5678));
}

#[test]
fn the_unimplemented_stub_and_nil_are_refused_by_name() {
    let (mut m, mut cpu, stub) = setup();
    // a Toolbox trap nobody registered answers with the _Unimplemented stub ($A89F; 0xA0FF was a SERVED trap's stub, which now runs as
    // 68k code and is refused here only for its D0-dispatched convention)
    let unimp = (0x300..0x3FF).find_map(|n| { let a = trap_stub(&mut m, n); (m.mac.mem.read_u16(a).unwrap() == 0xA89F).then_some(a) }).expect("an unregistered trap");
    cpu.gpr[3..6].copy_from_slice(&[unimp, 0x3E8, 0x0B]);
    m.native(&mut cpu, stub);
    assert!(m.fault().is_some_and(|f| f.starts_with("[HEIRLOOM-MACPPC CallUniversalProc REFUSED trap=0xa89f")), "{:?}", m.fault());
    let (mut m, mut cpu, stub) = setup();
    let served = trap_stub(&mut m, 0xA0FF);
    cpu.gpr[3..6].copy_from_slice(&[served, 0x3E8, 0x0B]);
    m.native(&mut cpu, stub);
    assert!(m.fault().is_some_and(|f| f.contains("kD0DispatchedPascalStackBased")), "{:?}", m.fault());
    let (mut m, mut cpu, stub) = setup();
    cpu.gpr[3..5].copy_from_slice(&[0, 0x3E8]);
    m.native(&mut cpu, stub);
    assert!(m.fault().is_some_and(|f| f.contains("upp=NIL")), "{:?}", m.fault());
}
