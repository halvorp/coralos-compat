//! The application start (`EikStart::RunApplication` as a state machine over the callback machine) and its vtable-anchor
//! check. The synthetic tests run without the corpus; the FileBrowser / HeroOfSparta ones are skipped when the images are absent.
use heirloom_cpu_arm::Memory;
use heirloom_ekaarm_engine::appstart::{self, SlotSpec, Slots};
use heirloom_ekaarm_engine::layout::{CODE_BASE, EXIT_SVC, STUB_BASE, STUB_SVC_BASE};
use heirloom_ekaarm_engine::{loader, Outcome, Process};
use heirloom_fmt_e32::OrdinalTable;

const ORD_TSV: &str = include_str!("../../heirloom-fmt-e32/data/eka-ordinals.tsv");
const FILE_BROWSER: &str = "/data/build/corpus-symbian/e32/FileBrowser.exe";
const HERO: &str = "/data/build/corpus-symbian/e32/HeroOfSparta.exe";

const BX_LR: u32 = 0xE12F_FF1E;
const RUN_APPLICATION: &str = "_ZN8EikStart14RunApplicationE22TApaApplicationFactory";
const EIK_CREATE_DOC: &str = "_ZN15CEikApplication15CreateDocumentLEP11CApaProcess";
const UID3: u32 = 0x100;
const APP: u32 = 0x0800_0000;
const DOC: u32 = 0x0810_0000;
const APPUI: u32 = 0x0820_0000;

fn words(w: &[u32]) -> Vec<u8> {
    w.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// An object at `at` whose vtable is at `at + 0x100`, with `slots` = (slot, address) pairs.
fn object(p: &mut Process, at: u32, slots: &[(u32, u32)]) {
    let mut w = vec![0u32; 0x100 / 4 + 32];
    w[0] = at + 0x100;
    for (s, a) in slots {
        w[0x100 / 4 + *s as usize] = *a;
    }
    p.map_data(at, &w).unwrap();
}

fn rd(p: &mut Process, a: u32) -> u32 {
    p.mem.read_u32(a).unwrap()
}

/// A synthetic application: stub 0 = RunApplication, stub 1 = CEikApplication::CreateDocumentL(CApaProcess*).
/// The guest code: a factory returning APP, AppDllUid returning UID3, the private CreateDocumentL() returning DOC,
/// CreateAppUiL returning APPUI, an AppUi::ConstructL that just returns.
fn synthetic() -> Process {
    let mut p = loader::bare(&[RUN_APPLICATION, EIK_CREATE_DOC]).unwrap();
    p.uid3 = UID3;
    let (factory, uid, doc_fn, appui_fn, construct) = (CODE_BASE + 0x10, CODE_BASE + 0x20, CODE_BASE + 0x30, CODE_BASE + 0x40, CODE_BASE + 0x50);
    p.map_code(CODE_BASE, &words(&[
        0xEF00_0000 | STUB_SVC_BASE, 0xEF00_0000 | EXIT_SVC, 0, 0,
        0xE3A0_0408, BX_LR, 0, 0,            // factory: mov r0, #0x08000000; bx lr
        0xE3A0_0C01, BX_LR, 0, 0,            // AppDllUid: mov r0, #0x100; bx lr
        0xE3A0_0681, BX_LR, 0, 0,            // CreateDocumentL(): mov r0, #0x08100000; bx lr
        0xE3A0_0682, BX_LR, 0, 0,            // CreateAppUiL: mov r0, #0x08200000; bx lr
        BX_LR,                               // ConstructL
    ])).unwrap();
    object(&mut p, APP, &[(appstart::SLOT_APP_CREATE_DOCUMENT, STUB_BASE + 8), (appstart::SLOT_APP_DLL_UID, uid), (appstart::SLOT_EIK_APP_CREATE_DOCUMENT_NOARG, doc_fn)]);
    object(&mut p, DOC, &[(appstart::SLOT_DOC_CREATE_APP_UI, appui_fn)]);
    object(&mut p, APPUI, &[(appstart::SLOT_APPUI_CONSTRUCT_L, construct)]);
    // the call: RunApplication(factory) with the factory's four words in r0-r3 (type function, fn, 0, 0)
    p.cpu.regs[..4].copy_from_slice(&[0, factory, 0, 0]);
    p.cpu.set_pc_interworking(CODE_BASE);
    p
}

#[test]
fn the_application_is_started_through_the_guests_own_code_up_to_construct_l() {
    let mut p = synthetic();
    match p.run(10_000) {
        Outcome::Refused(m) => assert!(m.contains("CActiveScheduler::Start"), "{m}"),
        o => panic!("{o:?}"),
    }
    let log = p.log.join("\n");
    for want in ["factory -> CApaApplication", "AppDllUid 0x00000100 == UID3 0x00000100", "CreateDocumentL(process", "CreateAppUiL(doc", "ConstructL returned"] {
        assert!(log.contains(want), "{want} missing from:\n{log}");
    }
    // the inherited CEikApplication::CreateDocumentL (a native) ran the private one and set the document's fields
    let process = p.nat.apa_process.expect("placeholder CApaProcess");
    assert_eq!(rd(&mut p, DOC + appstart::DOC_I_APPLICATION), APP);
    assert_eq!(rd(&mut p, DOC + appstart::DOC_I_APA_PROCESS), process);
    assert_eq!(rd(&mut p, process), 0, "the placeholder has no vtable pointer");
    assert_eq!(p.callback_depth(), 0);
}

#[test]
fn a_wrong_app_dll_uid_is_a_named_fault() {
    let mut p = synthetic();
    p.uid3 = UID3 + 1;
    match p.run(10_000) {
        Outcome::Fault(m) => assert!(m.contains("AppDllUid() returned 0x00000100") && m.contains("UID3"), "{m}"),
        o => panic!("{o:?}"),
    }
}

#[test]
fn a_factory_returning_nothing_is_refused_by_name() {
    let mut p = synthetic();
    p.map_code(CODE_BASE + 0x1000, &words(&[0xE3A0_0000, BX_LR])).unwrap(); // mov r0, #0; bx lr
    p.cpu.regs[1] = CODE_BASE + 0x1000;
    assert!(matches!(p.run(10_000), Outcome::Refused(m) if m == "factory returned no application"));
}

#[test]
fn a_factory_passed_by_reference_is_not_mistaken_for_one_passed_in_registers() {
    let mut p = synthetic();
    p.cpu.regs[0] = 0x0BFF_0000; // what a hidden reference would put in r0
    match p.run(10_000) {
        Outcome::Refused(m) => assert!(m.contains("not a function-type TApaApplicationFactory"), "{m}"),
        o => panic!("{o:?}"),
    }
}

// ---- the anchor check

fn spec(slot: u32, inherited: &'static [&'static str]) -> SlotSpec {
    SlotSpec { class: "CApaApplication", method: "AppDllUid", slot, inherited }
}

