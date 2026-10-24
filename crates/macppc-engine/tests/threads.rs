//! The Thread Manager over the scheduler: real interpreted PowerPC threads yielding to each other. Main creates a thread, both log into shared
//! data in a fixed order, the thread returns (ThreadExit) and main reads its result.
use guestui_abi::RecordingGuestUi;
use macppc_engine::asm::Asm;
use macppc_engine::{MacPpc, Step};
use retro_pef::loader::SymbolClass;
use retro_pef::writer::PefBuilder;
use retro_pef::Pef;

type M = MacPpc<RecordingGuestUi>;
const F: SymbolClass = SymbolClass::TVector;
const T: &[&str] = &["NewThread", "GetCurrentThread", "YieldToAnyThread", "YieldToThread", "DisposeThread", "GetThreadState", "SetThreadState"];
const NEW: i32 = 0; const GETCUR: i32 = 1; const YIELD: i32 = 2; const YIELD_TO: i32 = 3; const DISPOSE: i32 = 4; const GETST: i32 = 5; const SETST: i32 = 6;
const TV: i32 = 16; // data: main TVector [0,8), the thread entry TVector [8,16), then one word per import
const DATA: i32 = TV + 4 * 7 + 4 * 2; // after the imports (+ the WaitNextEvent import slots, see build)

fn toc(i: i32) -> i32 { TV + 4 * i }
fn words(w: &[u32]) -> Vec<u8> { w.iter().flat_map(|x| x.to_be_bytes()).collect() }
fn prologue(a: &mut Asm) { a.mflr(0).stw(0, 8, 1).stwu(1, -64, 1); }
fn epilogue(a: &mut Asm) { a.addi(1, 1, 64).lwz(0, 8, 1).mtlr(0).blr(); }

/// A fragment importing ThreadsLib's calls (+ InterfaceLib's WaitNextEvent and one spare slot), with `entry` as the second TVector's code.
fn build(main: &Asm, entry: &[u8], data: &[u8]) -> Vec<u8> {
    let mut code = main.finish();
    let entry_at = code.len() as u32;
    code.extend_from_slice(entry);
    let mut b = PefBuilder::new();
    b.code(&code);
    let mut init = vec![0u8; DATA as usize];
    init[8..12].copy_from_slice(&entry_at.to_be_bytes());
    init.extend_from_slice(data);
    let d = b.data(&init, init.len() as u32);
    let imports: Vec<(&str, SymbolClass, bool)> = T.iter().map(|n| (*n, F, false)).collect();
    b.import_library("ThreadsLib", false, &imports);
    b.import_library("InterfaceLib", false, &[("WaitNextEvent", F, false), ("GetNextEvent", F, false)]);
    b.reloc(d as u16, &[0x4601, 0x4A00 | 8]);
    b.main(d as i32, 0);
    b.build()
}
fn boot(bytes: &[u8]) -> M {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let frag = m.instantiate("test", &Pef::parse(bytes).unwrap()).unwrap();
    m.start(&frag).unwrap();
    m
}
fn rd(m: &M, base: u32, off: i32) -> i32 { m.mac.mem.read_u32(base + off as u32).unwrap() as i32 }
fn base(m: &M) -> u32 { m.cpu().unwrap().gpr[2] }

// data layout after DATA: log[0..5], result, tid, main_tid, thread_tid, err_new, st_err, st, yield_err
const LOG: i32 = DATA; const RESULT: i32 = DATA + 20; const TID: i32 = DATA + 24; const MAINTID: i32 = DATA + 28; const THREADTID: i32 = DATA + 32;
const ERR_NEW: i32 = DATA + 36; const ST_ERR: i32 = DATA + 40; const ST: i32 = DATA + 44; const YERR: i32 = DATA + 48;

