//! A WindowRecord's strucRgn / contRgn / updateRgn are real region handles in global coordinates (Inside Macintosh I, "The Window Manager").
mod common;
use common::*;
use mac68k_engine::winregions::{WR_CONT_RGN, WR_STRUC_RGN, WR_UPDATE_RGN};

fn bbox(g: &Rig, off: u32) -> Option<(i32, i32, i32, i32)> {
    let h = g.m.mem.read_u32(g.win + off).unwrap();
    assert_ne!(h, 0, "region at {off} is a real handle");
    let rg = g.m.read_region(h).unwrap();
    let rs = rg.rects();
    if rs.is_empty() { return None; }
    Some((rs.iter().map(|r| r.y0).min().unwrap(), rs.iter().map(|r| r.x0).min().unwrap(), rs.iter().map(|r| r.y1).max().unwrap(), rs.iter().map(|r| r.x1).max().unwrap()))
}

#[test]
fn a_new_window_has_a_content_region_a_structure_region_with_its_title_bar_and_a_full_update_region() {
    let g = Rig::new(); // content (top 100, left 50, bottom 220, right 250), proc 0: a titled document window
    assert_eq!(bbox(&g, WR_CONT_RGN), Some((100, 50, 220, 250)), "contRgn is the content in global coordinates");
    assert_eq!(bbox(&g, WR_STRUC_RGN), Some((100 - 19, 49, 221, 251)), "strucRgn: the frame and the 18-row title bar above the content");
    assert_eq!(bbox(&g, WR_UPDATE_RGN), Some((100, 50, 220, 250)), "a new window is wholly invalid, in global coordinates");
}

#[test]
fn the_update_region_follows_valid_and_inval_and_the_regions_follow_a_size_change() {
    let mut g = Rig::new();
    set_rect(&mut g.m, 0x3100, 0, 0, 120, 200);
    set_rect(&mut g.m, 0x3108, 10, 20, 30, 60);
    let mut p = Prog::new();
    p.set_port(g.win);
    p.l(0x3100).trap(0xA92A); // ValidRect(whole portRect)
    p.l(0x3108).trap(0xA928); // InvalRect(10,20,30,60) local
    g.run(p.exit());
    g.no_refusals();
    assert_eq!(bbox(&g, WR_UPDATE_RGN), Some((110, 70, 130, 110)), "just the invalid rectangle, shifted to global (+100 down, +50 right)");
    // SizeWindow(win, 100, 60, FALSE): the content regions follow
    let mut p = Prog::new();
    p.l(g.win).w(100).w(60).b(0).trap(0xA91D);
    g.run(p.exit());
    assert_eq!(bbox(&g, WR_CONT_RGN), Some((100, 50, 160, 150)), "contRgn after SizeWindow");
    assert_eq!(bbox(&g, WR_STRUC_RGN), Some((81, 49, 161, 151)));
}

#[test]
fn disposing_the_window_frees_its_three_regions() {
    let mut g = Rig::new();
    let hs: Vec<u32> = [WR_STRUC_RGN, WR_CONT_RGN, WR_UPDATE_RGN].iter().map(|&o| g.m.mem.read_u32(g.win + o).unwrap()).collect();
    assert!(hs.iter().all(|&h| g.m.mm.block(h).is_some()));
    let mut p = Prog::new();
    p.l(g.win).trap(0xA92D); // CloseWindow
    g.run(p.exit());
    assert!(hs.iter().all(|&h| g.m.mm.block(h).is_none()), "all three handles are gone (negative control: a leak keeps them)");
}

#[test]
fn moving_a_window_moves_its_structure_and_content_regions() {
    let mut g = Rig::new();
    let mut p = Prog::new();
    p.l(g.win).w(300).w(200).b(0).trap(0xA91B); // MoveWindow(win, hGlobal 300, vGlobal 200, front FALSE)
    g.run(p.exit());
    g.no_refusals();
    assert_eq!(bbox(&g, WR_CONT_RGN), Some((200, 300, 320, 500)), "contRgn after MoveWindow (no update was posted: this is the rectangle sync alone)");
    assert_eq!(bbox(&g, WR_STRUC_RGN), Some((181, 299, 321, 501)));
}

#[test]
fn a_procid_the_wdef_table_does_not_bind_is_refused_by_name_not_guessed_from_its_bits() {
    let mut g = Rig::new();
    // dBoxProc (1) is a dialog box: no title bar, so its structure region grows by the 1-pixel frame only
    g.m.windows.get_mut(&g.win).unwrap().proc_id = 1;
    g.m.sync_window_regions(g.win).unwrap();
    assert_eq!(bbox(&g, WR_STRUC_RGN), Some((99, 49, 221, 251)), "a dialog box has no title bar");
    // WDEF 128, variation 0: the program's own window definition
    g.m.windows.get_mut(&g.win).unwrap().proc_id = 2048;
    let e = g.m.sync_window_regions(g.win).unwrap_err();
    assert!(e.0.contains("WDEF-UNSUPPORTED"), "{}", e.0);
}
