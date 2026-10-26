//! intuition.library Request / EndRequest / InitRequester (rom/intuition/request.c, endrequest.c, requesters.c): a Requester is a rectangle of a
//! window with its own gadgets (native controls, positioned relative to the requester), BackFill and ReqText; EndRequest takes it away and posts
//! IDCMP_REQCLEAR. DOpus asks for a 0 x 0 requester to block its window: that is FALSE, as AROS answers.
mod gfxcommon;
use gfxcommon::*;
use heirloom_ui_abi::*;

const REQUEST: i32 = -240;
const END_REQUEST: i32 = -120;
const INIT_REQUESTER: i32 = -138;
const OPEN_WINDOW_TAG_LIST: i32 = -606;
const REQCLEAR: u32 = 0x1000;
const REQACTIVE: u16 = 1 << 13;
const POINTREL: u16 = 1;
const WD_FIRST_REQUEST: u32 = 36;
const WD_REQ_COUNT: u32 = 44;
const WD_BORDER: u32 = 54;
const WD_USERPORT: u32 = 86;
const WD_RPORT: u32 = 50;

struct Fx { g: G, win: u32, req: u32, gad: u32 }

fn requester(g: &mut G, w: u16, h: u16, flags: u16, left: u16, top: u16, gad: u32, text: u32) -> u32 {
    let mut r = vec![0u8; 112];
    for (o, v) in [(4usize, left), (6, top), (8, w), (10, h), (28, flags)] { r[o..o + 2].copy_from_slice(&v.to_be_bytes()); }
    r[16..20].copy_from_slice(&gad.to_be_bytes());
    r[24..28].copy_from_slice(&text.to_be_bytes());
    r[30] = 3; // BackFill pen
    g.bytes(&r)
}

fn fixture(flags: u16, w: u16, h: u16) -> Fx {
    let mut g = G::new();
    let (title, ok, sure) = (g.s("Host"), g.s("OK"), g.s("Sure?"));
    let win_tags = g.tags(&[(WA + 7, REQCLEAR), (WA + 3, 320), (WA + 4, 200), (WA + 11, title)]);
    g.run(&[intu(OPEN_WINDOW_TAG_LIST, &[a(0, 0), a(1, win_tags)])]);
    let win = g.result(0);
    let mut it = vec![0u8; 20]; // IntuiText "OK" for the gadget, "Sure?" for the requester
    it[12..16].copy_from_slice(&ok.to_be_bytes());
    let gt = g.bytes(&it);
    let mut it2 = vec![0u8; 20];
    it2[4..6].copy_from_slice(&4u16.to_be_bytes()); it2[6..8].copy_from_slice(&4u16.to_be_bytes()); it2[12..16].copy_from_slice(&sure.to_be_bytes());
    let text = g.bytes(&it2);
    let mut gd = vec![0u8; 44];
    for (off, v) in [(4usize, 5u16), (6, 30), (8, 40), (10, 12), (14, 1), (16, 1), (38, 9)] { gd[off..off + 2].copy_from_slice(&v.to_be_bytes()); }
    gd[26..30].copy_from_slice(&gt.to_be_bytes());
    let gad = g.bytes(&gd);
    let req = requester(&mut g, w, h, flags, 10, 8, gad, text);
    Fx { g, win, req, gad }
}

fn borders(f: &Fx) -> (i32, i32) { (f.g.r8(f.win + WD_BORDER) as i32, f.g.r8(f.win + WD_BORDER + 1) as i32) }

fn messages(f: &Fx) -> Vec<(u32, u32)> {
    let m = &f.g.a.mem;
    let port = m.read_u32(f.win + WD_USERPORT).unwrap();
    let (mut out, mut n) = (vec![], m.read_u32(port + 20).unwrap());
    while m.read_u32(n).unwrap() != 0 { out.push((m.read_u32(n + 20).unwrap(), m.read_u32(n + 28).unwrap())); n = m.read_u32(n).unwrap(); }
    out
}

#[test]
fn a_requester_links_into_the_window_and_its_gadget_is_a_control_inside_it() {
    let mut f = fixture(0, 100, 50);
    let (bl, bt) = borders(&f);
    f.g.run(&[intu(REQUEST, &[a(0, f.req), a(1, f.win)])]);
    assert_eq!(f.g.result(0), 1, "{:?}", f.g.a.refused);
    assert_eq!((f.g.r32(f.win + WD_FIRST_REQUEST), f.g.r16(f.win + WD_REQ_COUNT)), (f.req, 1), "Window.FirstRequest, ReqCount");
    assert_eq!(f.g.r16(f.req + 28) & REQACTIVE, REQACTIVE, "REQACTIVE");
    assert_eq!((f.g.r32(f.req + 72), f.g.r16(f.req + 4), f.g.r16(f.req + 6)), (f.win, 10, 8), "RWindow; LeftEdge / TopEdge are inner-area relative");
    assert_ne!(f.g.r32(f.req + 32), 0, "ReqLayer is non-NULL while it is up");
    let c: Vec<_> = f.g.a.ui.controls.values().map(|(_, m)| m.clone()).collect();
    assert_eq!(c.len(), 1, "{c:?}");
    assert_eq!((&c[0].kind, c[0].label.as_str(), c[0].rect), (&ControlKind::Button, "OK", GuestRect { left: bl + 10 + 5, top: bt + 8 + 30, width: 40, height: 12 }), "the gadget sits inside the requester");
    let rp = f.g.r32(f.win + WD_RPORT);
    f.g.run(&[gfx(READ_PIXEL, &[a(1, rp), di(0, bl + 10 + 80), di(1, bt + 8 + 40)]), gfx(READ_PIXEL, &[a(1, rp), di(0, bl + 10 + 120), di(1, bt + 8 + 40)])]);
    assert_eq!(f.g.result(0), 3, "BackFill pen inside the requester");
    assert_ne!(f.g.result(1), 3, "NEGATIVE CONTROL: outside the requester the window is untouched");
}

