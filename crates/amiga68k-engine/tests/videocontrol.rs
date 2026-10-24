//! graphics.library VideoControl (rom/graphics/videocontrol.c): the SET / CLR tags write the guest's ColorMap and a GET tag is answered by rewriting
//! its own TagItem (ti_Tag becomes the SET / CLR form). SnoopDos asks for the screen's mode before it builds its window.
mod gfxcommon;
use gfxcommon::*;

const VIDEO_CONTROL: i32 = -708;
const CM: u32 = 0x8B00; // the screen's ColorMap (graphics/layout.rs SCREEN_COLORMAP)
const VB: u32 = 0x8000_0000;

#[test]
fn set_then_get_round_trips_through_the_colormap_and_rewrites_the_tag_item() {
    let mut g = G::new();
    let set = g.tags(&[(VB + 0x21, 0x29000), (VB + 0x2F, 0), (VB + 0x10, 0x1234), (VB + 0x31, 2)]); // VPMODEID_SET, BORDERSPRITE_SET, NORMAL_DISP_SET, SPRITERESN_SET
    let get = g.tags(&[(VB + 0x20, 0), (VB + 0x2E, 0), (VB + 0x0F, 0), (VB + 0x32, 0)]);       // the GET forms
    let imm = g.bytes(&[0xAA; 4]);
    let with_imm = g.tags(&[(VB + 0x37, imm)]);
    g.run(&[gfx(VIDEO_CONTROL, &[a(0, CM), a(1, set)]), gfx(VIDEO_CONTROL, &[a(0, CM), a(1, get)]), gfx(VIDEO_CONTROL, &[a(0, CM), a(1, with_imm)])]);
    assert_eq!((g.result(0), g.result(1), g.result(2)), (0, 0, 0), "{:?}", g.a.refused);
    assert_eq!((g.r32(CM + 36), g.r8(CM + 0) & 0x40, g.r32(CM + 24), g.r8(CM + 17)), (0x29000, 0x40, 0x1234, 2), "stored in the ColorMap fields");
    let item = |i: u32| (g.r32(get + 8 * i), g.r32(get + 8 * i + 4));
    assert_eq!(item(0), (VB + 0x21, 0x29000), "VTAG_VPMODEID_GET -> _SET with the mode");
    assert_eq!(item(1), (VB + 0x2F, 1), "VTAG_BORDERSPRITE_GET -> _SET, TRUE");
    assert_eq!(item(2), (VB + 0x10, 0x1234), "VTAG_NORMAL_DISP_GET -> _SET");
    assert_eq!(item(3), (VB + 0x31, 2), "VTAG_SPRITERESN_GET -> _SET");
    assert_eq!(g.r32(imm), 0, "VTAG_IMMEDIATE's result word is cleared");
}

#[test]
fn clr_tags_clear_and_an_unknown_tag_is_error_one() {
    let mut g = G::new();
    let set = g.tags(&[(VB + 0x2F, 0), (VB + 0x21, 5)]);
    let clr = g.tags(&[(VB + 0x30, 0), (VB + 0x22, 0)]); // BORDERSPRITE_CLR, VPMODEID_CLR
    let get = g.tags(&[(VB + 0x2E, 0), (VB + 0x20, 0)]);
    let bad = g.tags(&[(VB + 0x2F, 0), (TAG_USER + 0x5000, 1), (VB + 0x31, 1)]); // a non-videocontrol tag in the middle: error 1, the walk goes on
    g.run(&[gfx(VIDEO_CONTROL, &[a(0, CM), a(1, set)]), gfx(VIDEO_CONTROL, &[a(0, CM), a(1, clr)]), gfx(VIDEO_CONTROL, &[a(0, CM), a(1, get)]), gfx(VIDEO_CONTROL, &[a(0, CM), a(1, bad)])]);
    assert_eq!((g.r32(get), g.r32(get + 4)), (VB + 0x30, 0), "BORDERSPRITE_GET after CLR answers the CLR form, FALSE");
    assert_eq!((g.r32(get + 8), g.r32(get + 12)), (VB + 0x22, 0xFFFF_FFFF), "VPMODEID_GET after CLR answers _CLR with INVALID_ID");
    assert_eq!(g.result(3), 1, "NEGATIVE CONTROL: a tag that is not a videocontrol tag is error 1");
    assert_eq!(g.r8(CM + 17), 1, "...and the tags after it were still applied");
    assert!(g.a.refused.is_empty(), "{:?}", g.a.refused);
}
