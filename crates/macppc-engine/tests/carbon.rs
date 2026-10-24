//! The Carbon Event Manager core and Carbon window calls, with CoreFoundation strings, driven by interpreted PowerPC: a main that opens a window,
//! installs handlers and runs the event loop; handlers that are PowerPC routines called back by the dispatcher. Events are injected into the classic
//! queue (what the GuestUi pump fills), so the conversion path is the real one.
use guestui_abi::RecordingGuestUi;
use macppc_engine::asm::Asm;
use macppc_engine::{MacPpc, Step};
use mac68k_engine::events::MacEvent;
use retro_pef::loader::SymbolClass;
use retro_pef::writer::PefBuilder;
use retro_pef::Pef;

type M = MacPpc<RecordingGuestUi>;
const F: SymbolClass = SymbolClass::TVector;
const N: &[&str] = &["CreateNewWindow", "__CFStringMakeConstantString", "SetWindowTitleWithCFString", "ShowWindow", "GetApplicationEventTarget", "InstallEventHandler",
    "RunApplicationEventLoop", "QuitApplicationEventLoop", "GetEventClass", "GetEventKind", "GetEventParameter", "ReceiveNextEvent", "SendEventToEventTarget",
    "GetEventDispatcherTarget", "ReleaseEvent", "CallNextEventHandler", "CFStringCreateWithCString", "CFStringGetCString", "CFStringGetLength", "CFRelease", "CFRetain"];
const CNW: i32 = 0; const CFSTR: i32 = 1; const SWT: i32 = 2; const SHOW: i32 = 3; const APPT: i32 = 4; const INST: i32 = 5; const RUN: i32 = 6; const QUIT: i32 = 7;
const GCLASS: i32 = 8; const GKIND: i32 = 9; const GPARAM: i32 = 10; const RECV: i32 = 11; const SEND: i32 = 12; const DISPT: i32 = 13; const RELEASE: i32 = 14; const NEXT: i32 = 15;
const CFNEW: i32 = 16; const CFGET: i32 = 17; const CFLEN: i32 = 18; const CFREL: i32 = 19; const CFRET: i32 = 20;
const TOC0: i32 = 24; // three TVectors: main [0,8), handler 1 [8,16), handler 2 [16,24)
const D: i32 = TOC0 + 4 * 21;
// data (word offsets from D): bytes at D..: rect (8), spec (8), title cstring (16), then results
const RECT: i32 = D; const SPEC: i32 = D + 8; const TITLE: i32 = D + 16; const ATTRS: i32 = D + 40; const WIN: i32 = D + 44; const REF: i32 = D + 48; const CFS: i32 = D + 52;
const R: i32 = D + 64; // results R + 4*i
const K: i32 = D + 256; // constants: 'mloc', 'QDPt', 'zzzz', 'magn'
const EVSAVE: i32 = R + 60;

fn toc(i: i32) -> i32 { TOC0 + 4 * i }
fn prologue(a: &mut Asm) { a.mflr(0).stw(0, 8, 1).stwu(1, -96, 1); }
fn epilogue(a: &mut Asm) { a.addi(1, 1, 96).lwz(0, 8, 1).mtlr(0).blr(); }
fn words(w: &[u32]) -> Vec<u8> { w.iter().flat_map(|x| x.to_be_bytes()).collect() }
fn fc(b: &[u8; 4]) -> u32 { u32::from_be_bytes(*b) }

