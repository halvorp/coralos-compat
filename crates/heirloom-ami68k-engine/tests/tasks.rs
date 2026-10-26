//! Several guest tasks in one engine: AddTask / CreateNewProc, scheduling in virtual time, Signal/Wait/PutMsg across tasks,
//! Forbid, semaphores, RemTask. Driven by compiled 68k programs (tests/guest/c/tasks_*.c); the first runs Directory Opus's
//! IPC startup handshake (Library/ipc.c of the DOpus 5 source) between a parent and a child it created.
mod common;
use common::*;

const PROC: &[u8] = include_bytes!("guest/tasks_proc.bin");
const SCHED: &[u8] = include_bytes!("guest/tasks_sched.bin");

fn run_all(a: &mut heirloom_ami68k_engine::Amiga<heirloom_ui_abi::RecordingGuestUi>, blob: &[u8]) -> Vec<u32> {
    use heirloom_fmt_hunk::writer::{build, Hunk};
    let r = a.run_hunk(&build(&[Hunk::code(blob.to_vec(), vec![])], None), 40_000_000).unwrap();
    assert!(r.stats.halted && r.stats.bus_faults == 0, "{:?} {:?}\n{:#?}", r.stats, a.refused, a.log);
    (0..128).map(|i| a.mem.read_u32(RES + 4 * i).unwrap()).collect()
}

#[test]
fn a_parent_creates_a_process_and_they_do_the_dopus_startup_handshake() {
    let mut a = machine();
    let r = run_all(&mut a, PROC);
    assert_eq!(r[0], 1, "CreateMsgPort");
    assert_eq!((r[1], r[2], r[3], r[4]), (1, 8192, 1, 1), "CreateNewProc -> a Process with NP_StackSize 8192, named \"child\", found by FindTask(name)");
    assert_eq!((r[5], r[6], r[7], r[8]), (8, 1, 13, 0xFFFF_FFFF), "pr_MsgPort on SIGB_DOS with mp_SigTask = the process; NT_PROCESS; NP_WindowPtr -1");
    assert_eq!(r[9], 1, "the child had not run when the parent sent the startup message (it runs only when the parent waits)");
    assert_eq!((r[10], r[11]), (1, 0xC0DE), "the child replied to the parent's reply port after answering in the message");
    assert_eq!((r[12], r[13]), (1, 0x1234), "the child's FindTask(0) is its own Process; tc_UserData is its own");
    assert_eq!((r[14], r[15]), (16, 17), "signal bits are allocated per task: the child's first port gets 16 as the parent's did");
    assert_eq!(r[16], 1, "the child ran after Delay()");
    assert_eq!(a.clock_us, 60_000, "Delay(3) slept 3 ticks of virtual time while the parent was blocked");
    assert_eq!(r[17], 1, "the child returned from its entry: its Process is gone (FindTask(name) = NULL)");
    assert_eq!(r[18], 1, "a second process with NP_Cli + NP_Arguments");
    assert_eq!((r[21], r[22], r[23], r[24]), (6, 1, 1, 1), "A0/D0 = the argument line and its length; pr_CLI and pr_Arguments set");
    assert_eq!((r[19], r[20]), (0, 116), "negative control: neither NP_Entry nor NP_Seglist fails with ERROR_REQUIRED_ARG_MISSING");
    // Every refusal is named, and these are all of them.
    let refused: Vec<&str> = a.refused.iter().map(String::as_str).collect();
    assert_eq!(refused.len(), 3, "{refused:?}");
    assert!(refused[0].starts_with("exec.library/Wait:") && refused[0].contains("signals 0x2000000 are not allocated"), "{refused:?}");
    assert!(refused[1].starts_with("exec.library/RemTask:") && refused[1].contains("0xdead00 is not a task"), "{refused:?}");
    assert!(refused[2].starts_with("exec.library/Signal:") && refused[2].contains("0xdead00 is not a task"), "{refused:?}");
    assert!(a.sched.tasks.is_empty() && a.sched.dropped.is_empty(), "both children had ended by themselves");
}

#[test]
fn tasks_share_the_cpu_by_priority_quantum_forbid_and_semaphores() {
    let mut a = machine();
    let r = run_all(&mut a, SCHED);
    // A. round robin
    assert_eq!(r[0], 1, "AddTask of equal-priority tasks does not preempt the adder");
    assert_eq!(r[3], 1, "FindTask(name) finds both");
    assert_eq!((r[4], r[5]), (1, 1), "both busy tasks made progress while the main task spun, and both ended when told to");
    // B. preemption
    assert_eq!((r[10], r[12]), (1, 1), "AddTask of a higher-priority task runs it at once; it then sleeps in TaskWait (found by name)");
    assert_eq!((r[26], r[27], r[28], r[29]), (1, 0, 1, 4), "a guest walking ExecBase: TaskWait holds exactly H (TS_WAIT), TaskReady is empty, ThisTask is the main task");
    assert_eq!((r[45], r[46]), (5, 3), "SetTaskPri returns the old priority and sets the new one");
    assert_eq!((r[14], r[17], r[19]), (1, 7, 1), "Signal of a higher-priority sleeper preempts at once; it saw the marker; it ended");
    assert_eq!(r[20], 0, "negative control: a LOWER priority task does not run when added");
    assert_eq!((r[22], r[23], r[25]), (16, 0, 1), "L ran once the main task slept; Signal(L) did not preempt; L finished while main slept");
    // C. Forbid / Disable
    assert_eq!((r[32], r[33], r[34]), (0, 0, 1), "no switch under Forbid, none under a nested one, the switch happens at the last Permit");
    assert_eq!((r[37], r[38]), (0, 1), "Disable/Enable act as Forbid/Permit");
    assert_eq!((r[40], r[41]), (0, 1), "a quantum switches nothing under Forbid, and switches to the equal-priority task after Permit");
    assert_eq!(r[47], 1, "a task that returns runs its finalPC, which ran after the entry and removed the task (nothing after RemTask(0) ran)");
    assert_eq!(r[42], 1, "RemTask of another task");
    assert_eq!(r[44], 4192, "RemTask freed the memory in the task's tc_MemEntry list");
    // D. semaphores
    assert_eq!((r[50], r[52], r[53], r[54]), (0, 1, 0, 1), "a semaphore held by another task blocks the caller until released");
    // E. the main program ends with tasks alive
    assert_eq!(a.sched.dropped.len(), 2, "{:?}", a.sched.dropped);
    assert!(a.log.iter().any(|l| l.contains("the main program exited; dropping 2 task(s)") && l.contains("\"E1\"") && l.contains("\"E2\"")));
    assert!(a.sched.tasks.is_empty());
    // Refusals: SetTaskPri of a non-task, Wait(0), and the release of a semaphore nobody holds; nothing else.
    let refused: Vec<&str> = a.refused.iter().map(String::as_str).collect();
    assert_eq!(refused.len(), 3, "{refused:?}");
    assert!(refused[0].starts_with("exec.library/SetTaskPri:") && refused[0].contains("0xdead00 is not a task"), "{refused:?}");
    assert!(refused[1].starts_with("exec.library/Wait:"), "{refused:?}");
    assert!(refused[2].contains("ReleaseSemaphore") && refused[2].contains("is not held"), "{refused:?}");
}
