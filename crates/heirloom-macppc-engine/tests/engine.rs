//! CFM instantiation, import binding and calls through glue, each with a negative control. No PowerPC CPU is needed: a
//! test plays the CPU's part exactly as the contract in `host.rs` describes (read the TOC word, read the TVector, reach
//! the stub, call `glue_call`).
use heirloom_ui_abi::{RecordingGuestUi, UiCall};
use heirloom_macppc_engine::cfm::{Fragment, ImportStatus};
use heirloom_macppc_engine::census::Census;
use heirloom_macppc_engine::fixture::{self, TOC_FIRST};
use heirloom_macppc_engine::{CfmError, GlueOutcome, MacPpc, PpcHost};
use heirloom_mac68k_engine::fixture::HELLO;
use heirloom_mac68k_engine::Mac;
use heirloom_fmt_pef::loader::SymbolClass;
use heirloom_fmt_pef::{Pef, PefError};

type M = MacPpc<RecordingGuestUi>;

fn machine() -> M { MacPpc::new(RecordingGuestUi::new()).unwrap() }

fn load(m: &mut M, bytes: &[u8]) -> Fragment {
    let pef = Pef::parse(bytes).unwrap();
    m.instantiate("test", &pef).unwrap()
}

/// What the CPU does for `blrl` through import `index`: TOC word -> TVector -> stub.
fn stub_for(m: &M, frag: &Fragment, index: usize) -> (u32, u32) {
    let data = frag.section_bases[1].unwrap();
    let tv = m.mac.mem.read_u32(data + TOC_FIRST + 4 * index as u32).unwrap();
    (tv, m.mac.mem.read_u32(tv).unwrap())
}

fn index_of(frag: &Fragment, name: &str) -> usize { frag.imports.iter().position(|i| i.symbol == name).unwrap() }

/// Call import `name` the way the CPU would: find its stub through the fragment's TOC, ask the host who it is, call.
fn call(m: &mut M, frag: &Fragment, name: &str, args: &[u32]) -> (GlueOutcome, u32) {
    let (_, stub) = stub_for(m, frag, index_of(frag, name));
    let sym = m.glue_symbol_at(stub).unwrap_or_else(|| panic!("{name}: pc {stub:#x} is not a glue stub"));
    assert_eq!(sym, name, "the stub the TOC leads to is the one for this symbol");
    let mut regs = [0u32; 32];
    regs[3..3 + args.len()].copy_from_slice(args);
    let o = m.glue_call(&sym, &mut regs);
    (o, regs[3])
}

#[test]
fn sections_tvectors_and_imports_are_placed_and_relocated() {
    let mut m = machine();
    let frag = load(&mut m, &fixture::startup_app());
    let (code, data) = (frag.section_bases[0].unwrap(), frag.section_bases[1].unwrap());
    assert_eq!(frag.section_bases[2], None, "the loader section stays in the container");
    // main's TVector {code, toc} was relocated by sectionC and sectionD
    let main = frag.main.unwrap();
    assert_eq!((main.address, main.code, main.toc), (data, code, data));
    assert_eq!(m.mac.mem.read_u32(code).unwrap(), 0x7C08_02A6, "the program's first word (mflr r0) is in guest memory");
    // every TOC word is the TVector of its import, whose code word is that import's own stub
    for (i, imp) in frag.imports.iter().enumerate() {
        let (tv, stub) = stub_for(&m, &frag, i);
        assert_eq!(tv, imp.value, "{}", imp.symbol);
        if imp.symbol == "qd" { continue; } // data: checked below
        assert_eq!(m.glue_symbol_at(stub).as_deref(), Some(imp.symbol.as_str()));
        assert_eq!(m.mac.mem.read_u32(tv + 4).unwrap(), 0, "glue TVectors carry no TOC");
        assert_eq!(m.mac.mem.read_u32(stub).unwrap(), 0x4E80_0020, "the stub is a blr");
    }
    // qd is DATA: the TOC word is the globals block itself, not a TVector
    let qd = frag.imports.iter().find(|i| i.symbol == "qd").unwrap();
    assert_eq!(qd.class, SymbolClass::Data);
    assert_eq!(qd.value, m.qd_block().unwrap());
    assert_eq!(m.glue_symbol_at(qd.value), None);
    // negative control: an address that is not a stub is not recognised, nor is the middle of a cell
    assert_eq!(m.glue_symbol_at(code), None);
    let (tv, stub) = stub_for(&m, &frag, 1);
    assert_eq!(m.glue_symbol_at(tv), None, "the TVector is data, only the stub is the hook");
    assert_eq!(m.glue_symbol_at(stub + 4), None);
}

