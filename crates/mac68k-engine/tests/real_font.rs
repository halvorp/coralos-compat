//! `RealFont(fontNum, size)` (Font Manager): true only for a font that exists in that size as a resource of its own.
mod common;
use common::{Prog, GLOBALS};
use guestui_abi::RecordingGuestUi;
use mac68k_engine::fixture::{ScriptArg::Lit, ScriptCall};
use mac68k_engine::{Mac, Step};

/// A FOND for family 20 listing sizes 12 (font id 2572 present) and 24 (font id 2584 absent), as a `FamRec` + association table.
fn fond() -> Vec<u8> {
    let mut d = vec![0u8; 52];
    d.extend_from_slice(&1i16.to_be_bytes()); // two associations
    for (size, id) in [(12i16, 20 * 128 + 12), (24, 20 * 128 + 24)] { for v in [size, 0, id] { d.extend_from_slice(&v.to_be_bytes()); } }
    d
}

fn ask(fam: u32, size: u32) -> u16 {
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    mac68k_engine::quickdraw::fill_globals(&mut m, GLOBALS).unwrap();
    m.qd_globals = GLOBALS;
    let fork = retro_resfork::writer::ForkBuilder::new()
        .add(b"FONT", 3 * 128 + 9, None, &[0; 4])     // Geneva 9 as a bitmap font
        .add(b"FOND", 20, Some("Times"), &fond())
        .add(b"NFNT", 20 * 128 + 12, None, &[0; 4])   // the one Times size its FOND names that exists
        .build();
    m.rm.open_fork(mac68k_engine::resmgr::APP_REF_NUM, retro_resfork::ResFork::parse(fork).unwrap());
    let mut p = Prog::new();
    p.script(&m, &[ScriptCall { name: "RealFont", args: vec![Lit(fam), Lit(size)], save: Some(0x3000) }]).exit();
    m.start(&p.0, 0).unwrap();
    assert_eq!(m.step(100_000).0, Step::Exited, "{:#?}", m.log);
    assert!(m.refused.is_empty(), "{:?}", m.refused);
    m.mem.read_u16(0x3000).unwrap() >> 8
}

#[test]
fn a_font_is_real_only_where_a_font_resource_backs_the_size() {
    assert_eq!(ask(3, 9), 1, "a FONT resource with id family*128+size");
    assert_eq!(ask(20, 12), 1, "a FOND association whose font resource exists");
    assert_eq!(ask(20, 24), 0, "a FOND association whose font resource is absent");
    assert_eq!(ask(3, 10), 0, "no such size");
    assert_eq!(ask(21, 12), 0, "a stock family nothing backs: drawn in the stand-in face, not real");
}