fn build(main: &Asm, h1: &Asm, h2: &Asm, data: &[u8]) -> Vec<u8> {
    let mut code = main.finish();
    let h1_at = code.len() as u32;
    code.extend(h1.finish());
    let h2_at = code.len() as u32;
    code.extend(h2.finish());
    let mut b = PefBuilder::new();
    b.code(&code);
    let mut init = vec![0u8; D as usize];
    init[8..12].copy_from_slice(&h1_at.to_be_bytes());
    init[16..20].copy_from_slice(&h2_at.to_be_bytes());
    init.extend_from_slice(data);
    init.resize(init.len().max(D as usize + 512), 0);
    let d = b.data(&init, init.len() as u32);
    let imports: Vec<(&str, SymbolClass, bool)> = N.iter().map(|n| (*n, F, false)).collect();
    b.import_library("CarbonLib", false, &imports);
    b.reloc(d as u16, &[0x4602, 0x4A00 | 20]);
    b.main(d as i32, 0);
    b.build()
}
/// Initial data: the window rect (top 50, left 40, bottom 150, right 240), one EventTypeSpec, the title, `extra` from R.
fn data(spec: (u32, u32), attrs: u32) -> Vec<u8> {
    let mut v = Vec::new();
    for x in [50i16, 40, 150, 240] { v.extend(x.to_be_bytes()); }
    v.extend(words(&[spec.0, spec.1]));
    let mut t = b"Carbon Test".to_vec(); t.resize(24, 0);
    v.extend(t);
    v.extend(words(&[attrs]));
    v.resize((K - D) as usize, 0);
    v.extend(words(&[fc(b"mloc"), fc(b"QDPt"), fc(b"zzzz"), fc(b"magn")]));
    v
}
fn boot(bytes: &[u8]) -> M {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let frag = m.instantiate("test", &Pef::parse(bytes).unwrap()).unwrap();
    m.start(&frag).unwrap();
    m
}
fn rd(m: &M, b: u32, off: i32) -> i32 { m.mac.mem.read_u32(b + off as u32).unwrap_or(0xDEAD) as i32 }
fn ev(what: u16, v: i16, h: i16, message: u32) -> MacEvent { MacEvent { what, message, when: 0, v, h, modifiers: 0 } }

/// main: window (document, close+zoom+resizable, standard handler), title via CFSTR, show, a handler on `spec`, RunApplicationEventLoop.
fn window_main(handler_tv_at: i32) -> Asm {
    let mut a = Asm::new();
    prologue(&mut a);
    a.li(3, 6).lwz(4, ATTRS, 2).addi(5, 2, RECT).addi(6, 2, WIN);
    a.call_toc(toc(CNW));
    a.stw(3, R, 2);
    a.addi(3, 2, TITLE).call_toc(toc(CFSTR)).stw(3, CFS, 2);
    a.lwz(3, WIN, 2).lwz(4, CFS, 2).call_toc(toc(SWT));
    a.lwz(3, WIN, 2).call_toc(toc(SHOW));
    a.call_toc(toc(APPT));
    a.addi(4, 2, handler_tv_at).li(5, 1).addi(6, 2, SPEC).li(7, 0x77).addi(8, 2, REF).call_toc(toc(INST));
    a.stw(3, R + 4, 2);
    a.call_toc(toc(RUN));
    a.li(9, 99).stw(9, R + 8, 2);
    epilogue(&mut a);
    a
}

/// A handler that records what it was given and asks the loop to end; returns `result`.
fn recording_handler(result: i32) -> Asm {
    let mut h = Asm::new();
    prologue(&mut h);
    h.stw(5, R + 20, 2).stw(4, EVSAVE, 2);
    h.lwz(3, EVSAVE, 2).call_toc(toc(GCLASS)).stw(3, R + 12, 2);
    h.lwz(3, EVSAVE, 2).call_toc(toc(GKIND)).stw(3, R + 16, 2);
    h.lwz(3, EVSAVE, 2).lwz(4, K, 2).lwz(5, K + 4, 2).li(6, 0).li(7, 4).li(8, 0).addi(9, 2, R + 24).call_toc(toc(GPARAM)).stw(3, R + 28, 2);   // 'mloc' as QDPt
    h.lwz(3, EVSAVE, 2).lwz(4, K + 8, 2).li(5, 0).li(6, 0).li(7, 4).li(8, 0).addi(9, 2, R + 40).call_toc(toc(GPARAM)).stw(3, R + 32, 2);        // 'zzzz': not found
    h.lwz(3, EVSAVE, 2).lwz(4, K, 2).lwz(5, K + 12, 2).li(6, 0).li(7, 4).li(8, 0).addi(9, 2, R + 40).call_toc(toc(GPARAM)).stw(3, R + 36, 2);  // 'mloc' as 'magn': no coercion
    h.call_toc(toc(QUIT));
    h.li(3, result);
    epilogue(&mut h);
    h
}