#[test]
fn prepare_main_returns_through_the_exit_stub() {
    let mut m = machine();
    let frag = load(&mut m, &fixture::startup_app());
    let s = m.prepare_main(&frag).unwrap();
    assert_eq!((s.pc, s.r2), (frag.main.unwrap().code, frag.main.unwrap().toc));
    assert_eq!(s.r1 % 16, 0);
    assert_eq!(m.mac.mem.read_u32(s.r1).unwrap(), 0, "back chain ends");
    let sym = m.glue_symbol_at(s.lr).unwrap();
    let mut regs = [0u32; 32];
    assert_eq!(m.glue_call(&sym, &mut regs), GlueOutcome::Exit, "returning from main ends the program");
    // negative control: a library (no main) has no start state
    let mut b = heirloom_fmt_pef::writer::PefBuilder::new();
    b.code(&fixture::MAIN_CODE);
    let lib = m.instantiate("lib", &Pef::parse(&b.build()).unwrap()).unwrap();
    assert_eq!(m.prepare_main(&lib).unwrap_err(), CfmError::NoMain);
}

#[test]
fn the_census_binds_the_startup_calls_and_names_what_is_next() {
    let mut m = machine();
    let frag = load(&mut m, &fixture::startup_app());
    let c = Census::of(&frag);
    assert_eq!(c.lines.len(), 21);
    for must in ["InitGraf", "InitFonts", "InitWindows", "InitMenus", "TEInit", "InitDialogs", "InitCursor", "NewHandle", "GetResource", "GetNewWindow", "NewWindow", "ShowWindow", "SetPort", "WaitNextEvent", "ExitToShell", "qd"] {
        let i = frag.imports.iter().find(|i| i.symbol == must).unwrap();
        assert!(matches!(i.status, ImportStatus::Bound(_)), "{must}: {:?}", i.status);
    }
    // MoveTo/DrawString bind to the qd lane's QuickDraw handlers by trap name, with no macppc-side change (the reuse
    // this seam exists for); SysBeep is a macppc native (the 68k side has no beep either)
    for must in ["MoveTo", "DrawString", "SysBeep", "FindWindow", "TrackGoAway"] {
        let i = frag.imports.iter().find(|i| i.symbol == must).unwrap();
        assert!(matches!(i.status, ImportStatus::Bound(_)), "{must}: {:?}", i.status);
    }
    assert!(c.next_needed.is_empty(), "{:?}", c.next_needed);
    assert_eq!(c.summary(), "21 imports: 21 bound, 0 NEXT-NEEDED, 0 weak-absent");
    // negative control: calling a NEXT-NEEDED import refuses by name and is recorded like the 68k's unimplemented trap
    let mut m = machine();
    let frag = load(&mut m, &fixture::app(&[("InterfaceLib", false, &[("PlayNoteForever", SymbolClass::TVector, false)])]));
    let (o, _) = call(&mut m, &frag, "PlayNoteForever", &[1]);
    assert!(matches!(&o, GlueOutcome::Refused(w) if w.starts_with("NEXT-NEEDED PlayNoteForever")), "{o:?}");
    assert!(m.mac.refused.iter().any(|r| r.starts_with("unimplemented trap import PlayNoteForever")), "{:?}", m.mac.refused);
}

