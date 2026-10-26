//! The built-in classes through the real intuition calls (call scripts): imageclass / gadgetclass / strgclass instance
//! data lives in the guest structs, attributes round-trip, DisposeObject gives the memory back, and the negative
//! results (unknown attribute, unknown class, strgclass editing) are the AROS ones or named refusals.
mod gfxcommon;
use gfxcommon::*;

const NEW_OBJECT_A: i32 = -636;
const DISPOSE_OBJECT: i32 = -642;
const SET_ATTRS_A: i32 = -648;
const GET_ATTR: i32 = -654;
const IA: u32 = TAG_USER + 0x20000;
const GA: u32 = TAG_USER + 0x30000;
const STRINGA: u32 = TAG_USER + 0x32000;

#[test]
fn imageclass_is_a_struct_image_with_the_aros_defaults() {
    let mut g = G::new();
    let (cn, bad) = (g.s("imageclass"), g.s("noSuchImageClass"));
    let t = g.tags(&[(IA + 1, 2), (IA + 3, 30), (IA + 5, 7)]);
    let out = g.zeroed(8);
    g.run(&[intu(NEW_OBJECT_A, &[a(0, 0), a(1, cn), a(2, t)]), intu(GET_ATTR, &[d(0, IA + 3), a_res(0, 0), a(1, out)]),
        intu(NEW_OBJECT_A, &[a(0, 0), a(1, bad), a(2, 0)])]);
    let im = g.result(0);
    assert_ne!(im, 0);
    assert_eq!((g.r16(im) as i16, g.r16(im + 4), g.r16(im + 6)), (2, 30, 40), "LeftEdge/Width from tags; Height keeps imageclass's default 40");
    assert_eq!(g.r16(im + 8), 0xFFFF, "Depth = CUSTOMIMAGEDEPTH: how Intuition knows a boopsi image");
    assert_eq!(g.r8(im + 14), 7, "IA_FGPen is PlanePick");
    assert_eq!((g.result(1), g.r32(out)), (1, 30), "GetAttr(IA_Width)");
    assert_eq!(g.result(2), 0, "control: a class that does not exist is NULL");
    assert_eq!(g.a.classes.objs.len(), 1);
}

#[test]
fn gadgetclass_tags_set_the_struct_gadget_and_unknown_get_is_false() {
    let mut g = G::new();
    let cn = g.s("gadgetclass");
    let t = g.tags(&[(GA + 1, 5), (GA + 3, 6), (GA + 5, 70), (GA + 7, 20), (GA + 16, 9), (GA + 22, 1), (GA + 4, (-3i32) as u32)]);
    let (out, out2) = (g.zeroed(8), g.zeroed(8));
    let t2 = g.tags(&[(GA + 14, 1)]);
    g.run(&[intu(NEW_OBJECT_A, &[a(0, 0), a(1, cn), a(2, t)]), intu(GET_ATTR, &[d(0, GA + 16), a_res(0, 0), a(1, out)]),
        intu(GET_ATTR, &[d(0, GA + 99), a_res(0, 0), a(1, out2)]), intu(SET_ATTRS_A, &[a_res(0, 0), a(1, t2)]),
        intu(DISPOSE_OBJECT, &[a_res(0, 0)])]);
    let o = g.result(0);
    assert_eq!((g.r16(o + 4), g.r16(o + 6), g.r16(o + 8), g.r16(o + 10)), (5, (-3i16) as u16, 70, 20), "GA_RelBottom overrides GA_Top, as in AROS");
    assert_eq!(g.r16(o + 38), 9, "GA_ID");
    assert_eq!(g.r16(o + 14) & 1, 1, "GACT_RELVERIFY");
    assert_eq!(g.r16(o + 12) & 0x8000, 0x8000, "GFLG_EXTENDED");
    assert_eq!(g.r16(o + 12) & 0x100, 0x100, "GA_Disabled set by SetAttrsA");
    assert_eq!((g.result(1), g.r32(out)), (1, 9));
    assert_eq!((g.result(2), g.r32(out2)), (0, 0), "an attribute gadgetclass does not know: GetAttr FALSE, storage untouched");
    assert_eq!(g.result(3), 1, "SetAttrsA reports a visual change");
    assert!(g.a.classes.objs.is_empty(), "DisposeObject freed the object and forgot it");
}

