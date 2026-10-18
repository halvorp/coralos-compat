//! A PowerPC application RUNS: retro-ppc executes the fixture's real code, its imports reach the mac68k Toolbox handlers
//! through the glue, the window opens on the GuestUi and the drawing shows up in `present_content` pixels.
use guestui_abi::{CallbackToken, GuestUiEvent, RecordingGuestUi, UiCall};
use macppc_engine::asm::Asm;
use macppc_engine::cfm::Fragment;
use macppc_engine::fixture::{self, PEN, WINDOW_RECT};
use macppc_engine::{MacPpc, Step};
use retro_pef::loader::SymbolClass;
use retro_pef::writer::PefBuilder;
use retro_pef::Pef;
use retro_raster::font::Font;

type M = MacPpc<RecordingGuestUi>;

fn boot(bytes: &[u8]) -> (M, Fragment) {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let frag = m.instantiate("test", &Pef::parse(bytes).unwrap()).unwrap();
    m.start(&frag).unwrap();
    (m, frag)
}

fn win(m: &M) -> u32 { *m.mac.windows.keys().next().expect("the program opened a window") }
fn native(m: &M) -> guestui_abi::UiObjectRef { m.mac.windows[&win(m)].native.unwrap() }
fn black(m: &M, x: i32, y: i32) -> bool { m.mac.qd.canvases[&win(m)].get(x, y) == Some(1) }
/// QuickDraw's baseline is a line between rows: a glyph without descender ends on row baseline-1 (mac68k qd/text.rs).
fn glyph_at(m: &M, ch: u8, x: i32, baseline: i32) -> bool {
    let f = Font::topaz8();
    (0..8).all(|gy| (0..8).all(|gx| black(m, x + gx, baseline - 7 + gy) == f.glyph_bit(f.slot(ch), gx, gy)))
}
fn frames(m: &M) -> usize { m.mac.ui.calls.iter().filter(|c| matches!(c, UiCall::PresentContent { .. } | UiCall::PresentPlane(..))).count() }
fn close(m: &mut M) { let win = native(m); m.mac.ui.events.push_back(GuestUiEvent::CloseRequested { win, token: CallbackToken(0) }); }

#[test]
fn the_application_opens_its_window_draws_its_string_and_waits_for_an_event() {
    let (mut m, _) = boot(&fixture::startup_app());
    let (step, s) = m.step(1_000_000);
    assert_eq!(step, Step::Blocked, "{:#?}\nfault: {:?}", m.mac.log, m.fault());
    assert!(s.insns > 100 && s.insns < 5_000, "a few hundred PowerPC instructions ran: {}", s.insns);
    assert!(m.mac.refused.is_empty(), "{:?}", m.mac.refused);
    // the Toolbox calls, in program order, through the glue
    let log = m.mac.log.join("\n");
    let at = |needle: &str| log.find(needle).unwrap_or_else(|| panic!("{needle} missing:\n{log}"));
    assert!(at("InitGraf") < at("InitWindows") && at("InitWindows") < at("NewWindow") && at("NewWindow") < at("SetPort") && at("SetPort") < at("DrawString"), "{log}");
    // InitGraf got &qd.thePort: the real QDGlobals block was filled in place
    let qd = m.mac.qd_globals - (macppc_engine::machine::QD_GLOBALS_SIZE - 4);
    assert_eq!(m.mac.mem.read_u32(qd + 76).unwrap(), 1, "randSeed (qd + privates[76])");
    assert_ne!(m.mac.mem.read_u32(m.mac.qd_globals).unwrap(), 0, "SetPort stored thePort in qd.thePort");
    // the window opened with its title and the content rectangle
    let opened: Vec<_> = m.mac.ui.calls.iter().filter_map(|c| if let UiCall::OpenWindow(w) = c { Some(w.clone()) } else { None }).collect();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].title, "PPC Hello");
    let [top, left, bottom, right] = WINDOW_RECT.map(i32::from);
    assert_eq!((opened[0].rect.left, opened[0].rect.top, opened[0].rect.width, opened[0].rect.height), (left, top, right - left, bottom - top));
    // the host got a frame, and the text is in it: every glyph of the string at the pen position, pixel for pixel
    assert!(frames(&m) >= 1, "{:?}", m.mac.ui.calls);
    let (h, v) = (i32::from(PEN.0), i32::from(PEN.1));
    for (i, ch) in b"Hello from PowerPC".iter().enumerate() {
        assert!(glyph_at(&m, *ch, h + 8 * i as i32, v), "glyph {:?} at {i}", *ch as char);
    }
    // the host got the string as a TEXT RUN of a display list (replayed with the kit's typography), not as guest pixels
    let plane = &m.mac.ui.planes[&(native(&m).id, 0)];
    let runs: Vec<_> = plane.ops.iter().filter_map(|op| if let guestui_abi::DrawOp::Text { string, at, .. } = op { Some((string.as_str(), *at)) } else { None }).collect();
    assert_eq!(runs.len(), 1, "{:?}", plane.ops);
    assert_eq!(runs[0].0, "Hello from PowerPC");
    assert_eq!(runs[0].1.x, i32::from(PEN.0), "the run starts at the pen");
    assert!(!plane.is_raw() && m.mac.ui.route_census(native(&m)).raw == 0, "nothing went the raw way");
    let bitmap = m.mac.qd.canvases[&win(&m)].pixels();
    assert!(bitmap.iter().any(|&q| q == 1), "control: the engine bitmap (still authoritative) is not blank");
    // NEGATIVE controls: the check is not vacuous (a different glyph at the same place fails), and nothing is drawn
    // outside the string's row
    assert!(!glyph_at(&m, b'Z', h, v), "'H' is not 'Z'");
    assert!(!black(&m, 2, 2) && !black(&m, 200, 70), "the rest of the window is white");
    // a blocked program does not run when nothing arrives, and an event for another window does not wake it
    let before = frames(&m);
    assert_eq!(m.mac.deliver(GuestUiEvent::CloseRequested { win: guestui_abi::UiObjectRef { id: 999, generation: 1 }, token: CallbackToken(0) }), Ok(mac68k_engine::Delivered::NotForAGuestWindow));
    let (step, s) = m.step(10_000);
    assert_eq!((step, s.insns), (Step::Blocked, 0));
    assert_eq!(frames(&m), before);
}

