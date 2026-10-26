//! The engine on the owner's test image and on its own stubs; the real S60 image is skipped when absent.
//! Named gap: no synthetic E32 EXE fixture yet, so no test runs without the corpus. DELETE WHEN slice 1's console EXE
//! fixture (18-SYMBIAN-EKA2L1.md §9, slice 1 acceptance) lands in this file.
use heirloom_ekaarm_engine::{layout, loader, stubs, Outcome};
use heirloom_fmt_e32::OrdinalTable;

const ORD_TSV: &str = include_str!("../../heirloom-fmt-e32/data/eka-ordinals.tsv");
const HERO: &str = "/data/build/corpus-symbian/e32/HeroOfSparta.exe";

fn hero() -> Option<loader::Process> {
    let bytes = std::fs::read(HERO).ok().or_else(|| { println!("SKIP: {HERO} absent"); None })?;
    Some(loader::load(&bytes, &OrdinalTable::parse(ORD_TSV).unwrap()).unwrap())
}

#[test]
fn a_stub_is_its_svc_then_bx_lr() {
    let b = stubs::encode_stub(5);
    assert_eq!(u32::from_le_bytes(b[..4].try_into().unwrap()), 0xEF00_0000 | (layout::STUB_SVC_BASE + 5));
    assert_eq!(u32::from_le_bytes(b[4..].try_into().unwrap()), 0xE12F_FF1E);
    assert_eq!(stubs::stub_addr(5), layout::STUB_BASE + 40);
}

#[test]
fn the_address_map_has_no_overlap_and_a_too_large_heap_is_refused_by_name() {
    assert_eq!(layout::check_overlap(0xC_0000, 0x5_A000, 0x8F0_E000, 0x2000, 0x1600), Ok(()));
    assert_eq!(layout::check_overlap(0x10_0000, 0x10_0000, 0x6100_0000, 0x10_0000, 0x1000), Err("heap/stubs overlap"));
}

#[test]
fn hero_of_sparta_binds_every_import_and_stops_at_its_first_os_call_by_name() {
    let Some(mut p) = hero() else { return };
    assert_eq!(p.stubs.len(), 704);
    match p.run(1_000_000) {
        Outcome::Refused(why) => assert!(why.contains("euser/") && why.contains("SetupThreadHeap"), "{why}"),
        _ => panic!("expected the first euser call to be refused by name"),
    }
}

#[test]
fn returning_from_the_entry_exits_with_r0() {
    // the exit stub the entry returns into (LR at start); this was refused as an exec call while its SVC and the run loop's disagreed
    let Some(mut p) = hero() else { return };
    p.cpu.regs[0] = 7;
    p.cpu.set_pc_interworking(layout::EXIT_STUB);
    assert!(matches!(p.run(10), Outcome::Exited(7)));
}
