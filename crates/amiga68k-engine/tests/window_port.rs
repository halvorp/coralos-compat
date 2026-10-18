//! Intuition's messages name the window's WindowPort as their reply port (AROS: `mn_ReplyPort = w->WindowPort`), and exec's CloseLibrary
//! accepts NULL. DOpus's ReplyFreeMsg(msg) is `ReplyMsg` when mn_ReplyPort is set and `FreeVec(msg)` when it is NULL: a NULL port made it free
//! a block inside the engine's heap bookkeeping (a named refusal) instead of replying.
mod gfxcommon;
use gfxcommon::*;

const EXEC: u32 = 0x1000;
const CLOSE_LIBRARY: i32 = -414;
const GET_MSG: i32 = -372;

#[test]
fn a_window_has_a_window_port_and_its_messages_name_it_as_reply_port() {
    let mut g = G::new();
    let nw = g.zeroed(48);
    for (o, v) in [(0u32, 0u16), (2, 0), (4, 80), (6, 40)] { g.a.mem.write_u16(nw + o, v).unwrap(); }
    g.a.mem.write_u32(nw + 10, 0x40000).unwrap(); // IDCMP_ACTIVEWINDOW
    g.a.mem.write_u32(nw + 14, 0x1000).unwrap(); // WFLG_ACTIVATE: Intuition tells the new window it is active
    g.run(&[intu(OPEN_WINDOW, &[a(0, nw)])]);
    let win = g.result(0);
    let (user, wport) = (g.r32(win + 86), g.r32(win + 90));
    assert_ne!(user, 0);
    assert_ne!(wport, 0, "Window.WindowPort exists");
    assert_ne!(wport, user, "control: it is not the UserPort");
    g.run(&[(EXEC, GET_MSG, vec![a(0, user)])]);
    let msg = g.result(0);
    assert_ne!(msg, 0, "the activation message");
    assert_eq!(g.r32(msg + 14), wport, "mn_ReplyPort is the WindowPort");
}

#[test]
fn close_library_null_is_a_no_op_and_a_stray_base_is_still_refused_by_name() {
    let mut g = G::new();
    g.run(&[(EXEC, CLOSE_LIBRARY, vec![a(1, 0)])]);
    assert!(g.a.refused.is_empty(), "{:?}", g.a.refused);
    g.run(&[(EXEC, CLOSE_LIBRARY, vec![a(1, 0x123456)])]);
    assert!(g.a.refused.iter().any(|r| r.contains("0x123456 is not an open library base")), "control: a base that is not a library is still a named refusal: {:?}", g.a.refused);
}