#[test]
fn closing_the_window_ends_the_run_through_findwindow_trackgoaway_sysbeep_exittoshell() {
    let (mut m, _) = boot(&fixture::startup_app());
    assert_eq!(m.step(1_000_000).0, Step::Blocked);
    // control: a mouseDown in the desktop (not the go-away box) is handled and the program keeps waiting
    m.mac.post(1, 0, 400, 400);
    assert_eq!(m.step(100_000).0, Step::Blocked, "{:#?}", m.mac.log);
    assert!(m.mac.log.iter().any(|l| l.contains("FindWindow")), "{:#?}", m.mac.log);
    assert!(!m.mac.log.iter().any(|l| l.contains("SysBeep")));
    // the native close arrives as the go-away click the program handles
    close(&mut m);
    let (step, _) = m.step(100_000);
    assert_eq!(step, Step::Exited, "{:#?}\nfault: {:?}", m.mac.log, m.fault());
    let log = m.mac.log.join("\n");
    assert!(log.contains("part 6"), "FindWindow said inGoAway: {log}");
    assert!(log.contains("SysBeep duration 1 (logged only"), "{log}");
    assert!(log.trim_end().ends_with("ExitToShell"), "{log}");
    assert!(m.mac.refused.is_empty(), "{:?}", m.mac.refused);
    // once exited, stepping again says so
    assert_eq!(m.step(1000).0, Step::Exited);
    assert!(m.cpu().is_none());
}

#[test]
fn the_cfm_entry_state_is_the_tvector_abi() {
    let (m, frag) = boot(&fixture::startup_app());
    let cpu = m.cpu().unwrap();
    let main = frag.main.unwrap();
    assert_eq!((cpu.pc, cpu.gpr[2], cpu.gpr[12]), (main.code, main.toc, main.address), "pc = code, r2 = RTOC, r12 = the TVector");
    assert_eq!(cpu.gpr[1] % 16, 0, "r1 is 16-byte aligned");
    assert_eq!(m.mac.mem.read_u32(cpu.gpr[1]).unwrap(), 0, "the back chain ends");
    assert!(cpu.is_native(cpu.lr), "returning from main lands on the ExitToShell glue");
}

/// A fragment with `code` (main at section-1 offset 0, `extra` second TVector at 8 when given) and a 32-byte data section.
fn tiny(code: &Asm, libs: &[(&str, bool, &[(&str, SymbolClass, bool)])], init_at_8: bool) -> Vec<u8> {
    let total: usize = libs.iter().map(|l| l.2.len()).sum();
    let mut b = PefBuilder::new();
    b.code(&code.finish());
    let data = b.data(&[0u8; 64], 64 + 4 * total as u32);
    for (n, w, s) in libs { b.import_library(n, *w, s); }
    let mut words = vec![if init_at_8 { 0x4601u16 } else { 0x4600 }];
    if total > 0 { words.push(0x4A00 | (total as u16 - 1)); }
    b.reloc(data as u16, &words);
    b.main(data as i32, 0);
    if init_at_8 { b.init(data as i32, 8); }
    b.build()
}

