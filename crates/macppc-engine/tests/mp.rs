//! Multiprocessing Services over the scheduler: interpreted PowerPC tasks, queues, semaphores, critical regions, timeouts; cooperative only.
use guestui_abi::RecordingGuestUi;
use macppc_engine::asm::Asm;
use macppc_engine::{MacPpc, Step};
use retro_pef::loader::SymbolClass;
use retro_pef::writer::PefBuilder;
use retro_pef::Pef;

type M = MacPpc<RecordingGuestUi>;
const F: SymbolClass = SymbolClass::TVector;
const N: &[&str] = &["MPCreateQueue", "MPCreateTask", "MPWaitOnQueue", "MPNotifyQueue", "MPYield", "MPCreateSemaphore", "MPWaitOnSemaphore", "MPSignalSemaphore",
    "MPCreateCriticalRegion", "MPEnterCriticalRegion", "MPExitCriticalRegion", "MPTerminateTask", "MPCurrentTaskID", "MPProcessors", "MPDeleteQueue"];
const CQ: i32 = 0; const CT: i32 = 1; const WQ: i32 = 2; const NQ_: i32 = 3; const YLD: i32 = 4; const CS: i32 = 5; const WS: i32 = 6; const SS: i32 = 7;
const CR: i32 = 8; const ER: i32 = 9; const XR: i32 = 10; const TT: i32 = 11; const CUR: i32 = 12; const PROCS: i32 = 13; const DQ: i32 = 14;
const TOC0: i32 = 16;
const D: i32 = TOC0 + 4 * 15; // data after the imports
// data slots (words): 0 forever, 1 Q, 2 NQ, 3 T, 4 S, 5 R, 6.. results
const FOREVER: i32 = D; const Q: i32 = D + 4; const NQ: i32 = D + 8; const T: i32 = D + 12; const S: i32 = D + 16; const R: i32 = D + 20;
const RES: i32 = D + 32; // results at RES + 4*i
const TQ: i32 = D + 128; // the task's saved parameter

fn toc(i: i32) -> i32 { TOC0 + 4 * i }
fn prologue(a: &mut Asm) { a.mflr(0).stw(0, 8, 1).stwu(1, -64, 1); }
fn epilogue(a: &mut Asm) { a.addi(1, 1, 64).lwz(0, 8, 1).mtlr(0).blr(); }
fn build(main: &Asm, entry: &[u8]) -> Vec<u8> {
    let mut code = main.finish();
    let at = code.len() as u32;
    code.extend_from_slice(entry);
    let mut b = PefBuilder::new();
    b.code(&code);
    let mut init = vec![0u8; 256 + D as usize];
    init[8..12].copy_from_slice(&at.to_be_bytes());
    init[FOREVER as usize..FOREVER as usize + 4].copy_from_slice(&0x7FFF_FFFFu32.to_be_bytes());
    let d = b.data(&init, init.len() as u32);
    let imports: Vec<(&str, SymbolClass, bool)> = N.iter().map(|n| (*n, F, false)).collect();
    b.import_library("MPLibrary", false, &imports);
    b.reloc(d as u16, &[0x4601, 0x4A00 | 14]);
    b.main(d as i32, 0);
    b.build()
}
fn boot(bytes: &[u8]) -> M {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let frag = m.instantiate("test", &Pef::parse(bytes).unwrap()).unwrap();
    m.start(&frag).unwrap();
    m
}
fn rd(m: &M, b: u32, off: i32) -> i32 { m.mac.mem.read_u32(b + off as u32).unwrap_or(0xDEAD) as i32 }
fn nothing() -> Vec<u8> { let mut e = Asm::new(); e.li(3, 0).blr(); e.finish() }