#[test]
fn two_threads_ping_pong_through_yields_and_the_exit_value_comes_back() {
    let mut e = Asm::new();
    prologue(&mut e);
    e.stw(3, LOG + 16, 2);                           // log[4] = the thread's parameter
    e.li(9, 1).stw(9, LOG, 2);                       // log[0] = 1
    e.call_toc(toc(YIELD));                          // back to main
    e.li(9, 3).stw(9, LOG + 8, 2);                   // log[2] = 3
    e.addi(3, 2, THREADTID).call_toc(toc(GETCUR));   // record my own id
    e.li(3, 0x1234);                                 // the exit value
    epilogue(&mut e);
    let mut a = Asm::new();
    prologue(&mut a);
    a.li(3, 1).addi(4, 2, 8).li(5, 0xAA).li(6, 0).li(7, 0).addi(8, 2, RESULT).addi(9, 2, TID).call_toc(toc(NEW));
    a.stw(3, ERR_NEW, 2);
    a.addi(3, 2, MAINTID).call_toc(toc(GETCUR));
    a.call_toc(toc(YIELD));                          // the thread runs: log[0], then yields back
    a.li(9, 2).stw(9, LOG + 4, 2);                   // log[1] = 2
    a.call_toc(toc(YIELD));                          // the thread finishes: log[2], its result; ThreadExit returns here
    a.li(9, 4).stw(9, LOG + 12, 2);                  // log[3] = 4
    a.lwz(3, TID, 2).addi(4, 2, ST).call_toc(toc(GETST)); // the finished thread no longer exists
    a.stw(3, ST_ERR, 2);
    a.lwz(3, TID, 2).call_toc(toc(YIELD_TO));        // nor can it be yielded to
    a.stw(3, YERR, 2);
    epilogue(&mut a);
    let mut m = boot(&build(&a, &e.finish(), &vec![0u8; 64]));
    let b = base(&m);
    let (step, insns) = m.run_loop(1_000_000);
    assert_eq!(step, Step::Exited, "{:?} {:#?}", m.fault(), m.mac.log);
    assert!(insns > 50);
    assert_eq!(rd(&m, b, ERR_NEW), 0);
    assert_eq!((0..5).map(|i| rd(&m, b, LOG + 4 * i)).collect::<Vec<_>>(), [1, 2, 3, 4, 0xAA], "the writes interleave exactly as the yields order them; the parameter arrived");
    assert_eq!(rd(&m, b, RESULT), 0x1234, "the thread's return value is stored at threadResult");
    let (main_id, thread_id) = (rd(&m, b, MAINTID), rd(&m, b, THREADTID));
    assert_eq!((main_id, thread_id), (1, rd(&m, b, TID)), "main is the application thread 1; GetCurrentThread inside the thread is the id NewThread made");
    assert_ne!(main_id, thread_id);
    assert_eq!(rd(&m, b, ST_ERR), -618, "threadNotFoundErr for the ended thread");
    assert_eq!(rd(&m, b, YERR), -618);
    assert_eq!(m.current_task(), 1);
    assert_eq!(m.callback_depth(), 0);
}

#[test]
fn a_new_suspended_thread_waits_for_setthreadstate_and_error_codes_are_exact() {
    // thread body: log[0] = 7, return
    let mut e = Asm::new();
    e.li(9, 7).stw(9, LOG, 2).li(3, 5).blr();
    let mut a = Asm::new();
    prologue(&mut a);
    a.li(3, 1).addi(4, 2, 8).li(5, 0).li(6, 0).li(7, 1).li(8, 0).addi(9, 2, TID).call_toc(toc(NEW)); // options = kNewSuspend
    a.call_toc(toc(YIELD));                                  // nothing is ready: returns at once, the thread has not run
    a.lwz(9, LOG, 2).stw(9, LOG + 4, 2);                     // log[1] = what log[0] was now (still 0)
    a.lwz(3, TID, 2).li(4, 0).li(5, 0).call_toc(toc(SETST)); // SetThreadState(tid, ready, none)
    a.stw(3, ST_ERR, 2);
    a.lwz(3, TID, 2).call_toc(toc(YIELD_TO));                // now it can be yielded to; it runs and ends
    a.lwz(3, TID, 2).li(4, 1).li(5, 0).call_toc(toc(SETST)); // the ended thread: not found
    a.stw(3, YERR, 2);
    a.li(3, 1).li(4, 0).li(5, 0).call_toc(toc(DISPOSE));     // DisposeThread(application thread) is a protocol error
    a.stw(3, ERR_NEW, 2);
    a.li(3, 99).addi(4, 2, ST).call_toc(toc(GETST));         // GetThreadState of an id that never existed
    a.stw(3, ST, 2);
    a.li(3, 9).li(4, 0).li(5, 0).li(6, 0).li(7, 0).li(8, 0).li(9, 0).call_toc(toc(NEW)); // NewThread with a style that is neither 1 nor 2
    a.stw(3, MAINTID, 2);
    epilogue(&mut a);
    let mut m = boot(&build(&a, &e.finish(), &vec![0u8; 64]));
    let b = base(&m);
    assert_eq!(m.run_loop(1_000_000).0, Step::Exited, "{:?} {:#?}", m.fault(), m.mac.log);
    assert_eq!(rd(&m, b, LOG + 4), 0, "a suspended thread does not run on a yield");
    assert_eq!(rd(&m, b, LOG), 7, "after SetThreadState(ready) and YieldToThread it did");
    assert_eq!(rd(&m, b, ST_ERR), 0);
    assert_eq!((rd(&m, b, YERR), rd(&m, b, ERR_NEW), rd(&m, b, ST), rd(&m, b, MAINTID)), (-618, -619, -618, -50));
}

