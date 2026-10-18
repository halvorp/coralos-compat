//! The update region (BeginUpdate/EndUpdate/InvalRect/ValidRect) and publishing window content through
//! `GuestUi::present_content`: one frame per engine step, the damage rectangle, the pixels, the hash.
mod common;
use common::*;
use guestui_abi::{pixel_hash, GuestRect, GuestUi, GuestUiEvent, UiCall};
use mac68k_engine::events::UPDATE_EVT;

const R1: u32 = 0x3100;
const R2: u32 = 0x3108;

fn presents(g: &Rig) -> Vec<(GuestRect, i32, i32, u64)> {
    g.m.ui.calls.iter().filter_map(|c| if let UiCall::PresentContent { damage, width, height, hash, .. } = c { Some((*damage, *width, *height, *hash)) } else { None }).collect()
}
fn rgb(g: &Rig, r: &GuestRect) -> Vec<u32> {
    let c = &g.m.qd.canvases[&g.win];
    (r.top..r.top + r.height).flat_map(|y| (r.left..r.left + r.width).map(move |x| (x, y))).map(|(x, y)| if c.get(x, y) == Some(1) { 0 } else { 0xFF_FFFF }).collect()
}

// ---------------------------------------------------------------- update region

#[test]
fn begin_update_confines_drawing_to_the_invalid_part_and_end_update_restores_the_visrgn() {
    let mut g = Rig::canvas();
    set_rect(&mut g.m, R1, 0, 0, 120, 200); // the whole port
    set_rect(&mut g.m, R2, 10, 10, 30, 50); // the invalid part
    let win = g.win;
    let mut p = Prog::new();
    p.l(R1).trap(0xA92A); // ValidRect(whole)
    p.l(R2).trap(0xA928); // InvalRect(small)
    p.l(win).trap(0xA922); // BeginUpdate
    p.paint_rect(R1);
    p.l(win).trap(0xA923); // EndUpdate
    g.run(p.exit());
    g.no_refusals();
    assert_eq!(g.count_black(), 20 * 40, "only the invalid rectangle was painted\n{}", g.pic(0, 0, 60, 35));
    assert!(g.black(10, 10) && g.black(49, 29) && !g.black(9, 10) && !g.black(50, 29) && !g.black(100, 100));
    // after EndUpdate the visRgn is whole again: ordinary drawing reaches everywhere
    let mut p = Prog::new();
    p.paint_rect(R1);
    g.run(p.exit());
    assert_eq!(g.count_black(), 120 * 200);
    // the update region was consumed by BeginUpdate
    assert!(g.m.qd.update.get(&win).map_or(true, |u| u.is_empty()));
}

#[test]
fn without_begin_update_drawing_is_not_confined_negative_control_and_a_fresh_window_is_all_invalid() {
    let mut g = Rig::canvas();
    set_rect(&mut g.m, R1, 0, 0, 120, 200);
    set_rect(&mut g.m, R2, 10, 10, 30, 50);
    let mut p = Prog::new();
    p.l(R1).trap(0xA92A).l(R2).trap(0xA928).paint_rect(R1);
    g.run(p.exit());
    assert_eq!(g.count_black(), 120 * 200);
    // a fresh window: BeginUpdate with nothing validated lets the program redraw all of it
    let mut g = Rig::canvas();
    set_rect(&mut g.m, R1, 0, 0, 120, 200);
    let win = g.win;
    g.run(Prog::new().l(win).trap(0xA922).paint_rect(R1).l(win).trap(0xA923).exit());
    assert_eq!(g.count_black(), 120 * 200);
}

#[test]
fn inval_rect_posts_one_update_event_and_valid_rect_removes_the_area() {
    let mut g = Rig::canvas();
    set_rect(&mut g.m, R1, 0, 0, 120, 200);
    set_rect(&mut g.m, R2, 10, 10, 30, 50);
    g.run(Prog::new().l(R1).trap(0xA92A).l(R2).trap(0xA928).l(R2).trap(0xA928).exit());
    let n = g.m.ev.queue.iter().filter(|e| e.what == UPDATE_EVT && e.message == g.win).count();
    assert_eq!(n, 1, "two InvalRects, one updateEvt");
    assert_eq!(g.m.qd.update[&g.win].bounds(), retro_raster::Rect::new(10, 10, 50, 30));
    // ValidRect over it removes it (negative control for the above)
    let mut g = Rig::canvas();
    set_rect(&mut g.m, R1, 0, 0, 120, 200);
    g.run(Prog::new().l(R1).trap(0xA92A).exit());
    assert!(g.m.qd.update[&g.win].is_empty());
}

