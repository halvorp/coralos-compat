//! The Apple Event Manager for PowerPC programs: a handler installed for 'aevt'/'oapp' is sent the Finder's "open application" event, which the program fetches with
//! WaitNextEvent (a high-level event) and hands to AEProcessAppleEvent; the handler is the program's own PowerPC routine.
use guestui_abi::RecordingGuestUi;
use macppc_engine::asm::Asm;
use macppc_engine::{MacPpc, Step};
use retro_pef::loader::SymbolClass;
use retro_pef::writer::PefBuilder;
use retro_pef::Pef;

type M = MacPpc<RecordingGuestUi>;
const F: SymbolClass = SymbolClass::TVector;
const N: &[&str] = &["AEInstallEventHandler", "WaitNextEvent", "AEProcessAppleEvent", "AEGetParamDesc", "AECountItems", "AEDisposeDesc", "AERemoveEventHandler"];
const INST: i32 = 0; const WNE: i32 = 1; const PROC: i32 = 2; const GETP: i32 = 3; const COUNT: i32 = 4; const DISPOSE: i32 = 5; const REMOVE: i32 = 6;
const TOC0: i32 = 16; // main TVector [0,8), the handler [8,16)
const D: i32 = TOC0 + 4 * 7;
const REC: i32 = D; const OK: i32 = D + 32;
fn toc(i: i32) -> i32 { TOC0 + 4 * i }
fn fc(b: &[u8; 4]) -> i32 { i32::from_be_bytes(*b) }

fn build(main: &Asm, handler: &Asm) -> Vec<u8> {
    let mut code = main.finish();
    let h_at = code.len() as u32; code.extend(handler.finish());
    let mut b = PefBuilder::new();
    b.code(&code);
    let mut init = vec![0u8; D as usize + 128];
    init[8..12].copy_from_slice(&h_at.to_be_bytes());
    // 'aevt' 'oapp' 'quit' as data words at D+64..
    for (i, w) in [fc(b"aevt"), fc(b"oapp"), fc(b"quit")].into_iter().enumerate() { init[D as usize + 64 + 4 * i..D as usize + 68 + 4 * i].copy_from_slice(&w.to_be_bytes()); }
    let d = b.data(&init, init.len() as u32);
    let imports: Vec<(&str, SymbolClass, bool)> = N.iter().map(|n| (*n, F, false)).collect();
    b.import_library("InterfaceLib", false, &imports);
    b.reloc(d as u16, &[0x4601, 0x4A00 | 6]);
    b.main(d as i32, 0);
    b.build()
}
fn rd(m: &M, b: u32, off: i32) -> i32 { m.mac.mem.read_u32(b + off as u32).unwrap() as i32 }

#[test]
fn the_open_application_event_reaches_the_programs_powerpc_handler_through_aeprocessappleevent() {
    // handler(event, reply, refcon): record refcon and the event's type, return noErr; a second handler result is checked via OK+8
    let mut h = Asm::new();
    h.stw(5, OK, 2).lwz(9, 0, 3).stw(9, OK + 4, 2).li(3, 0).blr();
    let mut a = Asm::new();
    a.mflr(0).stw(0, 8, 1).stwu(1, -96, 1);
    a.lwz(3, D + 64, 2).lwz(4, D + 68, 2).addi(5, 2, 8).li(6, 0x55).li(7, 0).call_toc(toc(INST)).stw(3, OK + 8, 2);   // AEInstallEventHandler('aevt','oapp',handler,0x55,false)
    a.li(3, 0x0400).addi(4, 2, REC).li(5, 0).li(6, 0).call_toc(toc(WNE)).stw(3, OK + 12, 2);                            // WaitNextEvent(highLevelEventMask, &rec, 0, 0)
    a.addi(3, 2, REC).call_toc(toc(PROC)).stw(3, OK + 16, 2);                                                           // AEProcessAppleEvent(&rec)
    a.li(3, 0).stw(3, REC + 12, 2);                                                                                      // an event nobody handles: id 'quit'
    a.lwz(9, D + 72, 2).stw(9, REC + 10, 2);                                                                             // (where := 'quit': the id lives in the 4 bytes at +10)
    a.addi(3, 2, REC).call_toc(toc(PROC)).stw(3, OK + 20, 2);
    a.addi(3, 2, OK + 40).li(4, 0).call_toc(toc(COUNT)).stw(3, OK + 24, 2);                                              // AECountItems on a non-list
    a.lwz(3, D + 64, 2).lwz(4, D + 68, 2).li(5, 0).call_toc(toc(REMOVE)).stw(3, OK + 28, 2);                            // AERemoveEventHandler
    a.lwz(3, D + 64, 2).lwz(4, D + 68, 2).li(5, 0).call_toc(toc(REMOVE)).stw(3, OK + 32, 2);                            // ... twice: not found
    a.addi(1, 1, 96).lwz(0, 8, 1).mtlr(0).blr();
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let frag = m.instantiate("t", &Pef::parse(&build(&a, &h)).unwrap()).unwrap();
    m.start(&frag).unwrap();
    let b = m.cpu().unwrap().gpr[2];
    let (step, _) = m.run_loop(1_000_000);
    assert_eq!(step, Step::Exited, "{:?} {:#?}", m.fault(), m.mac.log);
    assert_eq!(rd(&m, b, OK + 8), 0, "AEInstallEventHandler");
    assert_eq!(rd(&m, b, OK + 12) & 0xFF, 1, "WaitNextEvent returned an event");
    assert_eq!(m.mac.mem.read_u16(b + REC as u32).unwrap(), 23, "a high-level event (kHighLevelEvent)");
    assert_eq!(rd(&m, b, OK), 0x55, "the handler ran as PowerPC code with the refcon it was installed with");
    assert_eq!(rd(&m, b, OK + 4) as u32, u32::from_be_bytes(*b"aevt"), "its first argument was an 'aevt' event descriptor");
    assert_eq!(rd(&m, b, OK + 16), 0, "AEProcessAppleEvent returned the handler's OSErr");
    assert_eq!(rd(&m, b, OK + 20), -1708, "NEGATIVE CONTROL: an event with no handler is errAEEventNotHandled");
    assert_eq!(rd(&m, b, OK + 24), -1704, "AECountItems of a non-list");
    assert_eq!((rd(&m, b, OK + 28), rd(&m, b, OK + 32)), (0, -1717), "remove once ok, twice errAEHandlerNotFound");
    assert_eq!(m.callback_depth(), 0);
}