#[test]
fn memory_manager_calls_by_register_values_run_the_68k_handlers() {
    let mut m = machine();
    let frag = load(&mut m, &fixture::startup_app());
    let (o, h) = call(&mut m, &frag, "NewHandle", &[256]);
    assert_eq!(o, GlueOutcome::Return);
    assert_ne!(h, 0);
    assert_eq!(m.mac.mm.block(h).map(|b| b.size), Some(256), "the Memory Manager the 68k tests use");
    assert!(m.mac.log.iter().any(|l| l.starts_with("[mem] NewHandle 256 -> ")), "the 68k handler's own log line: {:?}", m.mac.log);
    // the same GetResource handler answers resNotFound with no resource file, and ResError reads it
    let (_, r) = call(&mut m, &frag, "GetResource", &[u32::from_be_bytes(*b"WIND"), 128]);
    assert_eq!(r, 0);
    assert_eq!(m.mac.mem.read_u16(heirloom_mac68k_engine::lowmem::RES_ERR).unwrap() as i16, -192);
    // negative control: a refusal comes through exactly as on the 68k (named, logged, call returns 0)
    let (o, _) = call(&mut m, &frag, "GetNewWindow", &[999, 0, 0xFFFF_FFFF]);
    assert_eq!(o, GlueOutcome::Return);
    assert!(m.mac.log.iter().any(|l| l.contains("GetNewWindow 999: no WIND resource")), "{:?}", m.mac.log);
}

/// The proof of reuse across the seam: the PowerPC glue call and the 68k program HELLO run the same NewWindow handler.
#[test]
fn new_window_through_glue_is_the_68k_new_window() {
    let mut bridge = Mac::new(RecordingGuestUi::new()).unwrap();
    bridge.run_program(HELLO, 0, 10_000).unwrap();
    let want_log = bridge.log.iter().find(|l| l.contains("NewWindow")).unwrap().clone();
    let want = bridge.ui.calls.iter().find_map(|c| if let UiCall::OpenWindow(w) = c { Some(w.clone()) } else { None }).unwrap();

    let mut m = machine();
    let frag = load(&mut m, &fixture::startup_app());
    let qd = m.qd_block().unwrap();
    assert_eq!(call(&mut m, &frag, "InitGraf", &[qd + 202]).0, GlueOutcome::Return); // &qd.thePort
    assert_eq!(m.mac.qd_globals, qd + 202, "InitGraf recorded where the PowerPC program keeps its globals");
    m.mac.mem.write_bytes(0x4000, &[0, 40, 0, 30, 0, 240, 1, 94]).unwrap();
    m.mac.mem.write_bytes(0x4010, b"\x09Hello Mac").unwrap();
    let (o, win) = call(&mut m, &frag, "NewWindow", &[0, 0x4000, 0x4010, 0x1_0001, 0, 0xFFFF_FFFF, 1, 0]);
    assert_eq!(o, GlueOutcome::Return);
    assert!(m.mac.windows.contains_key(&win));
    let got_log = m.mac.log.iter().find(|l| l.contains("NewWindow")).unwrap();
    let head = |s: &str| s.split(" -> ").next().unwrap().to_string();
    assert_eq!(head(got_log), head(&want_log));
    let got = m.mac.ui.calls.iter().find_map(|c| if let UiCall::OpenWindow(w) = c { Some(w.clone()) } else { None }).unwrap();
    assert_eq!((got.title, got.rect, got.role, got.closable), (want.title, want.rect, want.role, want.closable));
    // ShowWindow/SetPort on that window go through too; ExitToShell asks the CPU to stop
    assert_eq!(call(&mut m, &frag, "SetPort", &[win]).0, GlueOutcome::Return);
    assert_eq!(call(&mut m, &frag, "ExitToShell", &[]).0, GlueOutcome::Exit);
    // negative control: the empty rect the 68k refuses is refused here, the call returns NIL
    m.mac.mem.write_bytes(0x4100, &[0; 8]).unwrap();
    let (_, nil) = call(&mut m, &frag, "NewWindow", &[0, 0x4100, 0x4010, 1, 0, 0xFFFF_FFFF, 1, 0]);
    assert_eq!(nil, 0);
    assert!(m.mac.refused.iter().any(|r| r.contains("empty boundsRect")));
}