#[test]
fn a_producer_task_feeds_a_queue_the_main_task_waits_on_and_its_termination_notice_arrives() {
    let mut e = Asm::new();
    prologue(&mut e);
    e.stw(3, TQ, 2);
    e.lwz(3, TQ, 2).li(4, 11).li(5, 22).li(6, 33).call_toc(toc(NQ_));
    e.call_toc(toc(YLD));
    e.lwz(3, TQ, 2).li(4, 44).li(5, 55).li(6, 66).call_toc(toc(NQ_));
    e.li(3, 7);
    epilogue(&mut e);
    let mut a = Asm::new();
    prologue(&mut a);
    a.addi(3, 2, Q).call_toc(toc(CQ));
    a.addi(3, 2, NQ).call_toc(toc(CQ));
    a.addi(3, 2, 8).lwz(4, Q, 2).li(5, 0).lwz(6, NQ, 2).li(7, 111).li(8, 222).li(9, 0).addi(10, 2, T).call_toc(toc(CT));
    a.stw(3, RES, 2);
    for (k, base) in [(0, RES + 4), (1, RES + 20)] {
        let _ = k;
        a.lwz(3, Q, 2).addi(4, 2, base).addi(5, 2, base + 4).addi(6, 2, base + 8).lwz(7, FOREVER, 2).call_toc(toc(WQ));
        a.stw(3, base + 12, 2);
    }
    // the termination notice: (111, 222, the status 7)
    a.lwz(3, NQ, 2).addi(4, 2, RES + 36).addi(5, 2, RES + 40).addi(6, 2, RES + 44).lwz(7, FOREVER, 2).call_toc(toc(WQ));
    a.stw(3, RES + 48, 2);
    a.call_toc(toc(PROCS)).stw(3, RES + 52, 2);
    epilogue(&mut a);
    let mut m = boot(&build(&a, &e.finish()));
    let b = m.cpu().unwrap().gpr[2];
    let (step, _) = m.run_loop(1_000_000);
    assert_eq!(step, Step::Exited, "{:?} {:#?}", m.fault(), m.mac.log);
    assert_eq!(rd(&m, b, RES), 0, "MPCreateTask");
    assert_eq!((rd(&m, b, RES + 4), rd(&m, b, RES + 8), rd(&m, b, RES + 12), rd(&m, b, RES + 16)), (11, 22, 33, 0), "the first message, noErr");
    assert_eq!((rd(&m, b, RES + 32), rd(&m, b, RES + 20), rd(&m, b, RES + 24), rd(&m, b, RES + 28)), (0, 44, 55, 66), "second");
    assert_eq!((rd(&m, b, RES + 36), rd(&m, b, RES + 40), rd(&m, b, RES + 44), rd(&m, b, RES + 48)), (111, 222, 7, 0), "the termination notice: the two parameters and the task's status");
    assert_eq!(rd(&m, b, RES + 52), 1, "MPProcessors is one: nothing here runs in parallel");
    assert!(m.mac.log.iter().any(|l| l.contains("[mp] task 1 waits (MPWaitOnQueue)")), "{:#?}", m.mac.log);
}

#[test]
fn semaphores_time_out_hand_over_and_report_bad_ids_and_a_wait_nobody_can_end_is_a_named_deadlock() {
    // main: a semaphore (max 2, initial 0); wait 100 ms with no other task -> the clock advances, kMPTimeoutErr; a task signals it; the wait then succeeds
    let mut e = Asm::new();
    prologue(&mut e);
    e.lwz(3, S, 2).call_toc(toc(SS));
    epilogue(&mut e);
    let mut a = Asm::new();
    prologue(&mut a);
    a.li(3, 2).li(4, 0).addi(5, 2, S).call_toc(toc(CS));                       // MPCreateSemaphore(2, 0, &S)
    a.lwz(3, S, 2).li(4, 100).call_toc(toc(WS)).stw(3, RES, 2);                // MPWaitOnSemaphore(S, 100 ms): times out
    a.lwz(3, S, 2).li(4, 0).call_toc(toc(WS)).stw(3, RES + 4, 2);              // immediate: also a timeout
    a.addi(3, 2, 8).lwz(4, S, 2).li(5, 0).li(6, 0).li(7, 0).li(8, 0).li(9, 0).addi(10, 2, T).call_toc(toc(CT)); // the signalling task
    a.lwz(3, S, 2).lwz(4, FOREVER, 2).call_toc(toc(WS)).stw(3, RES + 8, 2);    // forever: the task signals, we wake with 0
    a.li(3, 0x4321).call_toc(toc(SS)).stw(3, RES + 12, 2);                     // an ID that is nobody's
    a.lwz(3, Q, 2).call_toc(toc(SS)).stw(3, RES + 16, 2);                      // Q is 0 here: not a semaphore either
    a.lwz(3, S, 2).call_toc(toc(SS)).call_toc(toc(YLD));                        // count 1
    a.lwz(3, S, 2).call_toc(toc(SS)).stw(3, RES + 20, 2);                      // count 2
    a.lwz(3, S, 2).call_toc(toc(SS)).stw(3, RES + 24, 2);                      // beyond the maximum
    epilogue(&mut a);
    let mut m = boot(&build(&a, &e.finish()));
    let b = m.cpu().unwrap().gpr[2];
    let t0 = m.mac.ev.ticks;
    let (step, _) = m.run_loop(1_000_000);
    assert_eq!(step, Step::Exited, "{:?} {:#?}", m.fault(), m.mac.log);
    assert_eq!(rd(&m, b, RES), -29296, "kMPTimeoutErr");
    assert!(m.mac.ev.ticks - t0 >= 6, "100 ms is 6 ticks and the sleeping machine's clock advanced to the deadline: {}", m.mac.ev.ticks - t0);
    assert_eq!((rd(&m, b, RES + 4), rd(&m, b, RES + 8)), (-29296, 0));
    assert_eq!((rd(&m, b, RES + 12), rd(&m, b, RES + 16)), (-29299, -29299), "kMPInvalidIDErr");
    assert_eq!((rd(&m, b, RES + 20), rd(&m, b, RES + 24)), (0, -29298), "signalling past the maximum count: kMPInsufficientResourcesErr");
    // NEGATIVE CONTROL: a forever wait with nobody to signal stops the run NAMING the wait
    let mut a = Asm::new();
    prologue(&mut a);
    a.li(3, 2).li(4, 0).addi(5, 2, S).call_toc(toc(CS));
    a.lwz(3, S, 2).lwz(4, FOREVER, 2).call_toc(toc(WS));
    epilogue(&mut a);
    let mut m = boot(&build(&a, &nothing()));
    let (step, _) = m.run_loop(1_000_000);
    assert!(matches!(&step, Step::Fault(w) if w.contains("MPWaitOnSemaphore") && w.contains("deadlock")), "{step:?}");
}

