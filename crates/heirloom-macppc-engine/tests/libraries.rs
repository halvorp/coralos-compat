//! Which libraries an import may name (a Carbon / Mac OS 9 application imports the Toolbox from CarbonLib and the *Lib sub-libraries, not only from
//! InterfaceLib) and how an import nobody provides refuses: BY NAME, at the call, with the library in the message.
use heirloom_ui_abi::RecordingGuestUi;
use heirloom_macppc_engine::cfm::{Fragment, ImportStatus};
use heirloom_macppc_engine::fixture::{self, TOC_FIRST};
use heirloom_macppc_engine::{GlueOutcome, MacPpc, PpcHost};
use heirloom_fmt_pef::loader::SymbolClass;
use heirloom_fmt_pef::Pef;

type M = MacPpc<RecordingGuestUi>;
const F: SymbolClass = SymbolClass::TVector;

fn load(m: &mut M, libs: &[(&str, bool, &[(&str, SymbolClass, bool)])]) -> Fragment {
    let bytes = fixture::app(libs);
    m.instantiate("test", &Pef::parse(&bytes).unwrap()).unwrap()
}
/// Call import number `index` the way the CPU would (TOC word -> TVector -> stub -> host), by its position in the fragment's import table.
fn call(m: &mut M, frag: &Fragment, index: usize, args: &[u32]) -> (GlueOutcome, u32) {
    let data = frag.section_bases[1].unwrap();
    let tv = m.mac.mem.read_u32(data + TOC_FIRST + 4 * index as u32).unwrap();
    let stub = m.mac.mem.read_u32(tv).unwrap();
    let sym = m.glue_symbol_at(stub).unwrap_or_else(|| panic!("import {index}: pc {stub:#x} is not a glue stub"));
    let mut regs = [0u32; 32];
    regs[3..3 + args.len()].copy_from_slice(args);
    let o = m.glue_call(&sym, &mut regs);
    (o, regs[3])
}

#[test]
fn carbonlib_and_the_toolbox_sublibraries_resolve_through_the_one_toolbox_table_and_run() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let frag = load(&mut m, &[
        ("CarbonLib", false, &[("NewPtr", F, false), ("FlushEvents", F, false)]),
        ("WindowsLib", false, &[("InitWindows", F, false)]),
        ("ControlsLib", false, &[("GetControlValue", F, false)]),
        ("InterfaceLib", false, &[("GetFPos", F, false)]),
    ]);
    for imp in &frag.imports { assert!(matches!(imp.status, ImportStatus::Bound(_)), "{}/{}: {:?}", imp.library, imp.symbol, imp.status); }
    // and a CALL through a CarbonLib import reaches the handler: NewPtr(64) returns a pointer, MemError 0
    let (o, p) = call(&mut m, &frag, 0, &[64]);
    assert_eq!(o, GlueOutcome::Return);
    assert!(p != 0 && m.mac.mem.read_u8(p).is_ok(), "NewPtr through CarbonLib gave {p:#x}");
    // GetSharedLibrary("CarbonLib") hands out a connection like InterfaceLib's (applications probe for CarbonLib this way)
    assert!(heirloom_macppc_engine::natives::provided("CarbonLib") && heirloom_macppc_engine::natives::provided("WindowsLib") && heirloom_macppc_engine::natives::provided("StdCLib"));
}

#[test]
fn every_toolbox_library_is_also_a_provided_library() {
    assert!(!heirloom_macppc_engine::natives::provided("NetSprocketLib"), "a library the engine does not provide is not claimed");
    assert!(heirloom_macppc_engine::natives::provided("OpenGLLibrary"), "OpenGL is provided since the guest-GL slice: its names are refused or served by the contract table");
}

#[test]
fn a_symbol_of_an_unprovided_library_refuses_by_name_when_called_not_at_load() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    // 3 imports: two from libraries nobody provides (one strong function, one weak), one served; the fragment LOADS (NetSprocket
    // since 2026-10-30, when the Movie Toolbox became a provided library, qt_c.rs)
    let frag = load(&mut m, &[("NetSprocketLib", false, &[("NSpInitialize", F, false)]), ("SpeechLib", true, &[("SpeakString", F, false)]), ("CarbonLib", false, &[("NewPtr", F, false)])]);
    assert!(matches!(&frag.imports[0].status, ImportStatus::Unbound(w) if w.contains("NetSprocketLib is not provided")));
    assert!(matches!(frag.imports[1].status, ImportStatus::WeakAbsent(_)), "{:?}", frag.imports[1].status);
    assert!(matches!(frag.imports[2].status, ImportStatus::Bound(_)));
    let (o, _) = call(&mut m, &frag, 0, &[]);
    match o {
        GlueOutcome::Refused(why) => assert!(why.contains("NEXT-NEEDED") && why.contains("NSpInitialize") && why.contains("NetSprocketLib"), "{why}"),
        other => panic!("expected a named refusal, got {other:?}"),
    }
    // the refusal is also on the engine's record, and a served import still works afterwards
    assert!(m.mac.refused.iter().any(|r| r.contains("NSpInitialize")), "{:?}", m.mac.refused);
    assert_eq!(call(&mut m, &frag, 2, &[16]).0, GlueOutcome::Return);
}

