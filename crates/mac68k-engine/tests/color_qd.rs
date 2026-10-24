//! Color QuickDraw on the Macintosh II profile (src/cqd): what the machine says about itself, the structures a colour program
//! builds, and drawing into 8-bit pixels. Colour-table values are checked against the documented standard 8-bit table (the
//! cube of FF/CC/99/66/33/00 with white first, the ramps, black last) because no corpus program shows them.
mod common;
use common::*;
use guestui_abi::{DrawOp, RecordingGuestUi, UiCall};
use mac68k_engine::cqd::color::clut8;
use mac68k_engine::fixture::{ScriptArg::Lit, ScriptCall};
use mac68k_engine::pascal::Rect;
use mac68k_engine::toolbox::WindowSpec;
use mac68k_engine::Mac;

const R1: u32 = 0x3100;
const RGB_A: u32 = 0x3200; // scratch colours
const RGB_B: u32 = 0x3208;
const OUT: u32 = 0x3300;

fn call(name: &'static str, args: &[mac68k_engine::fixture::ScriptArg], save: Option<u32>) -> ScriptCall { ScriptCall { name, args: args.to_vec(), save } }
fn rgb(m: &mut Mac<RecordingGuestUi>, at: u32, c: (u16, u16, u16)) { for (i, v) in [c.0, c.1, c.2].into_iter().enumerate() { m.mem.write_u16(at + 2 * i as u32, v).unwrap(); } }
fn read_rgb(m: &Mac<RecordingGuestUi>, at: u32) -> (u16, u16, u16) { (m.mem.read_u16(at).unwrap(), m.mem.read_u16(at + 2).unwrap(), m.mem.read_u16(at + 4).unwrap()) }
fn run(m: &mut Mac<RecordingGuestUi>, p: &Prog) {
    m.start(&p.0, 0).unwrap();
    let r = m.run_loop(5_000_000).unwrap();
    assert!(r.stats.halted && r.stats.bus_faults == 0, "{:?}\n{:#?}", r.stats, m.log);
}

/// Run scripted Toolbox calls (by name) to ExitToShell.
fn script(m: &mut Mac<RecordingGuestUi>, calls: &[ScriptCall]) {
    let mut p = Prog::new();
    p.script(m, calls).exit();
    run(m, &p);
}

/// A machine of `profile` after `InitGraf`.
fn machine(profile: &str) -> Mac<RecordingGuestUi> {
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    m.set_machine(profile).unwrap();
    let mut p = Prog::new();
    p.l(GLOBALS).trap(0xA86E).exit();
    run(&mut m, &p);
    m
}

/// A visible colour window (global 100,50 .. 220,250) on a mac-ii.
struct Win { m: Mac<RecordingGuestUi>, win: u32 }
fn color_window() -> Win {
    let mut m = machine("mac-ii");
    let spec = WindowSpec { storage: 0, rect: Rect { top: 100, left: 50, bottom: 220, right: 250 }, title: "C".into(), visible: true, proc_id: 0, behind: 0xFFFF_FFFF, go_away: false, ref_con: 0 };
    let win = m.make_window_as(spec, true).unwrap();
    m.ev.queue.clear();
    Win { m, win }
}
impl Win {
    fn px(&self, x: i32, y: i32) -> u8 { self.m.qd.canvases[&self.win].get(x, y).unwrap() }
    fn script(&mut self, calls: &[ScriptCall]) {
        let mut p = Prog::new();
        p.script(&self.m, calls).exit();
        run(&mut self.m, &p);
    }
}

// ---- 1. what the machine says -------------------------------------------------------------------------------------------------

fn gestalt(m: &mut Mac<RecordingGuestUi>, sel: &[u8; 4]) -> u32 {
    let sel = u32::from_be_bytes(*sel);
    let prog = [0x20, 0x3C, (sel >> 24) as u8, (sel >> 16) as u8, (sel >> 8) as u8, sel as u8, 0xA1, 0xAD, 0x23, 0xC8, 0x00, 0x00, 0x30, 0x00, 0xA9, 0xF4];
    m.start(&prog, 0).unwrap();
    m.run_loop(10_000).unwrap();
    m.mem.read_u32(0x3000).unwrap()
}
/// SysEnvirons(2, &rec): hasColorQD, the byte at offset 9.
fn has_color_qd(m: &mut Mac<RecordingGuestUi>) -> u8 {
    let prog = [0x70, 0x02, 0x20, 0x7C, 0x00, 0x00, 0x33, 0x00, 0xA0, 0x90, 0xA9, 0xF4];
    m.start(&prog, 0).unwrap();
    m.run_loop(10_000).unwrap();
    m.mem.read_u8(0x3309).unwrap()
}

#[test]
fn only_the_mac_ii_reports_color_quickdraw_and_the_three_answers_agree() {
    for (profile, color, rom85) in [("mac-ii", true, 0x3FFF), ("mac-plus", false, 0x7FFF), ("mac-se", false, 0x3FFF)] {
        let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
        m.set_machine(profile).unwrap();
        assert_eq!(has_color_qd(&mut m), u8::from(color), "{profile}: SysEnvirons hasColorQD");
        assert_eq!(gestalt(&mut m, b"qd  "), if color { 0x100 } else { 0 }, "{profile}: gestaltQuickdrawVersion");
        assert_eq!(gestalt(&mut m, b"qdrw"), u32::from(color), "{profile}: gestaltQuickdrawFeatures bit 0 (hasColor)");
        assert_eq!(m.mem.read_u16(0x28E).unwrap(), rom85, "{profile}: ROM85");
    }
}

#[test]
fn the_default_machine_is_the_mac_ii_and_has_color_quickdraw() {
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    assert_eq!((has_color_qd(&mut m), m.mem.read_u16(0x28E).unwrap()), (1, 0x3FFF));
}

// ---- 2. the structures --------------------------------------------------------------------------------------------------------

#[test]
fn the_main_device_is_an_8_bit_indexed_screen_with_the_standard_table() {
    let mut m = machine("mac-ii");
    script(&mut m, &[call("GetMainDevice", &[], Some(0x3400)), call("GetGDevice", &[], Some(0x3404)), call("GetDeviceList", &[], Some(0x3408))]);
    let gd = m.mem.read_u32(0x3400).unwrap();
    assert!(gd != 0 && m.mem.read_u32(0x3404).unwrap() == gd && m.mem.read_u32(0x3408).unwrap() == gd, "main = current = the whole list");
    let gp = m.mem.read_u32(gd).unwrap();
    assert_eq!(m.mem.read_u16(gp + 20).unwrap(), 0xBC01, "gdFlags: color, ramInit, mainScreen, allInit, screenDevice, screenActive");
    assert_eq!(m.mem.read_u32(gp + 30).unwrap(), 0, "gdNextGD (offset 30: gdPMap is a 4-byte handle at 22, gdRefCon a LONGINT at 26): the only device");
    // gdRect is at +34 (Inside Macintosh sums the record to 62 bytes only this way; systemless writes gdRect.top at gd+34)
    let (pmh, rect) = (m.mem.read_u32(gp + 22).unwrap(), gp + 34);
    assert_eq!([0, 2, 4, 6].map(|o| m.mem.read_u16(rect + o).unwrap()), [0, 0, 342, 512], "gdRect = the screen");
    let pm = m.mem.read_u32(pmh).unwrap();
    assert_eq!((m.mem.read_u16(pm + 4).unwrap(), m.mem.read_u16(pm + 32).unwrap(), m.mem.read_u16(pm + 34).unwrap(), m.mem.read_u16(pm + 36).unwrap()), (0x8000 | 512, 8, 1, 8), "PixMap: rowBytes with the PixMap flag, pixelSize 8, one 8-bit component");
    assert_eq!(m.mem.read_u32(pm + 22).unwrap(), 0x0048_0000, "hRes 72 dpi");
    let ct = m.mem.read_u32(m.mem.read_u32(pm + 42).unwrap()).unwrap();
    assert_eq!((m.mem.read_u32(ct).unwrap(), m.mem.read_u16(ct + 4).unwrap(), m.mem.read_u16(ct + 6).unwrap()), (8, 0x8000, 255), "ctSeed 8, device table, 256 entries");
    // the lowmem globals say the same
    assert_eq!((m.mem.read_u32(0x8A4).unwrap(), m.mem.read_u32(0xCC8).unwrap()), (gd, gd));
}

#[test]
fn a_machine_without_color_quickdraw_has_no_devices_and_refuses_by_name() {
    let mut m = machine("mac-plus");
    assert_eq!(m.mem.read_u32(0x8A4).unwrap(), 0, "no main device");
    let mut p = Prog::new();
    p.script(&m, &[call("GetMainDevice", &[], Some(0x3400))]).exit();
    run(&mut m, &p);
    assert!(m.refused.iter().any(|r| r.contains("GetMainDevice") && r.contains("mac-plus")), "{:?}", m.refused);
}

#[test]
fn device_attributes_and_set_gdevice() {
    let mut m = machine("mac-ii");
    let gd = m.mem.read_u32(0x8A4).unwrap();
    script(&mut m, &[
        call("TestDeviceAttribute", &[Lit(gd), Lit(11)], Some(0x3410)), // mainScreen
        call("TestDeviceAttribute", &[Lit(gd), Lit(14)], Some(0x3412)), // noDriver
        call("NewGDevice", &[Lit(0xFFFF), Lit(0)], Some(0x3414)),
    ]);
    assert_eq!((m.mem.read_u8(0x3410).unwrap(), m.mem.read_u8(0x3412).unwrap()), (1, 0));
    let off = m.mem.read_u32(0x3414).unwrap();
    assert_ne!(off, 0);
    script(&mut m, &[call("SetGDevice", &[Lit(off)], None), call("GetGDevice", &[], Some(0x3418)), call("TestDeviceAttribute", &[Lit(off), Lit(14)], Some(0x341C))]);
    assert_eq!(m.mem.read_u32(0x3418).unwrap(), off, "GetGDevice returns what SetGDevice set");
    assert_eq!(m.mem.read_u8(0x341C).unwrap(), 1, "a device made with refNum -1 has noDriver");
    assert_eq!(m.mem.read_u32(0x8A4).unwrap(), gd, "the main device is unchanged");
}

#[test]
fn the_color_manager_works_on_the_current_devices_table() {
    let mut m = machine("mac-ii");
    rgb(&mut m, RGB_A, (0xFFFF, 0xCCCC, 0x9999)); // cube entry 0*36 + 1*6 + 2 = 8
    rgb(&mut m, RGB_B, (0x1234, 0x5678, 0x9ABC)); // in the table: no
    script(&mut m, &[
        call("Color2Index", &[Lit(RGB_A)], Some(0x3400)),
        call("RealColor", &[Lit(RGB_A)], Some(0x3404)),
        call("RealColor", &[Lit(RGB_B)], Some(0x3406)),
        call("Index2Color", &[Lit(255), Lit(OUT)], None),
        call("Index2Color", &[Lit(8), Lit(OUT + 8)], None),
        call("InvertColor", &[Lit(RGB_B)], None),
    ]);
    assert_eq!(m.mem.read_u32(0x3400).unwrap(), 8);
    assert_eq!((m.mem.read_u8(0x3404).unwrap(), m.mem.read_u8(0x3406).unwrap()), (1, 0), "RealColor: an exact entry, and not");
    assert_eq!(read_rgb(&m, OUT), (0, 0, 0), "index 255 is black");
    assert_eq!(read_rgb(&m, OUT + 8), (0xFFFF, 0xCCCC, 0x9999));
    assert_eq!(read_rgb(&m, RGB_B), (!0x1234, !0x5678, !0x9ABC), "InvertColor complements each component");
    // a colour that is not in the table gets the nearest entry
    rgb(&mut m, RGB_B, (0xFE00, 0xFE00, 0xFE00));
    script(&mut m, &[call("Color2Index", &[Lit(RGB_B)], Some(0x3400))]);
    assert_eq!(m.mem.read_u32(0x3400).unwrap(), 0, "near white is white");
    // Index2Color beyond the table is a refusal, not a made-up colour
    let before = m.refused.len();
    script(&mut m, &[call("Index2Color", &[Lit(300), Lit(OUT)], None)]);
    assert!(m.refused[before..].iter().any(|r| r.contains("Index2Color") && r.contains("300")), "{:?}", m.refused);
}

#[test]
fn get_ctable_builds_standard_tables_and_seeds_are_unique() {
    let mut m = machine("mac-ii");
    script(&mut m, &[
        call("GetCTable", &[Lit(8)], Some(0x3400)), call("GetCTable", &[Lit(34)], Some(0x3404)), call("GetCTable", &[Lit(77)], Some(0x3408)),
        call("GetCTSeed", &[], Some(0x340C)), call("GetCTSeed", &[], Some(0x3410)),
    ]);
    let t8 = m.mem.read_u32(m.mem.read_u32(0x3400).unwrap()).unwrap();
    assert_eq!((m.mem.read_u32(t8).unwrap(), m.mem.read_u16(t8 + 6).unwrap()), (8, 255));
    for (i, c) in clut8().iter().enumerate() {
        let at = t8 + 8 + 8 * i as u32;
        assert_eq!((m.mem.read_u16(at).unwrap(), read_rgb(&m, at + 2)), (i as u16, (c.r, c.g, c.b)), "entry {i}");
    }
    let t2 = m.mem.read_u32(m.mem.read_u32(0x3404).unwrap()).unwrap();
    assert_eq!((m.mem.read_u16(t2 + 6).unwrap(), read_rgb(&m, t2 + 8 + 8 + 2), read_rgb(&m, t2 + 8 + 24 + 2)), (3, (0xAAAA, 0xAAAA, 0xAAAA), (0, 0, 0)), "the 2-bit gray table: white, light, dark, black");
    assert_eq!(m.mem.read_u32(0x3408).unwrap(), 0, "no such table: NIL");
    let (a, b) = (m.mem.read_u32(0x340C).unwrap(), m.mem.read_u32(0x3410).unwrap());
    assert!(a >= 1024 && b == a + 1, "unique seeds above the standard tables' ({a}, {b})");
    script(&mut m, &[call("QDError", &[], Some(0x3414)), call("GetCTable", &[Lit(8)], Some(0x3418)), call("QDError", &[], Some(0x341C))]);
    assert_eq!(m.mem.read_u16(0x3414).unwrap() as i16, -192, "resNotFound after the failed GetCTable");
    assert_eq!(m.mem.read_u16(0x341C).unwrap() as i16, 0, "a good call clears it");
}

// ---- 3. colour ports and colour windows ---------------------------------------------------------------------------------------

#[test]
fn new_cwindow_makes_a_cgrafport_over_an_8_bit_canvas_and_a_plain_window_stays_1_bit() {
    let w = color_window();
    assert_eq!(w.m.qd.canvases[&w.win].depth(), 8);
    assert_eq!(w.m.mem.read_u16(w.win + 6).unwrap() & 0xC000, 0xC000, "portVersion marks a CGrafPort");
    let pmh = w.m.mem.read_u32(w.win + 2).unwrap();
    let pm = w.m.mem.read_u32(pmh).unwrap();
    assert_eq!(w.m.mem.read_u16(pm + 32).unwrap(), 8);
    assert_eq!([0, 2, 4, 6].map(|o| w.m.mem.read_i16(pm + 6 + o).unwrap()), [-100, -50, 20, 150], "PixMap bounds map local coordinates to the screen");
    assert_eq!(read_rgb(&w.m, w.win + 36), (0, 0, 0), "rgbFgColor black");
    assert_eq!(read_rgb(&w.m, w.win + 42), (0xFFFF, 0xFFFF, 0xFFFF), "rgbBkColor white");
    // the same machine's NewWindow is the classic 1-bit GrafPort
    let mut m = machine("mac-ii");
    let spec = WindowSpec { storage: 0, rect: Rect { top: 100, left: 50, bottom: 220, right: 250 }, title: "P".into(), visible: true, proc_id: 0, behind: 0xFFFF_FFFF, go_away: false, ref_con: 0 };
    let plain = m.make_window(spec).unwrap();
    assert_eq!((m.qd.canvases[&plain].depth(), m.mem.read_u16(plain + 6).unwrap() & 0xC000), (1, 0));
}

#[test]
fn a_machine_without_color_quickdraw_refuses_new_cwindow() {
    let mut m = machine("mac-se");
    let mut p = Prog::new();
    p.clr_l().l(0).l(R1).l(0).b(1).w(0).l(0xFFFF_FFFF).b(0).l(0).trap(0xAA45).pop_l(0x3400).exit();
    set_rect(&mut m, R1, 50, 50, 150, 250);
    put(&mut m, 0x3600, &pstr("x"));
    run(&mut m, &p);
    assert!(m.refused.iter().any(|r| r.contains("NewCWindow") && r.contains("mac-se")), "{:?}", m.refused);
    assert!(m.windows.is_empty());
}

#[test]
fn rgb_colors_are_stored_in_the_port_and_painted_with_the_nearest_table_entry() {
    let mut w = color_window();
    set_rect(&mut w.m, R1, 10, 20, 30, 60);
    rgb(&mut w.m, RGB_A, (0xFFFF, 0, 0)); // exactly cube entry 5*6 + 5 = 35
    w.script(&[call("RGBForeColor", &[Lit(RGB_A)], None), call("PaintRect", &[Lit(R1)], None), call("GetForeColor", &[Lit(OUT)], None), call("GetBackColor", &[Lit(OUT + 8)], None)]);
    assert_eq!(w.px(30, 20), 35, "painted with the table's red");
    assert_eq!((w.px(19, 20), w.px(60, 20), w.px(30, 9), w.px(30, 30)), (0, 0, 0, 0), "the paper stays white (entry 0), outside the rectangle");
    assert_eq!(read_rgb(&w.m, w.win + 36), (0xFFFF, 0, 0), "rgbFgColor in the CGrafPort");
    assert_eq!(read_rgb(&w.m, OUT), (0xFFFF, 0, 0));
    assert_eq!(read_rgb(&w.m, OUT + 8), (0xFFFF, 0xFFFF, 0xFFFF), "GetBackColor");
    let painted = w.m.qd.canvases[&w.win].pixels().iter().filter(|&&p| p == 35).count();
    assert_eq!(painted, 20 * 40);
    // NEGATIVE CONTROL: without RGBForeColor the same PaintRect is black (entry 255)
    let mut n = color_window();
    set_rect(&mut n.m, R1, 10, 20, 30, 60);
    n.script(&[call("PaintRect", &[Lit(R1)], None)]);
    assert_eq!(n.px(30, 20), 255);
}

#[test]
fn the_back_color_is_the_paper_of_erase_and_of_patterns() {
    let mut w = color_window();
    set_rect(&mut w.m, R1, 0, 0, 120, 200);
    rgb(&mut w.m, RGB_A, (0, 0, 0xFFFF)); // blue: 5*36 + 0*6 + 0? levels FF=0, 00=5 -> r 5, g 5, b 0 = 5*36+5*6+0 = 210
    w.script(&[call("RGBBackColor", &[Lit(RGB_A)], None), call("EraseRect", &[Lit(R1)], None)]);
    assert_eq!(w.px(100, 60), 210, "EraseRect fills with the background colour");
    // a gray pen pattern alternates foreground and background pixels
    put(&mut w.m, 0x3500, &[0xAA, 0x55, 0xAA, 0x55, 0xAA, 0x55, 0xAA, 0x55]);
    set_rect(&mut w.m, R1, 0, 0, 8, 8);
    rgb(&mut w.m, RGB_B, (0xFFFF, 0, 0));
    w.script(&[call("RGBForeColor", &[Lit(RGB_B)], None), call("PenPat", &[Lit(0x3500)], None), call("PaintRect", &[Lit(R1)], None)]);
    assert_eq!((w.px(0, 0), w.px(1, 0), w.px(0, 1), w.px(1, 1)), (35, 210, 210, 35), "pattern bit 1 = foreground, 0 = background");
    // PenPat in a CGrafPort lives in its PixPat: pat1Data
    let pn = w.m.mem.read_u32(w.m.mem.read_u32(w.win + 58).unwrap()).unwrap();
    assert_eq!((w.m.mem.read_u16(pn).unwrap(), w.m.mem.read_u8(pn + 22).unwrap()), (0, 0xAA), "PixPat patType 0, pat1Data");
}

#[test]
fn boolean_transfer_modes_act_on_pixel_values_so_the_standard_table_reads_as_black_and_white() {
    let mut w = color_window();
    set_rect(&mut w.m, R1, 0, 0, 10, 10);
    // black over white, then invert (patXor with black): 255 -> 0
    w.script(&[call("PaintRect", &[Lit(R1)], None)]);
    assert_eq!(w.px(5, 5), 255);
    w.script(&[call("InverRect", &[Lit(R1)], None)]);
    assert_eq!(w.px(5, 5), 0, "InvertRect flips every bit of the pixel value: black is white");
    // patOr (9) of black over a coloured pixel gives black; patBic (11) of black clears to white
    rgb(&mut w.m, RGB_A, (0xFFFF, 0, 0));
    w.script(&[call("RGBForeColor", &[Lit(RGB_A)], None), call("PaintRect", &[Lit(R1)], None)]);
    assert_eq!(w.px(5, 5), 35);
    w.script(&[call("ForeColor", &[Lit(33)], None), call("PenMode", &[Lit(9)], None), call("PaintRect", &[Lit(R1)], None)]);
    assert_eq!(w.px(5, 5), 255, "35 | 255");
    w.script(&[call("PenMode", &[Lit(11)], None), call("PaintRect", &[Lit(R1)], None)]);
    assert_eq!(w.px(5, 5), 0, "255 & !255");
}

#[test]
fn the_classic_foreground_constants_set_the_rgb_color_of_a_colour_port() {
    let mut w = color_window();
    w.script(&[call("ForeColor", &[Lit(205)], None), call("BackColor", &[Lit(341)], None), call("GetForeColor", &[Lit(OUT)], None), call("GetBackColor", &[Lit(OUT + 8)], None)]);
    assert_eq!(read_rgb(&w.m, OUT), (0xDDDD, 0x0808, 0x0606), "redColor = the standard 4-bit table's red");
    assert_eq!(read_rgb(&w.m, OUT + 8), (0x0000, 0x6464, 0x1111), "greenColor");
}

// ---- 4. what the host is shown ------------------------------------------------------------------------------------------------

fn pixels_ops(m: &Mac<RecordingGuestUi>) -> Vec<(guestui_abi::GuestRect, Vec<u32>)> {
    let mut out = vec![];
    for c in &m.ui.calls { if let UiCall::PresentPlane(_, p) = c { for o in &p.ops { if let DrawOp::Pixels { rect, rgb } = o { out.push((*rect, rgb.clone())); } } } }
    out
}

#[test]
fn a_colour_window_is_shown_as_its_exact_rgb_pixels_never_as_theme_roles() {
    let mut w = color_window();
    set_rect(&mut w.m, R1, 10, 20, 30, 60);
    rgb(&mut w.m, RGB_A, (0xFFFF, 0, 0));
    w.script(&[call("RGBForeColor", &[Lit(RGB_A)], None), call("PaintRect", &[Lit(R1)], None)]);
    let ops = pixels_ops(&w.m);
    assert!(!ops.is_empty(), "drawn as raw pixels");
    let (rect, px) = ops.last().unwrap();
    let at = |x: i32, y: i32| px[((y - rect.top) * rect.width + (x - rect.left)) as usize];
    assert_eq!(at(30, 20), 0x00FF_0000, "the red the program painted, as the host's 0x00RRGGBB");
    assert!(w.m.ui.calls.iter().all(|c| !matches!(c, UiCall::PresentPlane(_, p) if p.ops.iter().any(|o| matches!(o, DrawOp::Fill { .. } | DrawOp::Line { .. })))), "no theme-role Fill/Line for a colour window");
}

#[test]
fn changing_the_windows_colour_table_repaints_the_window_through_the_new_table() {
    let mut w = color_window();
    set_rect(&mut w.m, R1, 10, 20, 30, 60);
    w.script(&[call("PaintRect", &[Lit(R1)], None)]); // entry 255
    let last = |w: &Win| { let (r, px) = pixels_ops(&w.m).pop().unwrap(); (r, px) };
    let (r, px) = last(&w);
    assert_eq!(px[((20 - r.top) * r.width + (30 - r.left)) as usize], 0x0000_0000, "black through the standard table");
    // the window shares the device table; give entry 255 a new colour in place (what SetEntries / an animated palette does)
    let pmh = w.m.mem.read_u32(w.win + 2).unwrap();
    let ct = w.m.mem.read_u32(w.m.mem.read_u32(w.m.mem.read_u32(pmh).unwrap() + 42).unwrap()).unwrap();
    let at = ct + 8 + 8 * 255 + 2;
    rgb(&mut w.m, at, (0x0000, 0xFFFF, 0));
    w.script(&[]);
    let (r, px) = last(&w);
    assert_eq!(px[((20 - r.top) * r.width + (30 - r.left)) as usize], 0x0000_FF00, "the painted pixels show the table's new entry 255");
    assert_eq!((r.width, r.height), (200, 120), "the whole window is shown again");
}

// ---- 5. a colour port over the program's own pixels (OpenCPort + a PixMap in guest memory) ---------------------------------

const PORT: u32 = 0x3800; // a CGrafPort record
const PIX: u32 = 0x30000; // 16 x 8 8-bit pixels (clear of the program at 0x10000 and the stack)

#[test]
fn open_cport_draws_into_a_pixmap_in_guest_memory_and_close_frees_what_it_made() {
    let mut m = machine("mac-ii");
    let live = m.mm.live_handles();
    run(&mut m, Prog::new().l(PORT).trap(0xAA00).exit());
    assert_eq!(m.mem.read_u16(PORT + 6).unwrap() & 0xC000, 0xC000);
    // point the port's own copy of the device PixMap at 16 x 8 pixels of the program's memory
    let pm = m.mem.read_u32(m.mem.read_u32(PORT + 2).unwrap()).unwrap();
    m.mem.write_u32(pm, PIX).unwrap();
    m.mem.write_u16(pm + 4, 0x8000 | 16).unwrap();
    for (i, v) in [0u16, 0, 8, 16].into_iter().enumerate() { m.mem.write_u16(pm + 6 + 2 * i as u32, v).unwrap(); m.mem.write_u16(PORT + 16 + 2 * i as u32, v).unwrap(); }
    m.mem.write_bytes(PIX, &[0u8; 128]).unwrap();
    set_rect(&mut m, R1, 2, 3, 5, 9);
    rgb(&mut m, RGB_A, (0xFFFF, 0, 0));
    let vis = m.mem.read_u32(PORT + 24).unwrap();
    m.write_region(vis, &retro_raster::Region::from_rect(retro_raster::Rect::new(0, 0, 16, 8))).unwrap();
    script(&mut m, &[call("RGBForeColor", &[Lit(RGB_A)], None), call("PaintRect", &[Lit(R1)], None)]);
    let b = m.mem.bytes(m.mem.span(PIX, 128).unwrap()).unwrap().to_vec();
    assert_eq!(b.iter().filter(|&&p| p == 35).count(), 3 * 6, "3 rows of 6 red pixels, packed one byte each");
    assert_eq!((b[2 * 16 + 3], b[4 * 16 + 8], b[5 * 16 + 3], b[2 * 16 + 2]), (35, 35, 0, 0));
    run(&mut m, Prog::new().l(PORT).trap(0xAA02).exit());
    assert_eq!(m.mm.live_handles(), live, "CloseCPort disposes the pixel map, its table, the patterns and the regions");
    assert!(m.refused.is_empty(), "{:?}", m.refused);
}

#[test]
fn copy_bits_colours_a_1_bit_source_with_the_port_pens_and_maps_between_tables() {
    let mut m = machine("mac-ii");
    run(&mut m, Prog::new().l(PORT).trap(0xAA00).exit());
    let pm = m.mem.read_u32(m.mem.read_u32(PORT + 2).unwrap()).unwrap();
    m.mem.write_u32(pm, PIX).unwrap();
    m.mem.write_u16(pm + 4, 0x8000 | 16).unwrap();
    for (i, v) in [0u16, 0, 8, 16].into_iter().enumerate() { m.mem.write_u16(pm + 6 + 2 * i as u32, v).unwrap(); m.mem.write_u16(PORT + 16 + 2 * i as u32, v).unwrap(); }
    m.mem.write_bytes(PIX, &[0u8; 128]).unwrap();
    // a 8 x 2 1-bit BitMap: first row 0xF0 (four black pixels then four white), second row 0x0F
    const BM: u32 = 0x3A00;
    m.mem.write_u32(BM, 0x3B00).unwrap();
    m.mem.write_u16(BM + 4, 2).unwrap();
    for (i, v) in [0u16, 0, 2, 8].into_iter().enumerate() { m.mem.write_u16(BM + 6 + 2 * i as u32, v).unwrap(); }
    put(&mut m, 0x3B00, &[0xF0, 0, 0x0F, 0]);
    set_rect(&mut m, R1, 0, 0, 2, 8);
    set_rect(&mut m, 0x3108, 3, 4, 5, 12);
    rgb(&mut m, RGB_A, (0xFFFF, 0, 0)); // fg: red 35
    rgb(&mut m, RGB_B, (0, 0, 0xFFFF)); // bg: 5*36 + 5*6 + 0 = 210
    let vis = m.mem.read_u32(PORT + 24).unwrap();
    m.write_region(vis, &retro_raster::Region::from_rect(retro_raster::Rect::new(0, 0, 16, 8))).unwrap();
    script(&mut m, &[
        call("RGBForeColor", &[Lit(RGB_A)], None), call("RGBBackColor", &[Lit(RGB_B)], None),
        call("CopyBits", &[Lit(BM), Lit(pm), Lit(R1), Lit(0x3108), Lit(0), Lit(0)], None),
    ]);
    let b = m.mem.bytes(m.mem.span(PIX, 128).unwrap()).unwrap().to_vec();
    let row = |y: usize| b[y * 16 + 4..y * 16 + 12].to_vec();
    assert_eq!(row(3), [35, 35, 35, 35, 210, 210, 210, 210], "black pixels take the foreground, white the background");
    assert_eq!(row(4), [210, 210, 210, 210, 35, 35, 35, 35]);
    assert_eq!(b[3 * 16 + 3], 0, "nothing outside the destination rectangle");
    assert!(m.refused.is_empty(), "{:?}", m.refused);
}

#[test]
fn an_unsupported_pixel_depth_is_a_named_refusal() {
    let mut m = machine("mac-ii");
    run(&mut m, Prog::new().l(PORT).trap(0xAA00).exit());
    let pm = m.mem.read_u32(m.mem.read_u32(PORT + 2).unwrap()).unwrap();
    m.mem.write_u16(pm + 32, 32).unwrap(); // a direct-colour pixel map
    m.mem.write_u32(pm, PIX).unwrap();
    for (i, v) in [0u16, 0, 8, 16].into_iter().enumerate() { m.mem.write_u16(pm + 6 + 2 * i as u32, v).unwrap(); }
    set_rect(&mut m, R1, 0, 0, 4, 4);
    script(&mut m, &[call("PaintRect", &[Lit(R1)], None)]);
    assert!(m.refused.iter().any(|r| r.contains("32-bit pixels are not implemented")), "{:?}", m.refused);
}

#[test]
fn has_depth_and_set_depth_know_the_screens_depths() {
    let mut m = machine("mac-ii");
    let gd = m.mem.read_u32(0x8A4).unwrap();
    let mut prog = Prog::new();
    for (d, at) in [(8i16, 0x3400u32), (4, 0x3402), (1, 0x3404), (16, 0x3406)] {
        prog.clr_w().l(gd).w(d).w(0).w(0);
        prog.0.extend_from_slice(&[0x30, 0x3C, 0x0A, 0x14, 0xAA, 0xA2]); // move.w #$0A14,d0 ; _PaletteDispatch (HasDepth)
        prog.pop_w(at);
    }
    prog.clr_w().l(gd).w(8).w(0).w(0);
    prog.0.extend_from_slice(&[0x30, 0x3C, 0x0A, 0x13, 0xAA, 0xA2]); // SetDepth(gd, 8)
    prog.pop_w(0x3408);
    prog.clr_w().l(gd).w(4).w(0).w(0);
    prog.0.extend_from_slice(&[0x30, 0x3C, 0x0A, 0x13, 0xAA, 0xA2]); // SetDepth(gd, 4): not modelled
    prog.pop_w(0x340A);
    prog.exit();
    run(&mut m, &prog);
    let r = |a| m.mem.read_u16(a).unwrap();
    assert_eq!((r(0x3400) != 0, r(0x3402) != 0, r(0x3404) != 0, r(0x3406) != 0), (true, true, true, false), "1, 4 and 8 bits yes; 16 (direct) no");
    assert_eq!(r(0x3408), 0, "SetDepth to the current depth is noErr");
    assert_eq!(r(0x340A) as i16, -50, "switching the screen's depth is refused (paramErr to the program)");
    assert!(m.refused.iter().any(|r| r.contains("SetDepth") && r.contains("4 bits")), "{:?}", m.refused);
}


/// A colour port over 16 x 8 program pixels (value `fill`), visRgn the whole thing.
fn offscreen(m: &mut Mac<RecordingGuestUi>, fill: u8) -> u32 {
    let mut p = Prog::new();
    p.l(PORT).trap(0xAA00).exit();
    run(m, &p);
    let pm = m.mem.read_u32(m.mem.read_u32(PORT + 2).unwrap()).unwrap();
    m.mem.write_u32(pm, PIX).unwrap();
    m.mem.write_u16(pm + 4, 0x8000 | 16).unwrap();
    for (i, v) in [0u16, 0, 8, 16].into_iter().enumerate() { m.mem.write_u16(pm + 6 + 2 * i as u32, v).unwrap(); m.mem.write_u16(PORT + 16 + 2 * i as u32, v).unwrap(); }
    m.mem.write_bytes(PIX, &[fill; 128]).unwrap();
    let vis = m.mem.read_u32(PORT + 24).unwrap();
    m.write_region(vis, &retro_raster::Region::from_rect(retro_raster::Rect::new(0, 0, 16, 8))).unwrap();
    pm
}

#[test]
fn copy_mask_copies_only_where_the_mask_is_black() {
    let mut m = machine("mac-ii");
    let pm = offscreen(&mut m, 7);
    // source: 8 x 2 all black; mask: first row 0xF0, second row 0x0F
    const SRC: u32 = 0x3A00;
    const MASK: u32 = 0x3A20;
    for (bm, data) in [(SRC, 0x3B00u32), (MASK, 0x3B10)] {
        m.mem.write_u32(bm, data).unwrap();
        m.mem.write_u16(bm + 4, 2).unwrap();
        for (i, v) in [0u16, 0, 2, 8].into_iter().enumerate() { m.mem.write_u16(bm + 6 + 2 * i as u32, v).unwrap(); }
    }
    put(&mut m, 0x3B00, &[0xFF, 0, 0xFF, 0]);
    put(&mut m, 0x3B10, &[0xF0, 0, 0x0F, 0]);
    set_rect(&mut m, R1, 0, 0, 2, 8);
    set_rect(&mut m, 0x3108, 3, 4, 5, 12);
    rgb(&mut m, RGB_A, (0xFFFF, 0, 0)); // foreground red: entry 35
    script(&mut m, &[call("RGBForeColor", &[Lit(RGB_A)], None), call("CopyMask", &[Lit(SRC), Lit(MASK), Lit(pm), Lit(R1), Lit(R1), Lit(0x3108)], None)]);
    let b = m.mem.bytes(m.mem.span(PIX, 128).unwrap()).unwrap().to_vec();
    assert_eq!(b[3 * 16 + 4..3 * 16 + 12], [35, 35, 35, 35, 7, 7, 7, 7], "row 0 of the mask: the left four pixels");
    assert_eq!(b[4 * 16 + 4..4 * 16 + 12], [7, 7, 7, 7, 35, 35, 35, 35], "row 1: the right four");
    assert_eq!(b.iter().filter(|&&p| p == 35).count(), 8, "nothing else was touched");
    assert!(m.refused.is_empty(), "{:?}", m.refused);
    // a mask of another size is refused by name, nothing drawn
    set_rect(&mut m, 0x3110, 0, 0, 2, 4);
    script(&mut m, &[call("CopyMask", &[Lit(SRC), Lit(MASK), Lit(pm), Lit(R1), Lit(0x3110), Lit(0x3108)], None)]);
    assert!(m.refused.iter().any(|r| r.contains("CopyMask") && r.contains("scaling")), "{:?}", m.refused);
}

#[test]
fn set_port_pix_repoints_a_colour_port_and_set_port_bits_will_not_overwrite_one() {
    let mut m = machine("mac-ii");
    let _ = offscreen(&mut m, 0);
    // a second PixMap (a copy of the port's) over other pixels
    let h2 = m.mm.new_handle(&mut m.mem, &mut m.heap, 50, true).unwrap();
    let pm2 = m.mm.block(h2).unwrap().ptr;
    let pm1 = m.mem.read_u32(m.mem.read_u32(PORT + 2).unwrap()).unwrap();
    let bytes = m.mem.bytes(m.mem.span(pm1, 50).unwrap()).unwrap().to_vec();
    m.mem.write_bytes(pm2, &bytes).unwrap();
    m.mem.write_u32(pm2, PIX + 0x1000).unwrap();
    m.mem.write_bytes(PIX + 0x1000, &[0u8; 128]).unwrap();
    set_rect(&mut m, R1, 0, 0, 2, 2);
    script(&mut m, &[call("SetPortPix", &[Lit(h2)], None), call("PaintRect", &[Lit(R1)], None)]);
    assert_eq!(m.mem.read_u32(PORT + 2).unwrap(), h2);
    let (a, b) = (m.mem.bytes(m.mem.span(PIX, 128).unwrap()).unwrap().to_vec(), m.mem.bytes(m.mem.span(PIX + 0x1000, 128).unwrap()).unwrap().to_vec());
    assert_eq!((a.iter().filter(|&&p| p == 255).count(), b.iter().filter(|&&p| p == 255).count()), (0, 4), "the black went to the new pixel map only");
    // SetPortBits on a CGrafPort would clobber portPixMap / portVersion: refused, the record untouched
    put(&mut m, 0x3A00, &[0; 14]);
    script(&mut m, &[call("SetPBits", &[Lit(0x3A00)], None)]);
    assert!(m.refused.iter().any(|r| r.contains("SetPortBits") && r.contains("SetPortPix")), "{:?}", m.refused);
    assert_eq!((m.mem.read_u32(PORT + 2).unwrap(), m.mem.read_u16(PORT + 6).unwrap() & 0xC000), (h2, 0xC000));
}

#[test]
fn the_gdevice_record_has_the_inside_macintosh_offsets_for_every_field_a_program_reads() {
    // gdRefNum 0, gdFlags 20, gdPMap 22 (4 bytes), gdRefCon 26, gdNextGD 30, gdRect 34, gdMode 42 (Imaging With QuickDraw sums to 62
    // bytes only this way). A program that reads (**GetMainDevice()).gdRect itself (SimpleText 1.3 centring its first window) must
    // see the screen there, not the high half of gdMode.
    let mut m = machine("mac-ii");
    script(&mut m, &[call("GetMainDevice", &[], Some(0x3400)), call("NewGDevice", &[Lit(0xFFFF), Lit(0x1234_5678)], Some(0x3404))]);
    let (main, new) = (m.mem.read_u32(0x3400).unwrap(), m.mem.read_u32(0x3404).unwrap());
    let (mp, np) = (m.mem.read_u32(main).unwrap(), m.mem.read_u32(new).unwrap());
    assert_eq!(m.mm.block(main).unwrap().size, 62, "the record is 62 bytes");
    assert_eq!([0, 2, 4, 6].map(|o| m.mem.read_i16(mp + 34 + o).unwrap()), [0, 0, 342, 512], "gdRect is at +34");
    assert_eq!(m.mem.read_u32(mp + 26).unwrap(), 0, "gdRefCon (+26) is clear and does not overlap gdPMap (+22)");
    assert_ne!(m.mem.read_u32(mp + 22).unwrap(), 0, "gdPMap survives");
    assert_eq!((m.mem.read_u16(np).unwrap(), m.mem.read_u32(np + 42).unwrap()), (0xFFFF, 0x1234_5678), "NewGDevice stores refNum at 0 and gdMode at +42");
    assert_eq!([0, 2, 4, 6].map(|o| m.mem.read_i16(np + 34 + o).unwrap()), [0, 0, 0, 0], "a new device's gdRect is empty: the mode did not leak into it");
}
