//! intuition.library ModifyProp / NewModifyProp on a classic GTYP_PROPGADGET: the PropInfo the program hands over is stored in the
//! guest's struct and the gadget's NATIVE ScrollBar follows (AROS rom/intuition newmodifyprop.c, modifyprop.c). Each behaviour has a
//! control that would FAIL without it.
mod gfxcommon;
use gfxcommon::*;
use heirloom_ui_abi::*;

const ADD_GADGET: i32 = -42;
const MODIFY_PROP: i32 = -156;
const NEW_MODIFY_PROP: i32 = -468;
const FREEHORIZ: u16 = 2;
const FREEVERT: u16 = 4;
const AUTOKNOB: u16 = 1;

struct Fx { g: G, win: u32, gad: u32, pi: u32 }

/// A window with one classic prop gadget of type `ty` (GadgetType), vertical, pot 0, body 0x1000.
fn fixture(ty: u16) -> Fx {
    let mut g = G::new();
    let nw = g.zeroed(48);
    for (o, v) in [(0u32, 0u16), (2, 0), (4, 200), (6, 100)] { g.a.mem.write_u16(nw + o, v).unwrap(); }
    g.a.mem.write_u32(nw + 14, 0x800).unwrap(); // WFLG_BORDERLESS
    g.run(&[intu(OPEN_WINDOW, &[a(0, nw)])]);
    let win = g.result(0);
    let pi = g.words(&[FREEVERT | AUTOKNOB, 0, 0, 0xFFFF, 0x1000, 0, 0, 0, 0, 0, 0]);
    let mut gad = vec![0u8; 44];
    for (off, v) in [(4usize, 180u16), (6, 10), (8, 12), (10, 80), (16, ty)] { gad[off..off + 2].copy_from_slice(&v.to_be_bytes()); }
    gad[34..38].copy_from_slice(&pi.to_be_bytes());
    let gad = g.bytes(&gad);
    g.run(&[intu(ADD_GADGET, &[a(0, win), a(1, gad), d(0, 0xFFFF)])]);
    assert!(g.a.refused.is_empty(), "{:?}", g.a.refused);
    Fx { g, win, gad, pi }
}

fn kind(f: &Fx) -> ControlKind {
    f.g.a.ui.controls.values().find(|(_, m)| m.token.0 == f.gad as u64).map(|(_, m)| m.kind.clone()).expect("the prop gadget is a native control")
}

fn call(f: &mut Fx, lvo: i32, flags: u16, hpot: u16, vpot: u16, hbody: u16, vbody: u16, num: Option<i16>) {
    let mut regs = vec![a(0, f.gad), a(1, f.win), a(2, 0), d(0, flags as u32 | 0xABCD_0000), d(1, hpot as u32), d(2, vpot as u32), d(3, hbody as u32), d(4, vbody as u32)];
    if let Some(n) = num { regs.push(d(5, n as u16 as u32 | 0x1234_0000)); }
    f.g.run(&[intu(lvo, &regs)]);
}

/// The scrollbar the pots-and-bodies conversion gives: total 0xFFFF, visible = body, top = pot x (total - body) / 0xFFFF.
fn bar(pot: u32, body: u32) -> ControlKind { ControlKind::ScrollBar { vertical: true, total: 0xFFFF, visible: body, top: (pot as u64 * (0xFFFF - body) as u64 / 0xFFFF) as u32 } }

#[test]
fn new_modify_prop_moves_the_native_scrollbar_and_stores_the_propinfo() {
    let mut f = fixture(3);
    assert_eq!(kind(&f), bar(0, 0x1000), "control: the registration state");
    call(&mut f, NEW_MODIFY_PROP, FREEVERT | AUTOKNOB, 0, 0x8000, 0xFFFF, 0x2000, Some(1));
    assert_eq!(kind(&f), bar(0x8000, 0x2000));
    assert_eq!((f.g.r16(f.pi), f.g.r16(f.pi + 4), f.g.r16(f.pi + 8)), (FREEVERT | AUTOKNOB, 0x8000, 0x2000), "Flags, VertPot, VertBody (the registers' upper halves are the caller's)");
}

#[test]
fn num_gad_zero_stores_the_propinfo_and_refreshes_nothing() {
    let mut f = fixture(3);
    call(&mut f, NEW_MODIFY_PROP, FREEVERT | AUTOKNOB, 0, 0x4000, 0xFFFF, 0x1000, Some(0));
    assert_eq!(f.g.r16(f.pi + 4), 0x4000, "the PropInfo changed (newmodifyprop.c:119-123)");
    assert_eq!(kind(&f), bar(0, 0x1000), "but numGad 0 refreshes not even this gadget, so the host still shows the old state");
    call(&mut f, NEW_MODIFY_PROP, FREEVERT | AUTOKNOB, 0, 0x4000, 0xFFFF, 0x1000, Some(-1));
    assert_eq!(kind(&f), bar(0x4000, 0x1000), "numGad -1 refreshes");
}

#[test]
fn a_flag_change_to_the_other_axis_is_a_horizontal_scrollbar() {
    let mut f = fixture(3);
    call(&mut f, NEW_MODIFY_PROP, FREEHORIZ | AUTOKNOB, 0xFFFF, 0, 0x3000, 0xFFFF, Some(1));
    assert_eq!(kind(&f), ControlKind::ScrollBar { vertical: false, total: 0xFFFF, visible: 0x3000, top: 0xFFFF * (0xFFFF - 0x3000) / 0xFFFF });
}

#[test]
fn modify_prop_refreshes_the_rest_of_the_list() {
    let mut f = fixture(3);
    call(&mut f, MODIFY_PROP, FREEVERT | AUTOKNOB, 0, 0xFFFF, 0xFFFF, 0x0800, None);
    assert_eq!(kind(&f), bar(0xFFFF, 0x0800));
}

#[test]
fn a_gadget_that_is_not_a_propgadget_is_left_alone() {
    // NEGATIVE CONTROL (newmodifyprop.c:85-87): a GTYP_CUSTOMGADGET (what propgclass makes) is not a GTYP_PROPGADGET.
    let mut f = fixture(5);
    call(&mut f, NEW_MODIFY_PROP, FREEVERT | AUTOKNOB, 0, 0x8000, 0xFFFF, 0x2000, Some(1));
    assert_eq!(f.g.r16(f.pi + 4), 0, "no PropInfo word was written");
    assert!(f.g.a.refused.is_empty(), "{:?}", f.g.a.refused);
}