#[test]
fn end_request_unlinks_removes_the_gadget_and_posts_reqclear() {
    let mut f = fixture(0, 100, 50);
    f.g.run(&[intu(REQUEST, &[a(0, f.req), a(1, f.win)])]);
    f.g.run(&[intu(END_REQUEST, &[a(0, f.req), a(1, f.win)]), intu(END_REQUEST, &[a(0, f.req), a(1, f.win)])]);
    assert_eq!((f.g.r32(f.win + WD_FIRST_REQUEST), f.g.r16(f.win + WD_REQ_COUNT), f.g.r16(f.req + 28) & REQACTIVE, f.g.r32(f.req + 32)), (0, 0, 0, 0));
    assert_eq!(f.g.r32(f.req + 16), f.gad, "ReqGadget is the program's again");
    assert!(f.g.a.ui.controls.is_empty(), "the gadget's control left with the requester");
    assert_eq!(messages(&f), [(REQCLEAR, f.win)], "one IDCMP_REQCLEAR with IAddress = the window; the second EndRequest is ignored (not a requester of the window)");
    // The requester can be opened again.
    f.g.run(&[intu(REQUEST, &[a(0, f.req), a(1, f.win)])]);
    assert_eq!(f.g.result(0), 1, "{:?}", f.g.a.refused);
    assert_eq!(f.g.a.ui.controls.len(), 1);
}

#[test]
fn an_empty_requester_is_false_and_a_stranger_window_is_refused_by_name() {
    let mut f = fixture(0, 0, 0);
    f.g.run(&[intu(REQUEST, &[a(0, f.req), a(1, f.win)])]);
    assert_eq!(f.g.result(0), 0, "a 0 x 0 requester does not open (request.c: right < left)");
    assert_eq!((f.g.r32(f.win + WD_FIRST_REQUEST), f.g.r16(f.win + WD_REQ_COUNT), f.g.r32(f.req + 32)), (0, 0, 0));
    assert!(f.g.a.ui.controls.is_empty() && f.g.a.refused.is_empty(), "{:?}", f.g.a.refused);
    f.g.a.start_hunk(&script(&[intu(REQUEST, &[a(0, f.req), a(1, 0x1234)])])).unwrap();
    f.g.a.step(1_000_000);
    assert!(f.g.a.refused.iter().any(|r| r.contains("Request: 0x1234 is not an open window")), "{:?}", f.g.a.refused);
}

#[test]
fn pointrel_centres_the_requester_and_flags_that_need_a_bitmap_are_refused() {
    let mut f = fixture(POINTREL, 100, 50);
    let (bl, bt) = borders(&f);
    f.g.run(&[intu(REQUEST, &[a(0, f.req), a(1, f.win)])]);
    assert_eq!(f.g.result(0), 1, "{:?}", f.g.a.refused);
    let (ww, wh) = (f.g.r16(f.win + 8) as i32, f.g.r16(f.win + 10) as i32);
    let (br, bb) = (f.g.r8(f.win + WD_BORDER + 2) as i32, f.g.r8(f.win + WD_BORDER + 3) as i32);
    // left - wleft = (wleft + wright - Width) / 2 + RelLeft - wleft  (RelLeft = 0 here; wright - wleft + 1 = the inner width)
    let inner = (ww - bl - br, wh - bt - bb);
    assert_eq!((f.g.r16(f.req + 4) as i32, f.g.r16(f.req + 6) as i32), ((inner.0 - 1 - 100) / 2, (inner.1 - 1 - 50) / 2), "centred in the inner area");
    let mut g2 = fixture(2, 100, 50); // PREDRAWN
    g2.g.a.start_hunk(&script(&[intu(REQUEST, &[a(0, g2.req), a(1, g2.win)])])).unwrap();
    g2.g.a.step(1_000_000);
    assert!(g2.g.a.refused.iter().any(|r| r.contains("PREDRAWN")), "{:?}", g2.g.a.refused);
    // InitRequester zeroes the whole 112-byte structure (the requester above has size, flags and a gadget).
    f.g.run(&[intu(INIT_REQUESTER, &[a(0, f.req)])]);
    assert!((0..112).step_by(2).all(|o| f.g.r16(f.req + o) == 0), "InitRequester clears every field");
}