#[test]
fn integer_results_are_sign_extended_and_booleans_are_not() {
    let mut m = machine();
    let frag = load(&mut m, &fixture::app(&[("InterfaceLib", false, &[("ResError", SymbolClass::TVector, false), ("WaitNextEvent", SymbolClass::TVector, false), ("MemError", SymbolClass::TVector, false)])]));
    m.mac.mem.write_u16(heirloom_mac68k_engine::lowmem::RES_ERR, (-192i16) as u16).unwrap();
    let (_, r) = call(&mut m, &frag, "ResError", &[]);
    assert_eq!(r, 0xFFFF_FF40, "an INTEGER result is sign-extended to the register");
    m.mac.mem.write_u16(heirloom_mac68k_engine::lowmem::MEM_ERR, (-108i16) as u16).unwrap();
    assert_eq!(call(&mut m, &frag, "MemError", &[]).1, (-108i32) as u32);
    // with nothing pending: a poll (sleep 0) answers FALSE and writes the null event; a caller that asks to sleep is
    // IDLE (the run loop parks it on the call -- see tests/run.rs), r3 untouched
    let (o, r) = call(&mut m, &frag, "WaitNextEvent", &[0xFFFF, 0x5000, 0, 0]);
    assert_eq!((o, r), (GlueOutcome::Return, 0));
    assert_eq!(m.mac.mem.read_u16(0x5000).unwrap(), 0, "null event");
    let (o, _) = call(&mut m, &frag, "WaitNextEvent", &[0xFFFF, 0x5000, 600, 0]);
    assert_eq!(o, GlueOutcome::Idle);
    // negative control: with an event pending the same call answers TRUE (1, not sign-extended junk)
    m.mac.post(1, 0, 5, 5); // a mouseDown
    let (_, r) = call(&mut m, &frag, "WaitNextEvent", &[0xFFFF, 0x5000, 600, 0]);
    assert_eq!(r, 1, "TRUE, not sign-extended junk");
    assert_eq!(m.mac.mem.read_u16(0x5000).unwrap(), 1, "the EventRecord holds the mouseDown");
}

#[test]
fn weak_strong_and_strict_imports() {
    let libs: &[fixture::Lib] = &[("InterfaceLib", false, &[("NewHandle", SymbolClass::TVector, false), ("SomeFutureCall", SymbolClass::TVector, true), ("MissingStrongCall", SymbolClass::TVector, false), ("missingData", SymbolClass::Data, false)]),
                                  ("OptionalLib", true, &[("Extra", SymbolClass::TVector, false)])];
    let bytes = fixture::app(libs);
    let mut m = machine();
    let frag = load(&mut m, &bytes);
    let st = |n: &str| frag.imports.iter().find(|i| i.symbol == n).unwrap();
    assert!(matches!(st("NewHandle").status, ImportStatus::Bound(_)));
    // weak symbol, and a symbol of a weak library: relocated to ZERO so the fragment can test for them
    assert!(matches!(st("SomeFutureCall").status, ImportStatus::WeakAbsent(_)));
    assert!(matches!(st("Extra").status, ImportStatus::WeakAbsent(_)));
    assert_eq!((st("SomeFutureCall").value, st("Extra").value), (0, 0));
    assert_eq!(stub_for(&m, &frag, index_of(&frag, "Extra")).0, 0, "the TOC word holds 0");
    // strong missing function: a TVector that refuses by name when called
    assert!(matches!(st("MissingStrongCall").status, ImportStatus::Unbound(_)));
    let (o, _) = call(&mut m, &frag, "MissingStrongCall", &[]);
    assert!(matches!(o, GlueOutcome::Refused(w) if w.contains("MissingStrongCall")));
    // strong missing data: an address outside guest RAM, so any access faults on the CPU instead of reading zeros
    let d = st("missingData");
    assert!(matches!(d.status, ImportStatus::Unbound(_)) && d.value == heirloom_macppc_engine::machine::UNBOUND_DATA);
    assert!(!m.mac.mem.contains(d.value, 4));
    let c = Census::of(&frag);
    assert_eq!((c.bound, c.next_needed.len(), c.weak_absent), (1, 2, 2));
    // strict mode: the same fragment is refused at load, naming the first strong import
    let mut strict = machine();
    strict.strict_imports = true;
    let e = strict.instantiate("t", &Pef::parse(&bytes).unwrap()).unwrap_err();
    assert!(matches!(&e, CfmError::UnresolvedImport { library, symbol, .. } if library == "InterfaceLib" && symbol == "MissingStrongCall"), "{e}");
    // negative control: strict mode loads a fragment whose strong imports are all bound
    let mut ok = machine();
    ok.strict_imports = true;
    let all_bound = fixture::app(&[("InterfaceLib", false, &[("NewHandle", SymbolClass::TVector, false), ("qd", SymbolClass::Data, false)])]);
    assert!(ok.instantiate("t", &Pef::parse(&all_bound).unwrap()).is_ok());
}