#[test]
fn the_init_routine_runs_before_main_and_both_see_their_toc() {
    // main (code offset 0): data[16] += 1. init (right after it): data[16] = 1. The result is 2 only if init ran first
    // (main first would end with 1). Both address the marker through r2, so each got the TOC of its own TVector.
    let mut main = Asm::new();
    main.lwz(4, 16, 2).addi(4, 4, 1).stw(4, 16, 2).li(3, 0).blr();
    let main_len = main.here();
    let mut both = main;
    both.li(0, 1).stw(0, 16, 2).blr();
    let bytes = tiny(&both, &[], true);
    let run = |with_init: bool| {
        let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
        let frag = m.instantiate("init", &Pef::parse(&bytes).unwrap()).unwrap();
        let (data, code) = (frag.section_bases[1].unwrap(), frag.section_bases[0].unwrap());
        // the PEF builder gives both TVectors code offset 0; point the init TVector at the second routine
        let init = macppc_engine::cfm::Tv { address: data + 8, code: code + main_len, toc: data };
        let frag = Fragment { init: with_init.then_some(init), ..frag };
        m.start(&frag).unwrap();
        assert_eq!(m.step(10_000).0, Step::Exited, "{:?}", m.fault());
        m.mac.mem.read_u32(data + 16).unwrap()
    };
    assert_eq!(run(true), 2, "init (1) then main (+1)");
    assert_eq!(run(false), 1, "negative control: without init, main alone ends at 1");
}

#[test]
fn a_dynamic_import_through_findsymbol_is_callable() {
    // GetSharedLibrary("InterfaceLib") ; FindSymbol(conn, "NewPtr", &addr) ; call the TVector; store the result.
    let imports: &[(&str, SymbolClass, bool)] = &[("GetSharedLibrary", SymbolClass::TVector, false), ("FindSymbol", SymbolClass::TVector, false)];
    let (lib_at, sym_at, mark) = (8 + 4 * 2 + 8, 8 + 4 * 2 + 8 + 13, 8 + 4 * 2);
    let mut a = Asm::new();
    a.mflr(0).stw(0, 8, 1).stwu(1, -64, 1);
    a.addi(3, 2, lib_at).li(4, 0).li(5, 0).addi(6, 1, 32).li(7, 0).li(8, 0).call_toc(8);
    a.stw(3, mark + 4, 2); // GetSharedLibrary's result
    a.lwz(3, 32, 1).addi(4, 2, sym_at).addi(5, 1, 36).li(6, 0).call_toc(12);
    a.stw(3, mark + 8, 2); // FindSymbol's result
    a.lwz(12, 36, 1).lwz(0, 0, 12).stw(2, 20, 1).mtctr(0).lwz(2, 4, 12).li(3, 100).bctrl().lwz(2, 20, 1);
    a.stw(3, mark, 2); // NewPtr(100)'s pointer
    a.addi(1, 1, 64).lwz(0, 8, 1).mtlr(0).blr();
    let mut b = PefBuilder::new();
    b.code(&a.finish());
    let mut data = vec![0u8; 8 + 4 * 2 + 8];
    data.extend_from_slice(b"\x0cInterfaceLib\x06NewPtr");
    let d = b.data(&data, data.len() as u32);
    b.import_library("InterfaceLib", false, imports);
    b.reloc(d as u16, &[0x4600, 0x4A01]);
    b.main(d as i32, 0);
    let (mut m, frag) = boot(&b.build());
    assert_eq!(m.step(100_000).0, Step::Exited, "{:?}\n{:#?}", m.fault(), m.mac.log);
    let base = frag.section_bases[1].unwrap() + mark as u32;
    let rd = |o: u32| m.mac.mem.read_u32(base + o).unwrap();
    assert_eq!((rd(4), rd(8)), (0, 0), "both CFM calls returned noErr");
    let p = rd(0);
    assert_ne!(p, 0, "NewPtr(100) through the TVector FindSymbol returned: the glue cell was bound after the run started");
    assert!(m.mac.mem.write_u8(p + 99, 7).is_ok());
}