fn run(m: &mut M) -> Step { m.run_loop(2_000_000).0 }

#[test]
fn a_carbon_program_opens_a_titled_window_and_its_handler_gets_the_mouse_event_with_its_parameters() {
    let attrs = 1 | 3 | (1 << 4) | (1 << 25);
    let bytes = build(&window_main(8), &recording_handler(0), &Asm::new(), &data((fc(b"mous"), 1), attrs));
    let mut m = boot(&bytes);
    let b = m.cpu().unwrap().gpr[2];
    m.mac.ev.queue.push_back(ev(1, 100, 100, 0)); // a mouse-down in the window's content
    let step = run(&mut m);
    assert_eq!(step, Step::Exited, "{:?} {:#?}", m.fault(), m.mac.log);
    assert_eq!(rd(&m, b, R), 0, "CreateNewWindow");
    let win = rd(&m, b, WIN) as u32;
    assert_ne!(win, 0);
    assert_eq!(rd(&m, b, R + 4), 0, "InstallEventHandler");
    let w = &m.mac.windows[&win];
    assert_eq!((w.title.as_str(), w.visible, w.proc_id, w.go_away), ("Carbon Test", true, 8, true), "document window with zoom+grow, close box, titled through a CFString, shown");
    assert_eq!((rd(&m, b, R + 12) as u32, rd(&m, b, R + 16), rd(&m, b, R + 20)), (fc(b"mous"), 1, 0x77), "class, kind and the userData the handler was installed with");
    assert_eq!(rd(&m, b, R + 24) as u32, (100 << 16) | 100, "kEventParamMouseLocation as a QDPoint (v, h)");
    assert_eq!((rd(&m, b, R + 28), rd(&m, b, R + 32), rd(&m, b, R + 36)), (0, -9870, -1700), "found; eventParameterNotFoundErr for a name not there; no coercion between types");
    assert_eq!(rd(&m, b, R + 8), 99, "QuitApplicationEventLoop made RunApplicationEventLoop return");
    assert_eq!(m.callback_depth(), 0);
    // NEGATIVE CONTROL: a click outside every window is not delivered to a content handler (nothing is called, the loop keeps waiting)
    let mut m = boot(&bytes);
    m.mac.ev.queue.push_back(ev(1, 400, 600, 0));
    assert_eq!(run(&mut m), Step::Blocked, "{:?}", m.fault());
    assert_eq!(rd(&m, b, R + 12), 0);
}

#[test]
fn the_close_box_is_a_window_close_event_and_the_standard_handler_disposes_the_window_only_if_it_is_installed() {
    for (attrs, disposed) in [(1u32 | 3 | (1 << 4) | (1 << 25), true), (1 | 3 | (1 << 4), false)] {
        // the app handler sees kEventWindowClose, asks the loop to end and passes it on (eventNotHandledErr)
        let bytes = build(&window_main(8), &recording_handler(-9874), &Asm::new(), &data((fc(b"wind"), 72), attrs));
        let mut m = boot(&bytes);
        let b = m.cpu().unwrap().gpr[2];
        // inGoAway: the close box is in the title bar, a few pixels in from the window's left edge
        m.mac.ev.queue.push_back(ev(1, 42, 50, 0));
        assert_eq!(run(&mut m), Step::Exited, "{:?} {:#?}", m.fault(), m.mac.log);
        let win = rd(&m, b, WIN) as u32;
        assert_eq!((rd(&m, b, R + 12) as u32, rd(&m, b, R + 16)), (fc(b"wind"), 72), "the close box click arrived as kEventWindowClose, not as a mouse event");
        assert_eq!(m.mac.windows.contains_key(&win), !disposed, "standard handler installed = {disposed}: the window {}", if disposed { "is disposed" } else { "stays: no default exists without it" });
    }
}