#[test]
fn a_critical_region_excludes_the_other_task_until_it_is_left() {
    // main enters R; the task tries to enter and blocks; main logs, leaves R (ownership passes); the task logs after
    let mut e = Asm::new();
    prologue(&mut e);
    e.stw(3, TQ, 2);
    e.lwz(3, TQ, 2).lwz(4, FOREVER, 2).call_toc(toc(ER));  // blocks until main leaves
    e.li(9, 2).stw(9, RES + 8, 2);                          // log[1] = 2: the region was handed over
    e.lwz(3, TQ, 2).call_toc(toc(XR));
    epilogue(&mut e);
    let mut a = Asm::new();
    prologue(&mut a);
    a.addi(3, 2, R).call_toc(toc(CR));
    a.lwz(3, R, 2).lwz(4, FOREVER, 2).call_toc(toc(ER)).stw(3, RES, 2);          // main enters
    a.addi(3, 2, 8).lwz(4, R, 2).li(5, 0).li(6, 0).li(7, 0).li(8, 0).li(9, 0).addi(10, 2, T).call_toc(toc(CT));
    a.call_toc(toc(YLD));                                                        // the task runs, blocks on the region, main resumes
    a.li(9, 1).stw(9, RES + 4, 2);                                               // log[0] = 1 inside the region
    a.lwz(3, R, 2).call_toc(toc(XR)).stw(3, RES + 12, 2);                        // leave: the task now owns it
    a.lwz(3, R, 2).li(4, 0).call_toc(toc(ER)).stw(3, RES + 16, 2);               // main cannot take it back without waiting: immediate = timeout
    a.call_toc(toc(YLD));                                                        // the task logs and leaves
    a.lwz(3, R, 2).li(4, 0).call_toc(toc(ER)).stw(3, RES + 20, 2);               // now free: main enters
    a.addi(3, 2, Q).call_toc(toc(CQ));
    a.lwz(3, Q, 2).call_toc(toc(XR)).stw(3, RES + 24, 2);                        // a queue id is not a region: invalid
    epilogue(&mut a);
    let mut m = boot(&build(&a, &e.finish()));
    let b = m.cpu().unwrap().gpr[2];
    let (step, _) = m.run_loop(1_000_000);
    assert_eq!(step, Step::Exited, "{:?} {:#?}", m.fault(), m.mac.log);
    assert_eq!((rd(&m, b, RES), rd(&m, b, RES + 4), rd(&m, b, RES + 8)), (0, 1, 2), "main logged 1 inside the region, the task logged 2 only after main left it");
    assert_eq!((rd(&m, b, RES + 12), rd(&m, b, RES + 16), rd(&m, b, RES + 20), rd(&m, b, RES + 24)), (0, -29296, 0, -29299));
}

#[test]
fn deleting_a_queue_wakes_its_waiter_with_kmpdeletederr() {
    let mut e = Asm::new();
    prologue(&mut e);
    e.call_toc(toc(DQ)); // r3 = the queue id (the parameter)
    epilogue(&mut e);
    let mut a = Asm::new();
    prologue(&mut a);
    a.addi(3, 2, Q).call_toc(toc(CQ));
    a.addi(3, 2, 8).lwz(4, Q, 2).li(5, 0).li(6, 0).li(7, 0).li(8, 0).li(9, 0).addi(10, 2, T).call_toc(toc(CT));
    a.lwz(3, Q, 2).addi(4, 2, RES + 4).addi(5, 2, RES + 8).addi(6, 2, RES + 12).lwz(7, FOREVER, 2).call_toc(toc(WQ)).stw(3, RES, 2);
    a.lwz(3, Q, 2).addi(4, 2, RES + 4).addi(5, 2, RES + 8).addi(6, 2, RES + 12).li(7, 0).call_toc(toc(WQ)).stw(3, RES + 16, 2); // the queue is gone
    epilogue(&mut a);
    let mut m = boot(&build(&a, &e.finish()));
    let b = m.cpu().unwrap().gpr[2];
    assert_eq!(m.run_loop(1_000_000).0, Step::Exited, "{:?} {:#?}", m.fault(), m.mac.log);
    assert_eq!(rd(&m, b, RES), -29295, "kMPDeletedErr");
    assert_eq!(rd(&m, b, RES + 16), -29299, "NEGATIVE CONTROL: waiting on the deleted id is kMPInvalidIDErr");
}