#[test]
fn strgclass_keeps_its_text_in_a_real_string_info() {
    let mut g = G::new();
    let (cn, hello, bye) = (g.s("strgclass"), g.s("hello"), g.s("bye"));
    let t = g.tags(&[(STRINGA + 1, 20), (STRINGA + 18, hello), (GA + 16, 3)]);
    let t2 = g.tags(&[(STRINGA + 18, bye)]);
    let out = g.zeroed(8);
    g.run(&[intu(NEW_OBJECT_A, &[a(0, 0), a(1, cn), a(2, t)]), intu(SET_ATTRS_A, &[a_res(0, 0), a(1, t2)]),
        intu(GET_ATTR, &[d(0, STRINGA + 18), a_res(0, 0), a(1, out)]), intu(DISPOSE_OBJECT, &[a_res(0, 0)])]);
    let o = g.result(0);
    let si = g.r32(o + 34);
    assert_ne!(si, 0, "Gadget.SpecialInfo points at the StringInfo");
    assert_eq!(si, o + 72, "...which is the instance data right after the 72-byte gadgetclass part");
    assert_eq!(g.r16(si + 10), 20, "STRINGA_MaxChars");
    assert_eq!(g.r16(o + 12) & 0x400, 0x400, "GFLG_STRINGEXTEND");
    assert_eq!(g.r16(o + 16) & 7, 5, "still a custom gadget type");
    let buf = g.r32(out);
    assert_ne!(buf, 0);
    assert_eq!(g.a.mem.read_cstr(buf).unwrap(), "bye", "OM_SET of STRINGA_TextVal replaced the text in the allocated buffer");
    assert_eq!(g.r32(si + 24), si + 36, "StringInfo.Extension points at the StringExtend");
    assert!(g.a.classes.strg_bufs.is_empty(), "OM_DISPOSE freed the buffers strgclass allocated");
}

const PGA: u32 = TAG_USER + 0x31000;

#[test]
fn propgclass_keeps_a_real_prop_info_and_derives_pot_and_body_from_total_visible_top() {
    let mut g = G::new();
    let cn = g.s("propgclass");
    let t = g.tags(&[(PGA + 7, 100), (PGA + 8, 25), (PGA + 9, 30)]);
    let t2 = g.tags(&[(PGA + 9, 500)]); // past the end: clamped to Total - Visible
    let out = g.zeroed(8);
    g.run(&[intu(NEW_OBJECT_A, &[a(0, 0), a(1, cn), a(2, t)]), intu(GET_ATTR, &[d(0, PGA + 9), a_res(0, 0), a(1, out)])]);
    let o = g.result(0);
    let pi = g.r32(o + 34);
    assert_eq!(pi, o + 72, "Gadget.SpecialInfo points at the PropInfo, the instance data after the gadgetclass part");
    assert_eq!(g.r16(pi) & 0x15, 0x15, "defaults AUTOKNOB|FREEVERT|PROPNEWLOOK");
    assert_eq!(g.r16(pi + 8), (25u32 * 0xFFFF / 100) as u16, "VertBody = Visible/Total of MAXBODY");
    assert_eq!(g.r16(pi + 4), (30u32 * 0xFFFF / 75) as u16, "VertPot = Top/(Total-Visible) of MAXPOT");
    assert_eq!(g.r32(out), 30, "PGA_Top reads back");
    assert_eq!(g.r16(pi + 2), 0, "control: the horizontal pot was never touched");
    g.run(&[intu(SET_ATTRS_A, &[a(0, o), a(1, t2)])]);
    assert_eq!(g.r16(pi + 4), 0xFFFF, "Top past the end is clamped to the last position");
    let t3 = g.tags(&[(PGA + 1, 2), (PGA + 8, 100)]); // PGA_Freedom FREEHORIZ, everything visible
    g.run(&[intu(SET_ATTRS_A, &[a(0, o), a(1, t3)])]);
    assert_eq!(g.r16(pi) & 6, 2, "PGA_Freedom replaced FREEVERT by FREEHORIZ");
    assert_eq!((g.r16(pi + 6), g.r16(pi + 2)), (0xFFFF, 0), "Visible >= Total: full body, pot 0 (the knob fills the container)");
}