#[test]
fn corefoundation_strings_create_measure_copy_retain_and_release() {
    let mut a = Asm::new();
    prologue(&mut a);
    a.li(3, 0).addi(4, 2, TITLE).li(5, 0).call_toc(toc(CFNEW)).stw(3, CFS, 2);
    a.lwz(3, CFS, 2).call_toc(toc(CFLEN)).stw(3, R, 2);
    a.lwz(3, CFS, 2).addi(4, 2, R + 80).li(5, 32).li(6, 0).call_toc(toc(CFGET)).stw(3, R + 4, 2);       // fits
    a.lwz(3, CFS, 2).addi(4, 2, R + 120).li(5, 5).li(6, 0).call_toc(toc(CFGET)).stw(3, R + 8, 2);       // 11 characters + NUL do not fit in 5
    a.addi(3, 2, TITLE).call_toc(toc(CFSTR)).stw(3, R + 12, 2);
    a.addi(3, 2, TITLE).call_toc(toc(CFSTR)).stw(3, R + 16, 2);                                          // CFSTR: equal constants are one object
    a.lwz(3, CFS, 2).call_toc(toc(CFRET));                                                               // retain 2
    a.lwz(3, CFS, 2).call_toc(toc(CFREL));                                                               // release 1
    a.lwz(3, CFS, 2).call_toc(toc(CFLEN)).stw(3, R + 20, 2);                                             // still alive
    a.lwz(3, CFS, 2).call_toc(toc(CFREL));                                                               // release 0: gone
    a.lwz(3, R + 12, 2).call_toc(toc(CFREL));                                                            // a constant is never freed
    a.lwz(3, R + 12, 2).call_toc(toc(CFLEN)).stw(3, R + 24, 2);
    epilogue(&mut a);
    let bytes = build(&a, &Asm::new(), &Asm::new(), &data((0, 0), 0));
    let mut m = boot(&bytes);
    let b = m.cpu().unwrap().gpr[2];
    assert_eq!(run(&mut m), Step::Exited, "{:?} {:#?}", m.fault(), m.mac.log);
    assert_eq!((rd(&m, b, R), rd(&m, b, R + 4), rd(&m, b, R + 8)), (11, 1, 0));
    assert_eq!(m.get_bytes(b + (R + 80) as u32, 12, "t").unwrap(), b"Carbon Test\0");
    assert_eq!(rd(&m, b, R + 12), rd(&m, b, R + 16));
    assert_eq!((rd(&m, b, R + 20), rd(&m, b, R + 24)), (11, 11));
    // NEGATIVE CONTROL: using the freed string stops the run naming the call
    let mut a = Asm::new();
    prologue(&mut a);
    a.li(3, 0).addi(4, 2, TITLE).li(5, 0).call_toc(toc(CFNEW)).stw(3, CFS, 2);
    a.lwz(3, CFS, 2).call_toc(toc(CFREL));
    a.lwz(3, CFS, 2).call_toc(toc(CFLEN));
    epilogue(&mut a);
    let mut m = boot(&build(&a, &Asm::new(), &Asm::new(), &data((0, 0), 0)));
    let (step, _) = m.run_loop(100_000);
    assert!(matches!(&step, Step::Fault(w) if w.starts_with("CFStringGetLength:") && w.contains("is not a CFString")), "{step:?}");
}

