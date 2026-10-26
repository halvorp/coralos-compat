//! The Time Manager (spec: macos9-specs/TimeManager/{InsTime,PrimeTime,RmvTime}): TMTask bookkeeping in guest memory, the extended-task cadence, and a due task's procedure running at the
//! next safepoint (a glue call) with the interrupted call resumed after it.
use heirloom_ui_abi::RecordingGuestUi;
use heirloom_macppc_engine::symbols::Binding;
use heirloom_macppc_engine::{GlueOutcome, MacPpc, PpcHost};
use heirloom_mac68k_engine::mixed_mode::{new_routine_descriptor, ISA_PPC};
use heirloom_cpu_ppc::{Cpu, Host};

type M = MacPpc<RecordingGuestUi>;
const T: u32 = 0x3400;
const TV: u32 = 0x0003_0000;
const CODE: u32 = 0x0003_1000;
const TOC: u32 = 0x0003_8000;
const SP: u32 = 0x0004_0000;

fn call(m: &mut M, name: &str, args: &[u32]) -> i32 {
    assert!(!matches!(m.bind(name), Binding::Unbound(_)), "{name}");
    let mut regs = [0u32; 32];
    regs[3..3 + args.len()].copy_from_slice(args);
    assert_eq!(m.glue_call(name, &mut regs), GlueOutcome::Return);
    regs[3] as i32
}
fn q(m: &M, t: u32) -> u16 { m.mac.mem.read_u16(t + 4).unwrap() }
fn count(m: &M, t: u32) -> i32 { m.mac.mem.read_u32(t + 0xA).unwrap() as i32 }

#[test]
fn instime_primetime_rmvtime_follow_the_spec_vectors() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    m.mac.mem.write_u16(T + 4, 0xFFE3).unwrap(); // junk in the high bits and a queue type 3
    // InsTime clears every bit above the low five; InsXTime sets 0x4000
    assert_eq!(call(&mut m, "InsTime", &[T]), 0);
    assert_eq!(q(&m, T), 0x0003);
    assert_eq!(call(&mut m, "InsXTime", &[T + 0x40]), 0);
    assert_eq!(q(&m, T + 0x40) & 0x6000, 0x4000);
    // PrimeTime on a task never inserted: -1; on an inserted one: tmCount = count, active; re-priming an active task is ignored (0, count unchanged)
    assert_eq!(call(&mut m, "PrimeTime", &[T + 0x80, 100]), -1);
    assert_eq!(call(&mut m, "PrimeTime", &[T, 100]), 0);
    assert_eq!((q(&m, T) & 0x8000, count(&m, T)), (0x8000, 100));
    assert_eq!(call(&mut m, "PrimeTime", &[T, 500]), 0);
    assert_eq!(count(&m, T), 100, "NEGATIVE control: an active task is not re-primed");
    // RmvTime 40 ms (2.4 ticks -> 2 ticks = 33 ms) into the 100 ms prime: the time left is returned in tmCount, the task is inactive and forgotten; again: -1
    m.mac.advance_ticks(2);
    assert_eq!(call(&mut m, "RmvTime", &[T]), 0);
    assert_eq!(q(&m, T) & 0x8000, 0);
    assert!((60..=70).contains(&count(&m, T)), "about 100 - 33 ms left: {}", count(&m, T));
    assert_eq!(call(&mut m, "RmvTime", &[T]), -1);
    // a task removed while NOT active reports 0 left
    call(&mut m, "InsTime", &[T]);
    assert_eq!(call(&mut m, "RmvTime", &[T]), 0);
    assert_eq!(count(&m, T), 0);
    // the 385th concurrently inserted task fails (-1)
    for i in 0..383u32 { assert_eq!(call(&mut m, "InsTime", &[0x10_0000 + 16 * i]), 0); } // one extended task is still inserted: 384 in all
    assert_eq!(call(&mut m, "InsTime", &[0x10_0000 + 16 * 383]), -1);
}

#[test]
fn a_due_task_runs_at_the_next_safepoint_then_the_interrupted_call_resumes() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    m.mac.mem.write_u32(TV, CODE).unwrap();
    m.mac.mem.write_u32(TV + 4, TOC).unwrap();
    let upp = new_routine_descriptor(&mut m.mac, TV, 0x2C0, ISA_PPC).unwrap();
    m.mac.mem.write_u32(T + 6, upp).unwrap(); // tmAddr
    call(&mut m, "InsTime", &[T]);
    call(&mut m, "PrimeTime", &[T, 100]); // 100 ms = 6 ticks
    m.bind("TickCount");
    let id = m.glue.intern(&mut m.mac, "CFM", "TickCount").unwrap();
    let stub = m.glue.entry(id).unwrap().stub;
    let mut cpu = Cpu::new(0x5000, SP);
    cpu.gpr[2] = 0x6000;
    // not yet due: the glue call is just the call (the task's procedure is not entered)
    m.mac.advance_ticks(5);
    m.native(&mut cpu, stub);
    assert_ne!(cpu.pc, CODE);
    assert_eq!(q(&m, T) & 0x8000, 0x8000, "still active");
    // due: the safepoint enters the task with r3 = the TMTask, the active bit already clear
    let mut cpu = Cpu::new(0x5000, SP);
    cpu.gpr[2] = 0x6000; cpu.gpr[3] = 0x77;
    m.mac.advance_ticks(1);
    assert!(m.native(&mut cpu, stub));
    assert_eq!((cpu.pc, cpu.gpr[3], cpu.gpr[2]), (CODE, T, TOC));
    assert_eq!(q(&m, T) & 0x8000, 0);
    assert_eq!(m.callback_depth(), 1);
    // no nesting and no second run: another glue call from inside the task is just the call
    // the task returns to $CallbackReturn: the interrupted call (same stub, registers restored) is entered again
    let ret = cpu.lr;
    assert!(m.native(&mut cpu, ret));
    assert_eq!((cpu.pc, cpu.gpr[3], cpu.gpr[2]), (stub, 0x77, 0x6000));
    assert_eq!(m.callback_depth(), 0);
    // NEGATIVE control: it does not fire again (the task is no longer primed)
    m.mac.advance_ticks(60);
    let mut cpu = Cpu::new(0x5000, SP);
    m.native(&mut cpu, stub);
    assert_ne!(cpu.pc, CODE);
}

#[test]
fn gestalt_tmgr_is_4() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    assert_eq!(heirloom_macppc_engine::gestalt_ppc::answer(*b"tmgr"), Some(4));
    let _ = &mut m;
}