#[test]
fn the_anchor_accepts_a_guest_override_and_the_expected_inherited_stub_and_nothing_else() {
    let mut p = loader::bare(&["_ZExpected", "_ZOther"]).unwrap();
    p.map_code(CODE_BASE, &words(&[BX_LR])).unwrap();
    object(&mut p, APP, &[(5, CODE_BASE + 1), (6, STUB_BASE), (7, STUB_BASE + 8), (8, 0xDEAD_BEEF), (9, STUB_BASE + 4)]);
    assert_eq!(appstart::anchor(&p, APP, &spec(5, &[])), Ok(CODE_BASE + 1), "a (Thumb) address inside the loaded code");
    assert_eq!(appstart::anchor(&p, APP, &spec(6, &["_ZExpected"])), Ok(STUB_BASE), "the expected base-class stub");
    let wrong = appstart::anchor(&p, APP, &spec(7, &["_ZExpected"])).unwrap_err();
    assert!(wrong.contains("vtable slot 7 of CApaApplication (AppDllUid)") && wrong.contains("_ZOther"), "{wrong}");
    let pure = appstart::anchor(&p, APP, &spec(6, &[])).unwrap_err();
    assert!(pure.contains("vtable slot 6") && pure.contains("_ZExpected"), "a stub in a pure slot: {pure}");
    let junk = appstart::anchor(&p, APP, &spec(8, &[])).unwrap_err();
    assert!(junk.contains("holds 0xdeadbeef"), "{junk}");
    let mid = appstart::anchor(&p, APP, &spec(9, &[])).unwrap_err();
    assert!(mid.contains("neither in the loaded code"), "an address in the stub page but not a stub: {mid}");
    let zero = appstart::anchor(&p, 0x0900_0000, &spec(5, &[]));
    assert!(zero.is_err(), "an unmapped object");
}

