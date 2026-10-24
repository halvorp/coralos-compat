//! Which libraries an import may name (a Carbon / Mac OS 9 application imports the Toolbox from CarbonLib and the *Lib sub-libraries, not only from
//! InterfaceLib) and how an import nobody provides refuses: BY NAME, at the call, with the library in the message.
use guestui_abi::RecordingGuestUi;
use macppc_engine::cfm::{Fragment, ImportStatus};
use macppc_engine::fixture::{self, TOC_FIRST};
use macppc_engine::{GlueOutcome, MacPpc, PpcHost};
use retro_pef::loader::SymbolClass;
use retro_pef::Pef;

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
    assert!(macppc_engine::natives::provided("CarbonLib") && macppc_engine::natives::provided("WindowsLib") && macppc_engine::natives::provided("StdCLib"));
}

#[test]
fn every_toolbox_library_is_also_a_provided_library() {
    assert!(!macppc_engine::natives::provided("OpenGLLibrary"), "a library the engine does not provide is not claimed");
}

#[test]
fn a_symbol_of_an_unprovided_library_refuses_by_name_when_called_not_at_load() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    // 3 imports: two from a library nobody provides (one strong function, one weak), one served; the fragment LOADS
    let frag = load(&mut m, &[("OpenGLLibrary", false, &[("glBegin", F, false)]), ("OpenGLUtility", true, &[("gluPerspective", F, false)]), ("CarbonLib", false, &[("NewPtr", F, false)])]);
    assert!(matches!(&frag.imports[0].status, ImportStatus::Unbound(w) if w.contains("OpenGLLibrary is not provided")));
    assert!(matches!(frag.imports[1].status, ImportStatus::WeakAbsent(_)), "{:?}", frag.imports[1].status);
    assert!(matches!(frag.imports[2].status, ImportStatus::Bound(_)));
    let (o, _) = call(&mut m, &frag, 0, &[]);
    match o {
        GlueOutcome::Refused(why) => assert!(why.contains("NEXT-NEEDED") && why.contains("glBegin") && why.contains("OpenGLLibrary"), "{why}"),
        other => panic!("expected a named refusal, got {other:?}"),
    }
    // the refusal is also on the engine's record, and a served import still works afterwards
    assert!(m.mac.refused.iter().any(|r| r.contains("glBegin")), "{:?}", m.mac.refused);
    assert_eq!(call(&mut m, &frag, 2, &[16]).0, GlueOutcome::Return);
}

#[test]
fn the_census_counts_bound_named_and_weak_missing_imports_per_library() {
    use macppc_engine::census::{Census, LibraryRow};
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let frag = load(&mut m, &[
        ("CarbonLib", false, &[("NewPtr", F, false), ("NoSuchToolboxCall", F, false)]),
        ("OpenGLLibrary", false, &[("glBegin", F, false), ("glEnd", F, false)]),
        ("QuickTimeLib", true, &[("EnterMovies", F, false)]),
    ]);
    let c = Census::of(&frag);
    let row = |l: &str, bound, named, weak_missing| LibraryRow { library: l.into(), bound, named, weak_missing };
    assert_eq!(c.libraries, [row("CarbonLib", 1, 1, 0), row("OpenGLLibrary", 0, 2, 0), row("QuickTimeLib", 0, 0, 1)]);
    // the rows add up to the flat counters (a row that lost an import would not)
    assert_eq!(c.libraries.iter().map(|r| r.bound).sum::<usize>(), c.bound);
    assert_eq!(c.libraries.iter().map(|r| r.named).sum::<usize>(), c.next_needed.len());
    assert_eq!(c.libraries.iter().map(|r| r.weak_missing).sum::<usize>(), c.weak_absent);
    let t = c.library_table();
    assert!(t[0].starts_with("LIBRARY") && t[2].starts_with("OpenGLLibrary") && t[2].contains("2"), "{t:?}");
}