#[test]
fn unserved_imports_and_cpu_faults_are_named_faults() {
    // NEXT-NEEDED: calling an import nothing serves stops the run with its name
    let mut a = Asm::new();
    a.mflr(0).stw(0, 8, 1).stwu(1, -64, 1).call_toc(8).blr();
    let (mut m, _) = boot(&tiny(&a, &[("InterfaceLib", false, &[("PlayNoteForever", SymbolClass::TVector, false)])], false));
    let (step, _) = m.step(10_000);
    assert!(matches!(&step, Step::Fault(w) if w.starts_with("NEXT-NEEDED PlayNoteForever")), "{step:?}");
    assert_eq!(m.step(10).0, step, "the fault is sticky");
    // an instruction the interpreter does not run is the CPU's named trap, with the pc
    let mut a = Asm::new();
    a.li(3, 0);
    let mut bytes = a.finish();
    bytes.extend_from_slice(&[0, 0, 0, 0]); // opcode 0: not an instruction
    let mut b = PefBuilder::new();
    b.code(&bytes);
    let d = b.data(&[0; 8], 8);
    b.reloc(d as u16, &[0x4600]);
    b.main(d as i32, 0);
    let (mut m, _) = boot(&b.build());
    let (step, _) = m.step(10_000);
    assert!(matches!(&step, Step::Fault(w) if w.starts_with("PowerPC fault: unimplemented")), "{step:?}");
    // control: the same program that ends in blr instead exits
    let mut a = Asm::new();
    a.li(3, 0).blr();
    let (mut m, _) = boot(&tiny(&a, &[], false));
    assert_eq!(m.step(10_000).0, Step::Exited);
}

#[test]
fn budget_spent_resumes_where_it_stopped() {
    let (mut m, _) = boot(&fixture::startup_app());
    let (step, s) = m.step(10);
    assert_eq!((step, s.insns), (Step::BudgetSpent, 10));
    let (step, _) = m.run_loop(1_000_000);
    assert_eq!(step, Step::Blocked, "{:?}", m.fault());
    assert_eq!(m.mac.windows.len(), 1, "one window, not two: the run resumed instead of restarting");
}


#[test]
fn stdclib_runtime_state_setjmp_and_exit() {
    // __setjmp(&__target_for_exit) ; exit(7): the jmp_buf holds r1, r2 and the caller's LR; _exit_status becomes 7; the run ends
    let libs: &[fixture::Lib] = &[("StdCLib", false, &[("__target_for_exit", SymbolClass::Data, false), ("_exit_status", SymbolClass::Data, false), ("__setjmp", SymbolClass::TVector, false), ("exit", SymbolClass::TVector, false), ("memcpy", SymbolClass::TVector, false)])];
    let mut a = Asm::new();
    a.mflr(0).stw(0, 8, 1).stwu(1, -64, 1);
    a.lwz(3, 8, 2).call_toc(16).stw(3, 40, 1); // r3 = &jmp_buf; __setjmp returns 0
    a.li(3, 7).call_toc(20).blr();
    let bytes = fixture::app_with(libs, &a.finish(), &[]);
    let (mut m, frag) = boot(&bytes);
    let r1 = m.cpu().unwrap().gpr[1];
    assert_eq!(m.step(10_000).0, Step::Exited, "{:?}\n{:#?}", m.fault(), m.mac.log);
    let imp = |n: &str| frag.imports.iter().find(|i| i.symbol == n).unwrap().value;
    assert_eq!(m.mac.mem.read_u32(imp("_exit_status")).unwrap(), 7);
    let buf = imp("__target_for_exit");
    assert_eq!(m.mac.mem.read_u32(buf).unwrap(), r1 - 64, "saved r1 is main's frame");
    assert_eq!(m.mac.mem.read_u32(buf + 4).unwrap(), frag.main.unwrap().toc, "saved r2");
    assert!(m.mac.log.iter().any(|l| l == "[stdclib] exit(7)"), "{:#?}", m.mac.log);
    // negative controls: the two C runtime variables are distinct blocks, and a StdCLib function this engine does not
    // provide is NEXT-NEEDED by name rather than being served by a Toolbox handler of the same name
    assert_ne!(imp("_exit_status"), buf);
    let c = macppc_engine::census::Census::of(&frag);
    assert_eq!(c.next_needed, ["StdCLib/memcpy"]);
    assert!(c.lines.iter().any(|l| l.contains("memcpy") && l.contains("not in the part of StdCLib")), "{:?}", c.lines);
}

