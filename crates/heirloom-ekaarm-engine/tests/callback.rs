//! The callback machine on hand-assembled code in a process with no image (`loader::bare`, `Process::map_code`): a native
//! calling a guest function, two levels of nesting, a Thumb target, and the refusals. Runs without the owner's corpus.
use std::cell::Cell;
use std::rc::Rc;
use heirloom_ekaarm_engine::callback::{Continuation, CALLBACK_STUB, MAX_CALLBACK_DEPTH};
use heirloom_ekaarm_engine::layout::{CODE_BASE, EXIT_SVC, STUB_SVC_BASE};
use heirloom_ekaarm_engine::{loader, Outcome};

const SVC_STUB0: u32 = 0xEF00_0000 | STUB_SVC_BASE;
const SVC_EXIT: u32 = 0xEF00_0000 | EXIT_SVC;
const BX_LR: u32 = 0xE12F_FF1E;
const ADD_R0_1: u32 = 0xE280_0001; // add r0, r0, #1
const ADD_R0_100: u32 = 0xE280_0064; // add r0, r0, #100
const PUSH_LR: u32 = 0xE92D_4000; // stmfd sp!, {lr}
const POP_PC: u32 = 0xE8BD_8000; // ldmfd sp!, {pc}

fn words(w: &[u32]) -> Vec<u8> {
    w.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// A process whose stub 0 is `__call_guest`: r0 = target, r1/r2 = the callee's r0/r1. The depth after each push is recorded.
fn machine(code: &[(u32, &[u32])]) -> (heirloom_ekaarm_engine::Process, Rc<Cell<usize>>) {
    let mut p = loader::bare(&["__call_guest"]).unwrap();
    for (at, w) in code {
        p.map_code(*at, &words(w)).unwrap();
    }
    let deepest = Rc::new(Cell::new(0));
    let seen = deepest.clone();
    p.native_hook = Some(Box::new(move |proc, name| {
        if name != "__call_guest" {
            return None;
        }
        let (target, a, b) = (proc.cpu.regs[0], proc.cpu.regs[1], proc.cpu.regs[2]);
        Some(match proc.call_guest(target, &[a, b], Continuation::Resume) {
            Ok(()) => {
                seen.set(seen.get().max(proc.callback_depth()));
                heirloom_ekaarm_engine::natives::NativeOutcome::Return
            }
            Err(e) => heirloom_ekaarm_engine::natives::NativeOutcome::Refused(e),
        })
    }));
    p.cpu.set_pc_interworking(CODE_BASE);
    (p, deepest)
}

const MAIN: &[u32] = &[SVC_STUB0, SVC_EXIT]; // call the guest function, then exit with r0

#[test]
fn a_native_calls_a_guest_function_and_gets_its_result() {
    let g = CODE_BASE + 0x1000;
    let (mut p, deepest) = machine(&[(CODE_BASE, MAIN), (g, &[ADD_R0_1, BX_LR])]);
    p.cpu.regs[0] = g;
    p.cpu.regs[1] = 41;
    assert!(matches!(p.run(1000), Outcome::Exited(42)));
    assert_eq!(deepest.get(), 1);
    assert_eq!(p.callback_depth(), 0);
}

#[test]
fn a_callback_that_reaches_a_native_calling_guest_code_nests_two_levels() {
    let (g1, g2) = (CODE_BASE + 0x1000, CODE_BASE + 0x2000);
    // g1 is called with (r0 = g2, r1 = 7): it calls the native again as it is (target g2, argument 7), adds 100, returns
    let (mut p, deepest) = machine(&[
        (CODE_BASE, MAIN),
        (g1, &[PUSH_LR, SVC_STUB0, ADD_R0_100, POP_PC]),
        (g2, &[ADD_R0_1, BX_LR]),
    ]);
    p.cpu.regs[0] = g1;
    p.cpu.regs[1] = g2;
    p.cpu.regs[2] = 7;
    assert!(matches!(p.run(1000), Outcome::Exited(108)), "7 + 1 in g2, + 100 in g1");
    assert_eq!(deepest.get(), 2, "two frames were live at once");
    assert_eq!(p.callback_depth(), 0);
}

#[test]
fn a_thumb_target_is_entered_in_thumb_state_and_returns_to_the_arm_stub() {
    let t = CODE_BASE + 0x1000;
    // thumb: adds r0, #1 (0x3001); bx lr (0x4770)
    let (mut p, _) = machine(&[(CODE_BASE, MAIN), (t, &[0x4770_3001])]);
    p.cpu.regs[0] = t | 1;
    p.cpu.regs[1] = 9;
    assert!(matches!(p.run(1000), Outcome::Exited(10)));
    assert!(!p.cpu.thumb, "back in the interrupted ARM code");
}

#[test]
fn the_interrupted_registers_come_back_but_for_r0() {
    let g = CODE_BASE + 0x1000;
    // g clobbers r4 and r5; the interrupted code's r4 must survive
    let mov_r4 = 0xE3A0_4063u32; // mov r4, #99
    let mov_r5 = 0xE3A0_5001u32;
    let (mut p, _) = machine(&[(CODE_BASE, MAIN), (g, &[mov_r4, mov_r5, ADD_R0_1, BX_LR])]);
    p.cpu.regs[0] = g;
    p.cpu.regs[1] = 1;
    p.cpu.regs[4] = 1234;
    assert!(matches!(p.run(1000), Outcome::Exited(2)));
    assert_eq!(p.cpu.regs[4], 1234);
}

#[test]
fn a_null_target_and_a_runaway_nest_are_refused_by_name() {
    let (mut p, _) = machine(&[(CODE_BASE, MAIN)]);
    p.cpu.regs[0] = 0;
    match p.run(100) {
        Outcome::Refused(m) => assert!(m.contains("null function pointer"), "{m}"),
        o => panic!("{o:?}"),
    }
    // a guest function that calls the native on itself forever: refused at the depth limit, never a host stack overflow
    let g = CODE_BASE + 0x1000;
    let (mut p, deepest) = machine(&[(CODE_BASE, MAIN), (g, &[PUSH_LR, 0xE1A0_1000 /* mov r1, r0 */, SVC_STUB0, POP_PC])]);
    p.cpu.regs[0] = g;
    p.cpu.regs[1] = g;
    p.cpu.regs[2] = 0;
    match p.run(100_000) {
        Outcome::Refused(m) => assert!(m.contains("nesting deeper than"), "{m}"),
        o => panic!("{o:?}"),
    }
    assert_eq!(deepest.get(), MAX_CALLBACK_DEPTH);
}

#[test]
fn the_return_stub_with_no_frame_is_a_fault_not_a_jump_to_nowhere() {
    let (mut p, _) = machine(&[(CODE_BASE, MAIN)]);
    p.cpu.set_pc_interworking(CALLBACK_STUB);
    match p.run(10) {
        Outcome::Fault(m) => assert!(m.contains("no callback frame"), "{m}"),
        o => panic!("{o:?}"),
    }
}