#[test]
fn a_class_mismatch_is_unbound_not_a_wild_call() {
    // qd imported as a function, NewHandle imported as data
    let bytes = fixture::app(&[("InterfaceLib", false, &[("qd", SymbolClass::TVector, false), ("NewHandle", SymbolClass::Data, false)])]);
    let mut m = machine();
    let frag = load(&mut m, &bytes);
    for i in &frag.imports {
        assert!(matches!(i.status, ImportStatus::Unbound(_)), "{}: {:?}", i.symbol, i.status);
        assert_eq!(i.value, heirloom_macppc_engine::machine::UNBOUND_DATA);
    }
}

#[test]
fn more_imports_than_one_glue_chunk_all_get_distinct_stubs() {
    let names: Vec<String> = (0..300).map(|i| format!("Unknown{i:03}")).collect();
    let syms: Vec<(&str, SymbolClass, bool)> = names.iter().map(|n| (n.as_str(), SymbolClass::TVector, false)).collect();
    let mut m = machine();
    let frag = load(&mut m, &fixture::app(&[("InterfaceLib", false, &syms)]));
    let mut seen = std::collections::BTreeSet::new();
    for (i, n) in names.iter().enumerate() {
        let (tv, stub) = stub_for(&m, &frag, i);
        assert!(seen.insert(stub) && tv != 0);
        assert_eq!(m.glue_symbol_at(stub).as_deref(), Some(n.as_str()), "import {i}, across the {}-cell chunk boundary", heirloom_macppc_engine::glue::CHUNK_CELLS);
    }
    assert!(m.glue.len() >= 300);
}

#[test]
fn cfm_manager_calls_find_libraries_and_symbols() {
    let mut m = machine();
    let frag = load(&mut m, &fixture::app(&[("InterfaceLib", false, &[("GetSharedLibrary", SymbolClass::TVector, false), ("FindSymbol", SymbolClass::TVector, false), ("CloseConnection", SymbolClass::TVector, false), ("NewPtr", SymbolClass::TVector, false)])]));
    let pstr = |m: &mut M, at: u32, s: &str| { m.mac.mem.write_u8(at, s.len() as u8).unwrap(); m.mac.mem.write_bytes(at + 1, s.as_bytes()).unwrap(); };
    pstr(&mut m, 0x6000, "InterfaceLib");
    let (o, e) = call(&mut m, &frag, "GetSharedLibrary", &[0x6000, u32::from_be_bytes(*b"pwpc"), 0, 0x6100, 0x6104, 0x6200]);
    assert_eq!((o, e), (GlueOutcome::Return, 0));
    let conn = m.mac.mem.read_u32(0x6100).unwrap();
    assert_ne!(conn, 0);
    pstr(&mut m, 0x6300, "NewPtr");
    let (_, e) = call(&mut m, &frag, "FindSymbol", &[conn, 0x6300, 0x6400, 0x6404]);
    assert_eq!(e, 0);
    let tv = m.mac.mem.read_u32(0x6400).unwrap();
    assert_eq!(m.mac.mem.read_u8(0x6404).unwrap(), 2, "kTVectorCFragSymbol");
    // the address FindSymbol returned is the SAME TVector the fragment's own import of NewPtr got
    assert_eq!(tv, frag.imports[index_of(&frag, "NewPtr")].value);
    // negative controls
    pstr(&mut m, 0x6500, "NoSuchLibrary");
    assert_eq!(call(&mut m, &frag, "GetSharedLibrary", &[0x6500, 0, 0, 0x6100, 0x6104, 0x6200]).1 as i32, -2804);
    assert_eq!(m.mac.mem.read_u8(0x6200).unwrap(), 13, "errName holds the library that was not found");
    pstr(&mut m, 0x6600, "NoSuchSymbol");
    assert_eq!(call(&mut m, &frag, "FindSymbol", &[conn, 0x6600, 0x6400, 0x6404]).1 as i32, -2802);
    assert_eq!(call(&mut m, &frag, "FindSymbol", &[99, 0x6300, 0x6400, 0x6404]).1 as i32, -2801);
    assert_eq!(call(&mut m, &frag, "CloseConnection", &[0x6100]).1, 0);
    assert_eq!(m.mac.mem.read_u32(0x6100).unwrap(), 0);
    assert_eq!(call(&mut m, &frag, "CloseConnection", &[0x6100]).1 as i32, -2801, "closing twice");
}

