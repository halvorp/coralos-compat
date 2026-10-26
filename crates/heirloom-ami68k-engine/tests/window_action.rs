//! intuition.library WindowAction: WAC_SENDIDCMPCLOSE posts IDCMP_CLOSEWINDOW to a window that subscribes to it
//! (rom/intuition/windowaction.c:51-62 -> inputhandler_support.c fire_message:195 `(w->IDCMPFlags & Class) && w->UserPort`); every other
//! action is refused by name. MultiView closes its window this way.
mod gfxcommon;
use gfxcommon::*;

const WINDOW_ACTION: i32 = -942;
const GET_MSG: i32 = -372;
const EXEC: u32 = 0x1000;
const WAC_SENDIDCMPCLOSE: u32 = 3;
const IDCMP_CLOSEWINDOW: u32 = 0x200;

fn open(g: &mut G, idcmp: u32) -> u32 {
    let nw = g.zeroed(48);
    for (o, v) in [(4u32, 80u16), (6, 40)] { g.a.mem.write_u16(nw + o, v).unwrap(); }
    g.a.mem.write_u32(nw + 10, idcmp).unwrap();
    g.run(&[intu(OPEN_WINDOW, &[a(0, nw)])]);
    g.result(0)
}

#[test]
fn sendidcmpclose_posts_closewindow_to_a_subscriber_only() {
    let mut g = G::new();
    let (sub, deaf) = (open(&mut g, IDCMP_CLOSEWINDOW), open(&mut g, 0x40)); // 0x40 = IDCMP_MOUSEBUTTONS
    g.run(&[intu(WINDOW_ACTION, &[a(0, sub), d(0, WAC_SENDIDCMPCLOSE), a(1, 0)]),
            intu(WINDOW_ACTION, &[a(0, deaf), d(0, WAC_SENDIDCMPCLOSE), a(1, 0)])]);
    let (up, deaf_up) = (g.r32(sub + 86), g.r32(deaf + 86));
    g.run(&[(EXEC, GET_MSG, vec![a(0, up)]), (EXEC, GET_MSG, vec![a(0, deaf_up)])]);
    let msg = g.result(0);
    assert_ne!(msg, 0, "the subscriber got a message");
    assert_eq!((g.r32(msg + 20), g.r16(msg + 24)), (IDCMP_CLOSEWINDOW, 0), "Class CLOSEWINDOW, Code 0");
    assert_eq!(g.r32(msg + 28), 0, "IAddress NULL, as windowaction.c passes");
    // NEGATIVE CONTROL: a window that did not ask for CLOSEWINDOW is not sent one.
    assert_eq!(g.result(1), 0, "no message for the unsubscribed window");
    assert!(g.a.refused.is_empty(), "{:?}", g.a.refused);
}

#[test]
fn every_other_action_is_refused_by_name() {
    let mut g = G::new();
    let w = open(&mut g, IDCMP_CLOSEWINDOW);
    g.a.start_hunk(&script(&[intu(WINDOW_ACTION, &[a(0, w), d(0, 1), a(1, 0)])])).unwrap(); // WAC_HIDEWINDOW
    g.a.step(1_000_000);
    assert!(g.a.refused.iter().any(|r| r.contains("WindowAction") && r.contains("action 1 is not implemented")), "{:?}", g.a.refused);
    let up = g.r32(w + 86);
    g.run(&[(EXEC, GET_MSG, vec![a(0, up)])]);
    assert_eq!(g.result(0), 0, "nothing was posted");
}

const START_SCREEN_NOTIFY: i32 = -924;
const END_SCREEN_NOTIFY: i32 = -930;
const SNA_NOTIFY: u32 = 0x8000_0002;

/// MultiView's Cleanup is `while (!EndScreenNotify(isnstarted)) Delay(10);`: the vector at -930 IS EndScreenNotify (155 * 6 in
/// rom/intuition/endscreennotify.c; the engine's table once named it WindowAction, so the loop never ended). NULL is TRUE, a token from
/// StartScreenNotifyTagList is given back once, and giving it back twice is a named refusal.
#[test]
fn screen_notify_tokens_are_registered_and_given_back_once() {
    let mut g = G::new();
    let tags = g.tags(&[(SNA_NOTIFY, 0x7)]);
    g.run(&[intu(START_SCREEN_NOTIFY, &[a(0, tags)]), intu(END_SCREEN_NOTIFY, &[a(0, 0)]), intu(END_SCREEN_NOTIFY, &[a_res(0, 0)])]);
    assert_ne!(g.result(0), 0, "a token");
    assert_eq!((g.result(1), g.result(2)), (1, 1), "End(NULL) and End(token) are TRUE");
    assert!(g.a.refused.is_empty(), "{:?}", g.a.refused);
    let tok = g.result(0);
    // NEGATIVE CONTROL: the second End of the same token, and a token nobody issued, are refused by name.
    g.a.start_hunk(&script(&[intu(END_SCREEN_NOTIFY, &[a(0, tok)]), intu(END_SCREEN_NOTIFY, &[a(0, 0x1234)])])).unwrap();
    g.a.step(1_000_000);
    assert_eq!(g.a.refused.iter().filter(|r| r.contains("not an outstanding StartScreenNotifyTagList token")).count(), 2, "{:?}", g.a.refused);
}
