//! Thread Manager hooks: SetThreadTerminator, SetThreadSwitcher (in and out) run as the program's own PowerPC routines on the right thread in the right order, and
//! ThreadCurrentStackSpace answers from the thread's real stack.
use guestui_abi::RecordingGuestUi;
use macppc_engine::asm::Asm;
use macppc_engine::{MacPpc, Step};
use retro_pef::loader::SymbolClass;
use retro_pef::writer::PefBuilder;
use retro_pef::Pef;

type M = MacPpc<RecordingGuestUi>;
const F: SymbolClass = SymbolClass::TVector;
const N: &[&str] = &["NewThread", "YieldToAnyThread", "SetThreadTerminator", "SetThreadSwitcher", "GetCurrentThread", "ThreadCurrentStackSpace"];
const NEW: i32 = 0; const YIELD: i32 = 1; const TERM: i32 = 2; const SWITCH: i32 = 3; const GETCUR: i32 = 4; const SPACE: i32 = 5;
const TOC0: i32 = 24; // main TVector [0,8), the thread entry [8,16), the hook [16,24)
const D: i32 = TOC0 + 4 * 6;
const PTR: i32 = D; const TID: i32 = D + 4; const ME: i32 = D + 8; const FREE: i32 = D + 12; const LOG: i32 = D + 32;

fn toc(i: i32) -> i32 { TOC0 + 4 * i }
fn build(main: &Asm, entry: &Asm, hook: &Asm) -> Vec<u8> {
    let mut code = main.finish();
    let e_at = code.len() as u32; code.extend(entry.finish());
    let h_at = code.len() as u32; code.extend(hook.finish());
    let mut b = PefBuilder::new();
    b.code(&code);
    let mut init = vec![0u8; D as usize + 256];
    init[8..12].copy_from_slice(&e_at.to_be_bytes());
    init[16..20].copy_from_slice(&h_at.to_be_bytes());
    let d = b.data(&init, init.len() as u32);
    let imports: Vec<(&str, SymbolClass, bool)> = N.iter().map(|n| (*n, F, false)).collect();
    b.import_library("ThreadsLib", false, &imports);
    b.reloc(d as u16, &[0x4602, 0x4A00 | 5]);
    b.main(d as i32, 0);
    b.build()
}
fn rd(m: &M, b: u32, off: i32) -> i32 { m.mac.mem.read_u32(b + off as u32).unwrap() as i32 }

#[test]
fn terminator_and_switchers_run_on_the_thread_in_order_and_stack_space_is_real() {
    // hook(id, param): append (id, param) to the log at *PTR
    let mut hook = Asm::new();
    hook.lwz(9, PTR, 2).stw(3, 0, 9).stw(4, 4, 9).addi(9, 9, 8).stw(9, PTR, 2).blr();
    // thread entry: records its own stack space, returns
    let mut entry = Asm::new();
    entry.mflr(0).stw(0, 8, 1).stwu(1, -64, 1);
    entry.addi(3, 2, ME).call_toc(toc(GETCUR));
    entry.lwz(3, ME, 2).addi(4, 2, FREE).call_toc(toc(SPACE));
    entry.li(3, 0x4D);
    entry.addi(1, 1, 64).lwz(0, 8, 1).mtlr(0).blr();
    let mut main = Asm::new();
    main.mflr(0).stw(0, 8, 1).stwu(1, -64, 1);
    main.addi(9, 2, LOG).stw(9, PTR, 2);                                                                   // the log pointer
    main.li(3, 1).addi(4, 2, 8).li(5, 0).li(6, 0x4000).li(7, 0).li(8, 0).addi(9, 2, TID).call_toc(toc(NEW)); // 16 KiB stack
    main.lwz(3, TID, 2).addi(4, 2, 16).li(5, 0x99).call_toc(toc(TERM));
    main.lwz(3, TID, 2).addi(4, 2, 16).li(5, 0x77).li(6, 1).call_toc(toc(SWITCH));                          // in
    main.lwz(3, TID, 2).addi(4, 2, 16).li(5, 0x66).li(6, 0).call_toc(toc(SWITCH));                          // out
    main.call_toc(toc(YIELD));
    main.addi(1, 1, 64).lwz(0, 8, 1).mtlr(0).blr();
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let frag = m.instantiate("t", &Pef::parse(&build(&main, &entry, &hook)).unwrap()).unwrap();
    m.start(&frag).unwrap();
    let b = m.cpu().unwrap().gpr[2];
    assert_eq!(m.run_loop(1_000_000).0, Step::Exited, "{:?} {:#?}", m.fault(), m.mac.log);
    let tid = rd(&m, b, TID);
    assert_eq!(tid, 2);
    let log: Vec<(i32, i32)> = (0..3).map(|i| (rd(&m, b, LOG + 8 * i), rd(&m, b, LOG + 8 * i + 4))).collect();
    assert_eq!(log, [(tid, 0x77), (tid, 0x99), (tid, 0x66)], "switched IN, then its terminator at the end, then switched OUT -- all with the thread's id and each hook's own parameter");
    let free = rd(&m, b, FREE);
    assert!(free > 0 && free < 0x4000, "stack space left inside a 16 KiB stack: {free}");
    assert_eq!(rd(&m, b, LOG + 24), 0, "exactly three hook calls");
    assert_eq!(m.callback_depth(), 0);
}