fn dispatch_main(h_next: bool) -> (Asm, Asm, Asm) {
    let mut a = Asm::new();
    prologue(&mut a);
    a.li(3, 6).lwz(4, ATTRS, 2).addi(5, 2, RECT).addi(6, 2, WIN).call_toc(toc(CNW));
    a.lwz(3, WIN, 2).call_toc(toc(SHOW));
    a.call_toc(toc(APPT));
    a.addi(4, 2, 8).li(5, 1).addi(6, 2, SPEC).li(7, 0x11).addi(8, 2, REF).call_toc(toc(INST));
    a.call_toc(toc(APPT));
    a.addi(4, 2, 16).li(5, 1).addi(6, 2, SPEC).li(7, 0x22).addi(8, 2, REF).call_toc(toc(INST));
    a.li(3, 0).li(4, 0).li(7, 1).addi(8, 2, R + 40).call_toc(toc(RECV)).stw(3, R, 2);            // ReceiveNextEvent(0, NULL, 0.0, pull, &event)
    a.call_toc(toc(DISPT)).mr(4, 3).lwz(3, R + 40, 2).call_toc(toc(SEND)).stw(3, R + 4, 2);       // SendEventToEventTarget(event, dispatcher)
    a.lwz(3, R + 40, 2).call_toc(toc(RELEASE));
    // drain what showing the window queued (activate/update events), counting them, until ReceiveNextEvent times out
    a.label("drain");
    a.li(3, 0).li(4, 0).li(7, 1).addi(8, 2, R + 44).call_toc(toc(RECV));
    a.cmpwi(3, 0).bne("drained");
    a.lwz(3, R + 44, 2).call_toc(toc(RELEASE));
    a.lwz(9, R + 48, 2).addi(9, 9, 1).stw(9, R + 48, 2);
    a.b("drain");
    a.label("drained");
    a.stw(3, R + 8, 2);
    epilogue(&mut a);
    let mut h1 = Asm::new();
    prologue(&mut h1);
    h1.stw(5, R + 20, 2).li(9, 1).stw(9, R + 28, 2);
    if h_next { h1.call_toc(toc(NEXT)).stw(3, R + 12, 2); } else { h1.li(3, 0); }
    epilogue(&mut h1);
    let mut h2 = Asm::new();
    prologue(&mut h2);
    h2.stw(5, R + 24, 2).lwz(9, R + 28, 2).stw(9, R + 32, 2).li(3, 0);
    epilogue(&mut h2);
    (a, h1, h2)
}

#[test]
fn receivenextevent_and_sendeventtoeventtarget_walk_the_handler_chain_and_callnexteventhandler_continues_it() {
    let attrs = 1 | 3 | (1 << 4);
    let (a, h1, h2) = dispatch_main(true);
    let mut m = boot(&build(&a, &h1, &h2, &data((fc(b"mous"), 1), attrs)));
    let b = m.cpu().unwrap().gpr[2];
    m.mac.ev.queue.push_back(ev(1, 100, 100, 0));
    assert_eq!(run(&mut m), Step::Exited, "{:?} {:#?}", m.fault(), m.mac.log);
    assert_eq!((rd(&m, b, R), rd(&m, b, R + 4), rd(&m, b, R + 8)), (0, 0, -9875), "event received; handled; then eventLoopTimedOutErr with the queue empty");
    assert!(rd(&m, b, R + 48) >= 1, "showing the window queued update/activate events, which ReceiveNextEvent returned before it timed out");
    assert_eq!((rd(&m, b, R + 20), rd(&m, b, R + 24)), (0x11, 0x22), "both handlers ran, each with its own userData");
    assert_eq!((rd(&m, b, R + 12), rd(&m, b, R + 32)), (0, 1), "the first handler's CallNextEventHandler returned the second's result, and the first ran first");
    assert_eq!(m.callback_depth(), 0);
    // NEGATIVE CONTROL: a handler that returns noErr WITHOUT calling the next ends the chain: the second never runs
    let (a, h1, h2) = dispatch_main(false);
    let mut m = boot(&build(&a, &h1, &h2, &data((fc(b"mous"), 1), attrs)));
    let b = m.cpu().unwrap().gpr[2];
    m.mac.ev.queue.push_back(ev(1, 100, 100, 0));
    assert_eq!(run(&mut m), Step::Exited, "{:?}", m.fault());
    assert_eq!((rd(&m, b, R + 20), rd(&m, b, R + 24)), (0x11, 0));
}