#[test]
fn relocation_damage_in_a_fragment_is_refused_by_name() {
    let mut m = machine();
    // a relocation that runs past the end of its (8-byte) section
    let mut b = heirloom_fmt_pef::writer::PefBuilder::new();
    b.code(&fixture::MAIN_CODE);
    let d = b.data(&[0; 8], 8);
    b.reloc(d as u16, &[0x4000 | 2]); // BySectC x3 = 12 bytes
    b.main(d as i32, 0);
    let e = m.instantiate("x", &Pef::parse(&b.build()).unwrap()).unwrap_err();
    assert!(matches!(e, CfmError::Pef(PefError::BadReloc(w)) if w.contains("past the end of its section")), "{e}");
    // an undefined opcode
    let mut b = heirloom_fmt_pef::writer::PefBuilder::new();
    b.code(&fixture::MAIN_CODE);
    let d = b.data(&[0; 8], 8);
    b.reloc(d as u16, &[0x4C00]);
    let e = m.instantiate("x", &Pef::parse(&b.build()).unwrap()).unwrap_err();
    assert!(matches!(e, CfmError::Pef(PefError::UnknownRelocOpcode { opcode: 0x4C00, .. })), "{e}");
    // negative control: the same fragment with a legal relocation loads
    let mut b = heirloom_fmt_pef::writer::PefBuilder::new();
    b.code(&fixture::MAIN_CODE);
    let d = b.data(&[0; 8], 8);
    b.reloc(d as u16, &[0x4000 | 1]);
    b.main(d as i32, 0);
    assert!(m.instantiate("x", &Pef::parse(&b.build()).unwrap()).is_ok());
}

#[test]
fn pattern_data_and_alignment_are_honoured_at_instantiation() {
    let mut m = machine();
    let mut b = heirloom_fmt_pef::writer::PefBuilder::new();
    b.code(&fixture::MAIN_CODE);
    let d = b.pattern_data(&[0x20 | 3, 1, 2, 3, 0x00 | 5], 8, 32);
    b.export("table", SymbolClass::Data, d as i16, 0);
    let frag = m.instantiate("p", &Pef::parse(&b.build()).unwrap()).unwrap();
    let base = frag.section_bases[d].unwrap();
    assert_eq!(base % 4, 0);
    assert_eq!(m.mac.mem.bytes(m.mac.mem.span(base, 32).unwrap()).unwrap()[..9], [1, 2, 3, 0, 0, 0, 0, 0, 0]);
    assert_eq!(frag.exports, vec![("table".to_string(), base)]);
}