#[test]
fn a_task_that_would_idle_yields_to_a_ready_thread_and_only_then_does_the_machine_block() {
    // thread: WaitNextEvent(0xFFFF, &rec, sleep 60, 0) parks (no event) -> idle-yield to main; main writes a marker and returns
    let wne = toc(7);
    let mut e = Asm::new();
    prologue(&mut e);
    e.li(3, -1).addi(4, 2, DATA + 80).li(5, 60).li(6, 0).call_toc(wne);
    epilogue(&mut e);
    let mut a = Asm::new();
    prologue(&mut a);
    a.li(3, 1).addi(4, 2, 8).li(5, 0).li(6, 0).li(7, 0).li(8, 0).addi(9, 2, TID).call_toc(toc(NEW));
    a.call_toc(toc(YIELD));                  // the thread runs, calls WaitNextEvent, would idle: control comes back here
    a.li(9, 42).stw(9, LOG, 2);              // marker: main got the CPU while the thread sleeps
    epilogue(&mut a);
    let mut m = boot(&build(&a, &e.finish(), &vec![0u8; 128]));
    let b = base(&m);
    let (step, _) = m.run_loop(1_000_000);
    assert_eq!(step, Step::Exited, "main returned instead of the machine blocking: {:?} {:#?}", m.fault(), m.mac.log);
    assert_eq!(rd(&m, b, LOG), 42);
    assert!(m.mac.log.iter().any(|l| l.contains("[sched] task 2 would idle: task 1 runs instead")), "{:#?}", m.mac.log);
    // NEGATIVE CONTROL: with no other ready task the same call parks the machine (Blocked)
    let mut a = Asm::new();
    prologue(&mut a);
    a.li(3, -1).addi(4, 2, DATA + 80).li(5, 60).li(6, 0).call_toc(wne);
    epilogue(&mut a);
    let mut m = boot(&build(&a, &e.finish(), &vec![0u8; 128]));
    assert_eq!(m.run_loop(1_000_000).0, Step::Blocked);
}

#[test]
fn a_switch_binds_only_the_stubs_interned_since_the_task_last_ran() {
    // main and a thread yield to each other 100 times: 200 switches. The glue table has a few dozen stubs; the OLD switch re-bound all of them every time.
    let mut e = Asm::new();
    prologue(&mut e);
    e.label("t");
    e.call_toc(toc(YIELD));
    e.b("t");
    epilogue(&mut e);
    let mut a = Asm::new();
    prologue(&mut a);
    a.li(3, 1).addi(4, 2, 8).li(5, 0).li(6, 0).li(7, 0).addi(8, 2, RESULT).addi(9, 2, TID).call_toc(toc(NEW));
    a.li(9, 100).stw(9, LOG, 2);
    a.label("loop");
    a.call_toc(toc(YIELD));
    a.lwz(9, LOG, 2).addi(9, 9, -1).stw(9, LOG, 2).cmpwi(9, 0).bne("loop");
    epilogue(&mut a);
    let mut m = boot(&build(&a, &e.finish(), &vec![0u8; 64]));
    let (step, _) = m.run_loop(2_000_000);
    assert_eq!(step, Step::Exited, "{:?}", m.fault());
    let stubs = m.glue.len() as u64;
    assert!(stubs >= 10, "a realistic table: {stubs}");
    // each of the two tasks needs its own CPU bound once (their first run) and nothing per further switch
    assert!(m.switch_rebinds() <= 2 * stubs, "{} rebinds for 200 switches over {stubs} stubs: a switch must not rebind what the task has", m.switch_rebinds());
}
