//! DrawImage / DrawImageState of an image object of a GUEST class: Intuition sends IM_DRAW to the object's own dispatcher (rom/intuition/
//! drawimagestate.c) - a guest call, with impDraw = {MethodID, imp_RPort, imp_Offset {X, Y}, imp_State, imp_DrInfo} - and carries on with the
//! chain. tests/guest/c/imgdraw.c is the class; it records what its dispatcher received.
mod common;
use common::*;

#[test]
fn a_guest_image_class_receives_im_draw_with_the_offsets_state_and_drawinfo_of_the_call() {
    let mut a = machine();
    let r = run(&mut a, include_bytes!("guest/imgdraw.bin"));
    assert_eq!(r[11], 1, "the fixture ran to its end");
    assert_ne!(r[9], 0, "the guest image object exists");
    // DrawImage ran first (offset 3,4, state 0, NULL DrawInfo), DrawImageState second and overwrote the record.
    assert_eq!(r[12], 1, "one IM_DRAW after DrawImage");
    assert_eq!(r[6], 2, "two after DrawImageState");
    assert_eq!(r[0], r[10], "imp_RPort is the rastport the program drew into");
    assert_eq!((r[1] >> 16, r[1] & 0xFFFF), (7, 9), "imp_Offset X, Y");
    assert_eq!((r[2], r[3]), (2, 0x1234), "imp_State and imp_DrInfo");
    assert_eq!((r[4], r[5]), (r[9], r[8]), "the dispatcher was called with A2 = the object and A0 = its class");
}