#[test]
fn a_huge_total_size_is_refused_before_any_host_allocation_and_over_alignment_is_named() {
    let mut m = machine();
    // totalSize 0xF0000000 from a few bytes of file: refused by the guest allocator (the image is only built after it fits)
    let mut b = heirloom_fmt_pef::writer::PefBuilder::new();
    b.code(&fixture::MAIN_CODE);
    b.data(&[], 0xF000_0000);
    let e = m.instantiate("huge", &Pef::parse(&b.build()).unwrap()).unwrap_err();
    assert!(matches!(e, CfmError::NoMemory { what: "a PEF section", .. }), "{e}");
    // alignment 2^13 is more than the loader provides: refused, not reduced to 2^12
    let mut b = heirloom_fmt_pef::writer::PefBuilder::new();
    let c = b.code(&fixture::MAIN_CODE);
    b.align(c, 13);
    let e = m.instantiate("al", &Pef::parse(&b.build()).unwrap()).unwrap_err();
    assert_eq!(e, CfmError::AlignmentTooLarge { section: c, exponent: 13 });
    // negative control: 2^12 is the largest that loads, and lands aligned
    let mut b = heirloom_fmt_pef::writer::PefBuilder::new();
    let c = b.code(&fixture::MAIN_CODE);
    b.align(c, 12);
    let frag = m.instantiate("al12", &Pef::parse(&b.build()).unwrap()).unwrap();
    assert_eq!(frag.section_bases[c].unwrap() % 4096, 0);
}

#[test]
fn code_class_imports_get_a_code_address_tvector_class_imports_a_tvector() {
    let mut m = machine();
    let frag = load(&mut m, &fixture::app(&[("InterfaceLib", false, &[("NewPtr", SymbolClass::Code, false), ("NewHandle", SymbolClass::TVector, false)])]));
    let code = frag.imports[index_of(&frag, "NewPtr")].value;
    assert_eq!(m.glue_symbol_at(code).as_deref(), Some("NewPtr"), "a Code-class import is the stub itself");
    let tv = frag.imports[index_of(&frag, "NewHandle")].value;
    assert_eq!(m.glue_symbol_at(tv), None, "negative control: a TVector-class import is not a stub address");
    assert_eq!(m.glue_symbol_at(m.mac.mem.read_u32(tv).unwrap()).as_deref(), Some("NewHandle"));
}

#[test]
fn imports_from_a_library_the_engine_does_not_provide_do_not_bind_by_symbol_name() {
    let mut m = machine();
    let frag = load(&mut m, &fixture::app(&[("SomeOtherLib", false, &[("NewPtr", SymbolClass::TVector, false)])]));
    assert!(matches!(&frag.imports[0].status, ImportStatus::Unbound(w) if w.contains("SomeOtherLib")), "{:?}", frag.imports[0].status);
    // negative control: the same symbol from InterfaceLib binds
    let frag = load(&mut m, &fixture::app(&[("InterfaceLib", false, &[("NewPtr", SymbolClass::TVector, false)])]));
    assert!(matches!(frag.imports[0].status, ImportStatus::Bound(_)));
}

#[test]
fn a_closed_connection_is_gone_for_find_symbol_and_ids_are_not_reused() {
    let mut m = machine();
    let frag = load(&mut m, &fixture::app(&[("InterfaceLib", false, &[("GetSharedLibrary", SymbolClass::TVector, false), ("FindSymbol", SymbolClass::TVector, false), ("CloseConnection", SymbolClass::TVector, false)])]));
    let pstr = |m: &mut M, at: u32, s: &str| { m.mac.mem.write_u8(at, s.len() as u8).unwrap(); m.mac.mem.write_bytes(at + 1, s.as_bytes()).unwrap(); };
    pstr(&mut m, 0x6000, "InterfaceLib");
    pstr(&mut m, 0x6300, "NewPtr");
    call(&mut m, &frag, "GetSharedLibrary", &[0x6000, 0, 0, 0x6100, 0x6104, 0x6200]);
    let conn = m.mac.mem.read_u32(0x6100).unwrap();
    assert_eq!(call(&mut m, &frag, "FindSymbol", &[conn, 0x6300, 0x6400, 0x6404]).1, 0, "open: found");
    assert_eq!(call(&mut m, &frag, "CloseConnection", &[0x6100]).1, 0);
    assert_eq!(call(&mut m, &frag, "FindSymbol", &[conn, 0x6300, 0x6400, 0x6404]).1 as i32, -2801, "closed: connection id not found");
    call(&mut m, &frag, "GetSharedLibrary", &[0x6000, 0, 0, 0x6100, 0x6104, 0x6200]);
    assert_ne!(m.mac.mem.read_u32(0x6100).unwrap(), conn, "a reopened library gets a fresh id");
}