#[test]
fn an_object_without_a_vtable_pointer_is_refused_by_name() {
    let mut p = loader::bare(&[]).unwrap();
    p.map_data(APP, &[0; 4]).unwrap();
    let e = appstart::anchor(&p, APP, &spec(5, &[])).unwrap_err();
    assert!(e.contains("vtable pointer 0") && e.contains("CApaApplication"), "{e}");
}

#[test]
fn negative_control_one_slot_off_by_one_is_caught_by_the_anchor() {
    let mut p = synthetic();
    p.slots = Slots { app_dll_uid: appstart::SLOT_APP_DLL_UID + 1, ..Slots::default() };
    match p.run(10_000) {
        Outcome::Fault(m) => assert!(m.contains("vtable slot 6 of CApaApplication (AppDllUid)"), "{m}"),
        o => panic!("the corrupted slot was not caught: {o:?}"),
    }
    let mut p = synthetic();
    p.slots = Slots { appui_construct_l: appstart::SLOT_APPUI_CONSTRUCT_L + 1, ..Slots::default() };
    match p.run(10_000) {
        Outcome::Fault(m) => assert!(m.contains("vtable slot 17 of CEikAppUi (ConstructL)"), "{m}"),
        o => panic!("the corrupted slot was not caught: {o:?}"),
    }
}

// ---- the owner's corpus (skipped when absent)

fn corpus(path: &str) -> Option<Process> {
    let bytes = std::fs::read(path).ok().or_else(|| { println!("SKIP: {path} absent"); None })?;
    Some(loader::load(&bytes, &OrdinalTable::parse(ORD_TSV).unwrap()).unwrap())
}

#[test]
fn file_browser_starts_its_application_and_stops_at_a_named_call() {
    let Some(mut p) = corpus(FILE_BROWSER) else { return };
    let outcome = p.run(5_000_000);
    let log = p.log.join("\n");
    assert!(log.contains("AppDllUid 0x102828d6 == UID3 0x102828d6"), "{log}");
    match outcome {
        // today: its document constructor needs the CCoeEnv singleton (cone _ZN7CCoeEnv6StaticEv = CCoeEnv::Static), not yet built
        Outcome::Refused(m) => assert!(m.starts_with("NEXT-NEEDED") && (m.contains("BaseConstructL") || m.contains("_ZN7CCoeEnv6StaticEv")), "{m}"),
        o => panic!("expected a named refusal: {o:?}"),
    }
}

#[test]
fn hero_of_sparta_creates_its_document_and_appui_and_stops_at_the_first_base_construct_l() {
    let Some(mut p) = corpus(HERO) else { return };
    let outcome = p.run(5_000_000);
    let log = p.log.join("\n");
    for want in ["AppDllUid 0x20030e78 == UID3 0x20030e78", "CreateDocumentL(process", "CreateAppUiL(doc"] {
        assert!(log.contains(want), "{want} missing from:\n{log}");
    }
    match outcome {
        Outcome::Refused(m) => assert!(m.starts_with("NEXT-NEEDED") && m.contains("BaseConstructL"), "{m}"),
        o => panic!("expected a refusal naming BaseConstructL: {o:?}"),
    }
}

#[test]
fn negative_control_on_the_real_image_a_wrong_slot_is_a_named_fault() {
    let Some(mut p) = corpus(HERO) else { return };
    p.slots = Slots { app_dll_uid: appstart::SLOT_APP_DLL_UID + 1, ..Slots::default() };
    match p.run(5_000_000) {
        Outcome::Fault(m) => assert!(m.contains("vtable slot 6 of CApaApplication (AppDllUid)"), "{m}"),
        o => panic!("the corrupted slot was not caught: {o:?}"),
    }
    let Some(mut p) = corpus(HERO) else { return };
    p.slots = Slots { appui_construct_l: appstart::SLOT_APPUI_CONSTRUCT_L + 1, ..Slots::default() };
    match p.run(5_000_000) {
        Outcome::Fault(m) => assert!(m.contains("vtable slot 17 of CEikAppUi (ConstructL)"), "{m}"),
        o => panic!("the corrupted slot was not caught: {o:?}"),
    }
}