#[test]
fn end_update_without_begin_update_is_a_named_refusal() {
    let mut g = Rig::canvas();
    let win = g.win;
    g.m.start(&Prog::new().l(win).trap(0xA923).exit().0, 0).unwrap();
    g.m.run_loop(100_000).unwrap();
    assert!(g.m.refused.iter().any(|r| r.contains("EndUpdate") && r.contains("no matching BeginUpdate")), "{:?}", g.m.refused);
}

// ---------------------------------------------------------------- present_content

#[test]
fn many_primitives_in_one_step_are_one_frame_whose_pixels_and_hash_match_the_canvas() {
    let mut g = Rig::canvas();
    set_rect(&mut g.m, R1, 10, 20, 30, 60);
    set_rect(&mut g.m, R2, 50, 80, 70, 120);
    put(&mut g.m, 0x3600, &pstr("QD"));
    let mut p = Prog::new();
    p.paint_rect(R1).frame_rect(R2).rect_op(0xA8B8, R2).move_to(5, 100).draw_string(0x3600);
    g.run(p.exit());
    g.no_refusals();
    let frames = presents(&g);
    assert_eq!(frames.len(), 1, "one present_content for the whole step: {:?}", g.m.ui.calls);
    let (damage, w, h, hash) = frames[0];
    assert_eq!((w, h), (WIN_W, WIN_H));
    assert_eq!(damage, GuestRect { left: 0, top: 0, width: WIN_W, height: WIN_H }, "the first frame is the whole canvas");
    assert_eq!(hash, pixel_hash(&rgb(&g, &damage)), "the frame carries the canvas's resolved pixels");
    let native = g.m.windows[&g.win].native.unwrap();
    let shown = &g.m.ui.canvases[&native.id];
    assert_eq!(shown.2, rgb(&g, &damage), "what the host shows is what the engine drew");
    assert_eq!(shown.2.iter().filter(|&&p| p == 0).count(), g.count_black());
    assert!(g.m.log.iter().any(|l| l.contains("[quickdraw] content window") && l.contains("200x120")), "{:?}", g.m.log);
}

#[test]
fn later_steps_present_only_the_damaged_rectangle_and_idle_steps_present_nothing() {
    let mut g = Rig::canvas();
    set_rect(&mut g.m, R1, 10, 20, 30, 60);
    g.run(Prog::new().paint_rect(R1).exit());
    assert_eq!(presents(&g).len(), 1);
    // idle step: a program that draws nothing -> no new frame
    g.run(Prog::new().exit());
    assert_eq!(presents(&g).len(), 1, "negative control: no drawing, no frame");
    // two small rectangles far apart -> ONE frame covering their union, and only that
    set_rect(&mut g.m, R1, 5, 5, 8, 9);
    set_rect(&mut g.m, R2, 90, 150, 95, 160);
    g.run(Prog::new().paint_rect(R1).paint_rect(R2).exit());
    let frames = presents(&g);
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[1].0, GuestRect { left: 5, top: 5, width: 155, height: 90 }, "union of the two rects: left 5..160, top 5..95");
    assert_eq!(frames[1].3, pixel_hash(&rgb(&g, &frames[1].0)));
    assert_ne!(frames[1].3, frames[0].3);
    // the host's composed canvas equals the engine's canvas pixel for pixel
    let native = g.m.windows[&g.win].native.unwrap();
    assert_eq!(g.m.ui.canvases[&native.id].2, rgb(&g, &GuestRect { left: 0, top: 0, width: WIN_W, height: WIN_H }));
}

#[test]
fn drawing_while_invisible_shows_as_the_first_frame_when_the_window_opens() {
    let mut g = Rig::canvas_with(false);
    set_rect(&mut g.m, R1, 10, 20, 30, 60);
    g.run(Prog::new().paint_rect(R1).exit());
    assert!(presents(&g).is_empty(), "an invisible window opens nothing and presents nothing");
    let win = g.win;
    g.run(Prog::new().l(win).trap(0xA915).exit()); // ShowWindow
    let frames = presents(&g);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].0.width, WIN_W, "the first frame is the whole canvas, including what was drawn before");
    let native = g.m.windows[&g.win].native.unwrap();
    assert_eq!(g.m.ui.canvases[&native.id].2.iter().filter(|&&p| p == 0).count(), 20 * 40);
}