#[test]
fn an_init_routine_that_returns_an_error_stops_the_fragment_before_main() {
    // init returns OSErr -5 (r3 = -5): main (data[16] += 1) must NOT run. The positive control is the test above (init returns 0).
    let mut main = Asm::new();
    main.lwz(4, 16, 2).addi(4, 4, 1).stw(4, 16, 2).li(3, 0).blr();
    let main_len = main.here();
    let mut both = main;
    both.li(3, -5).blr();
    let bytes = tiny(&both, &[], true);
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let frag = m.instantiate("badinit", &Pef::parse(&bytes).unwrap()).unwrap();
    let (data, code) = (frag.section_bases[1].unwrap(), frag.section_bases[0].unwrap());
    let init = macppc_engine::cfm::Tv { address: data + 8, code: code + main_len, toc: data };
    m.start(&Fragment { init: Some(init), ..frag }).unwrap();
    let (step, _) = m.step(10_000);
    assert!(matches!(&step, Step::Fault(w) if w.contains("OSErr -5")), "{step:?}");
    assert_eq!(m.mac.mem.read_u32(data + 16).unwrap(), 0, "main did not run");
    assert!(matches!(m.step(10).0, Step::Fault(_)), "the fault is sticky");
}

/// `<call>(everyEvent, &event, 60, nil)` once with an empty event queue, storing the Boolean result in the tail word
/// (preset to 0xFFFFFFFF), then returning from main.
fn poll_once(call: &str) -> (Step, u32) {
    let libs: &[fixture::Lib] = &[("InterfaceLib", false, &[(call, SymbolClass::TVector, false)])];
    let mut a = Asm::new();
    a.mflr(0).stw(0, 8, 1).stwu(1, -96, 1);
    a.li(3, -1).addi(4, 1, 30).li(5, 60).li(6, 0).call_toc(fixture::TOC_FIRST as i32).stw(3, 12, 2);
    a.addi(1, 1, 96).lwz(0, 8, 1).mtlr(0).blr();
    let bytes = fixture::app_with(libs, &a.finish(), &[0xFF; 4]);
    let (mut m, frag) = boot(&bytes);
    let data = frag.main.unwrap().toc;
    let step = m.step(10_000).0;
    (step, m.mac.mem.read_u32(data + 12).unwrap())
}

#[test]
fn getnextevent_never_waits_but_waitnextevent_with_a_sleep_does() {
    // Inside Macintosh: GetNextEvent returns FALSE with a null event at once; the program goes on and finishes.
    assert_eq!(poll_once("GetNextEvent"), (Step::Exited, 0));
    // control: WaitNextEvent with sleep 60 and nothing queued parks the CPU on the call (the result word is untouched)
    assert_eq!(poll_once("WaitNextEvent"), (Step::Blocked, 0xFFFF_FFFF));
}

#[test]
fn an_event_code_beyond_the_mask_width_matches_no_mask_instead_of_overflowing() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    m.mac.post(20, 0, 0, 0);
    assert!(!m.mac.event_pending(0xFFFF));
    m.mac.post(1, 0, 0, 0);
    assert!(m.mac.event_pending(0x0002), "mouseDown (1) is accepted by its bit");
    assert!(!m.mac.event_pending(0x0001), "control: not by another bit");
}

#[test]
fn stdclib_data_has_the_real_layout_and_bindings_are_per_library() {
    use macppc_engine::symbols::Binding;
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    // _IntEnv is the 14-byte packed struct (argc/argv/envp at +2/+6/+10): its block must not run into __C_phase
    let env = m.bind_from("StdCLib", "_IntEnv");
    let phase = m.bind_from("StdCLib", "__C_phase");
    assert!(matches!(env, Binding::CData(n) if n >= 14), "{env:?}");
    let (e, p) = (m.data_block("_IntEnv", &env).unwrap(), m.data_block("__C_phase", &phase).unwrap());
    assert!(p >= e + 14 || e >= p + 4, "_IntEnv {e:#x} (14 bytes) overlaps __C_phase {p:#x}");
    for off in [2, 6, 10] { assert_eq!(m.mac.mem.read_u32(e + off).unwrap(), 0, "argc/argv/envp start as zeros (+{off})"); }
    assert!(matches!(m.bind_from("StdCLib", "__target_for_exit"), Binding::CData(256)), "a 64-word jump buffer");
    assert!(matches!(m.bind_from("StdCLib", "__NubAt3"), Binding::CData(4)), "one function-pointer word");
    // StdCLib's `exit` does not leak into InterfaceLib (and so into any other library) through a shared by-name table
    assert!(matches!(m.bind_from("StdCLib", "exit"), Binding::Native(_)));
    assert!(matches!(m.bind_from("InterfaceLib", "exit"), Binding::Unbound(_)), "InterfaceLib exports no `exit`");
    assert!(matches!(m.bindings.get("exit"), None | Some(Binding::Unbound(_))), "the by-name entry never holds StdCLib's");
    assert!(matches!(m.bindings.get("StdCLib::exit"), Some(Binding::Native(_))));
}