#[test]
fn the_census_counts_bound_named_and_weak_missing_imports_per_library() {
    use heirloom_macppc_engine::census::{Census, LibraryRow};
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let frag = load(&mut m, &[
        ("CarbonLib", false, &[("NewPtr", F, false), ("NoSuchToolboxCall", F, false)]),
        ("OpenGLLibrary", false, &[("glCompileShader", F, false), ("glLinkProgram", F, false)]),
        ("SpeechLib", true, &[("SpeakString", F, false)]),
    ]);
    let c = Census::of(&frag);
    let row = |l: &str, bound, named, weak_missing| LibraryRow { library: l.into(), bound, named, weak_missing };
    assert_eq!(c.libraries, [row("CarbonLib", 1, 1, 0), row("OpenGLLibrary", 0, 2, 0), row("SpeechLib", 0, 0, 1)]);
    // the rows add up to the flat counters (a row that lost an import would not)
    assert_eq!(c.libraries.iter().map(|r| r.bound).sum::<usize>(), c.bound);
    assert_eq!(c.libraries.iter().map(|r| r.named).sum::<usize>(), c.next_needed.len());
    assert_eq!(c.libraries.iter().map(|r| r.weak_missing).sum::<usize>(), c.weak_absent);
    let t = c.library_table();
    assert!(t[0].starts_with("LIBRARY") && t[2].starts_with("OpenGLLibrary") && t[2].contains("2"), "{t:?}");
}

/// The Movie Toolbox with NO decoder (qt_c.rs): the library is provided, Gestalt 'qtim' names its API level (4.0), EnterMovies
/// succeeds, every way of making a movie is refused by NAME with invalidMovie and a NIL movie, calls on a NIL movie are
/// invalidMovie and change nothing, IsMovieDone is TRUE. Warcraft III checked 'qtim' and stopped at "you need QuickTime 4.0".
#[test]
fn the_movie_toolbox_is_there_and_refuses_every_movie_by_name() {
    assert!(heirloom_macppc_engine::natives::provided("Apple;Carbon;Multimedia") == false, "the Movie Toolbox is its own library, not a Toolbox one");
    assert!(heirloom_macppc_engine::qt_c::provided("Apple;Carbon;Multimedia") && heirloom_macppc_engine::qt_c::provided("QuickTimeLib"));
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let frag = load(&mut m, &[("Apple;Carbon;Multimedia", true, &[("EnterMovies", F, false), ("NewMovie", F, false), ("GetMoviesError", F, false), ("IsMovieDone", F, false), ("StartMovie", F, false), ("SetMovieVolume2", F, false)])]);
    for i in 0..5 {
        assert!(matches!(frag.imports[i].status, ImportStatus::Bound(_)), "{i}: {:?}", frag.imports[i].status);
    }
    assert!(matches!(&frag.imports[5].status, ImportStatus::WeakAbsent(_) | ImportStatus::Unbound(_)), "a call outside the served part is not bound");
    assert_eq!(call(&mut m, &frag, 0, &[]), (GlueOutcome::Return, 0), "EnterMovies: noErr");
    assert_eq!(call(&mut m, &frag, 1, &[0]), (GlueOutcome::Return, 0), "NewMovie: NIL");
    assert_eq!(call(&mut m, &frag, 2, &[]).1 as i32, heirloom_macppc_engine::qt_c::INVALID_MOVIE, "GetMoviesError: invalidMovie");
    assert!(m.mac.log.iter().any(|l| l.contains("[QT-MOVIE REFUSED n=1]") && l.contains("NewMovie")), "refused by name: {:?}", m.mac.log);
    assert_eq!(call(&mut m, &frag, 3, &[0]), (GlueOutcome::Return, 1), "IsMovieDone(NIL): TRUE");
    assert_eq!(call(&mut m, &frag, 4, &[0]).0, GlueOutcome::Return, "StartMovie(NIL) changes nothing");
    assert_eq!(heirloom_macppc_engine::gestalt_ppc::answer(*b"qtim"), Some(heirloom_macppc_engine::qt_c::QUICKTIME_4), "'qtim' names the API level served");
}