#[test]
fn a_native_resize_makes_a_new_blank_canvas_a_full_update_and_a_full_frame() {
    let mut g = Rig::canvas();
    set_rect(&mut g.m, R1, 10, 20, 30, 60);
    g.run(Prog::new().paint_rect(R1).exit());
    let native = g.m.windows[&g.win].native.unwrap();
    g.m.deliver(GuestUiEvent::Resized { win: native, rect: GuestRect { left: 50, top: 100, width: 300, height: 150 } }).unwrap();
    assert_eq!(g.count_black(), 0, "the resized canvas is blank");
    assert_eq!(g.m.qd.canvases[&g.win].width(), 300);
    assert!(g.m.ev.queue.iter().any(|e| e.what == UPDATE_EVT && e.message == g.win));
    assert_eq!(g.m.qd.update[&g.win].bounds(), retro_raster::Rect::new(0, 0, 300, 150), "the whole window is invalid");
    g.run(Prog::new().paint_rect(R1).exit());
    let frames = presents(&g);
    assert_eq!((frames[1].0.width, frames[1].0.height, frames[1].1), (300, 150, 300), "full first frame at the new size");
}

#[test]
fn a_host_without_a_canvas_part_is_logged_once_and_is_not_a_refusal_storm() {
    struct NoCanvas(guestui_abi::RecordingGuestUi);
    impl GuestUi for NoCanvas {
        fn open_window(&mut self, m: guestui_abi::WindowModel) -> Result<guestui_abi::UiObjectRef, guestui_abi::UiRefusal> { self.0.open_window(m) }
        fn close_window(&mut self, w: guestui_abi::UiObjectRef) -> Result<(), guestui_abi::UiRefusal> { self.0.close_window(w) }
        fn set_window_title(&mut self, w: guestui_abi::UiObjectRef, t: String) -> Result<(), guestui_abi::UiRefusal> { self.0.set_window_title(w, t) }
        fn set_menu_strip(&mut self, w: guestui_abi::UiObjectRef, m: guestui_abi::MenuModel) -> Result<(), guestui_abi::UiRefusal> { self.0.set_menu_strip(w, m) }
        fn clear_menu_strip(&mut self, w: guestui_abi::UiObjectRef) -> Result<(), guestui_abi::UiRefusal> { self.0.clear_menu_strip(w) }
        fn add_control(&mut self, w: guestui_abi::UiObjectRef, c: guestui_abi::ControlModel) -> Result<guestui_abi::UiObjectRef, guestui_abi::UiRefusal> { self.0.add_control(w, c) }
        fn remove_control(&mut self, w: guestui_abi::UiObjectRef, c: guestui_abi::UiObjectRef) -> Result<(), guestui_abi::UiRefusal> { self.0.remove_control(w, c) }
        fn set_control(&mut self, w: guestui_abi::UiObjectRef, c: guestui_abi::UiObjectRef, v: guestui_abi::ControlValue) -> Result<(), guestui_abi::UiRefusal> { self.0.set_control(w, c, v) }
        fn poll_event(&mut self) -> Option<GuestUiEvent> { None }
    }
    let mut m = mac68k_engine::Mac::new(NoCanvas(guestui_abi::RecordingGuestUi::new())).unwrap();
    mac68k_engine::quickdraw::fill_globals(&mut m, GLOBALS).unwrap();
    m.qd_globals = GLOBALS;
    let spec = mac68k_engine::toolbox::WindowSpec { storage: 0, rect: mac68k_engine::pascal::Rect { top: 100, left: 50, bottom: 200, right: 150 }, title: "n".into(), visible: true, proc_id: 0, behind: 0xFFFF_FFFF, go_away: false, ref_con: 0 };
    let win = m.make_window(spec).unwrap();
    for _ in 0..3 {
        m.mem.write_bytes(0x3100, &[0, 0, 0, 0, 0, 10, 0, 10]).unwrap();
        m.start(&Prog::new().paint_rect(0x3100).exit().0, 0).unwrap();
        m.run_loop(100_000).unwrap();
    }
    assert!(m.refused.is_empty(), "{:?}", m.refused);
    assert_eq!(m.log.iter().filter(|l| l.contains("content window") && l.contains("not shown")).count(), 1, "logged once: {:?}", m.log);
    assert!(m.qd.canvases[&win].pixels().iter().any(|&p| p == 1), "drawing itself still works");
}
