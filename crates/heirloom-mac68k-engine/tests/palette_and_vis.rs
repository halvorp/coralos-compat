//! Unit tests + negative controls for PaletteDispatch::RestoreDeviceClut and Window Manager
//! CalcVis / CalcVBehind / PaintOne / PaintBehind traps.
use heirloom_mac68k_engine::cqd::color::{clut8, read_rgb, write_rgb, Rgb};
use heirloom_mac68k_engine::cqd::ctab::{CT_TABLE, SPEC};
use heirloom_mac68k_engine::cqd::gdevice::GD_PMAP;
use heirloom_mac68k_engine::cqd::pixmap::PM_TABLE;
use heirloom_mac68k_engine::Mac;
use heirloom_ui_abi::RecordingGuestUi;

fn emit_u16(code: &mut Vec<u8>, w: u16) { code.extend_from_slice(&w.to_be_bytes()); }
fn emit_u32(code: &mut Vec<u8>, l: u32) { code.extend_from_slice(&l.to_be_bytes()); }

#[test]
fn restore_device_clut_restores_standard_8bit_table() {
    let mut m = Mac::new(RecordingGuestUi::new()).expect("machine");
    // InitGraf(-4(A5)), InitFonts, InitWindows to initialize main_device
    let mut code = Vec::new();
    emit_u16(&mut code, 0x486D); emit_u16(&mut code, 0xFFFC); // pea -4(a5)
    emit_u16(&mut code, 0xA86E); // _InitGraf
    emit_u16(&mut code, 0x4E75); // rts
    m.run_program(&code, 0, 10_000).expect("init");

    let gd = m.cqd.main_device;
    assert_ne!(gd, 0, "main device initialized");
    let gd_ptr = m.mem.read_u32(gd).unwrap();
    let pmap_h = m.mem.read_u32(gd_ptr + GD_PMAP).unwrap();
    let pmap_p = m.mem.read_u32(pmap_h).unwrap();
    let ctab_h = m.mem.read_u32(pmap_p + PM_TABLE).unwrap();
    let ctab_p = m.mem.read_u32(ctab_h).unwrap();

    // Corrupt entries 0 and 1 to distinct values
    let entry0 = ctab_p + CT_TABLE + 2;
    let entry1 = ctab_p + CT_TABLE + SPEC + 2;
    write_rgb(&mut m.mem, entry0, Rgb { r: 0x1234, g: 0x5678, b: 0x9ABC }).unwrap();
    write_rgb(&mut m.mem, entry1, Rgb { r: 0xDEAD, g: 0xBEEF, b: 0xCAFE }).unwrap();

    // Call RestoreDeviceClut(NULL) via PaletteDispatch (D0 = 0x0002, push 0L)
    let mut code2 = Vec::new();
    emit_u16(&mut code2, 0x42A7); // clr.l -(sp)  (gd = NULL)
    emit_u16(&mut code2, 0x303C); emit_u16(&mut code2, 0x0002); // move.w #$0002, d0
    emit_u16(&mut code2, 0xAAA2); // _PaletteDispatch
    emit_u16(&mut code2, 0x4E75); // rts
    let r = m.run_program(&code2, 0, 10_000).expect("restore");
    assert_eq!(r.stats.bus_faults, 0);

    let std = clut8();
    assert_eq!(read_rgb(&m.mem, entry0).unwrap(), std[0]);
    assert_eq!(read_rgb(&m.mem, entry1).unwrap(), std[1]);
}

#[test]
fn unknown_palette_dispatch_selector_refuses_and_halts() {
    let mut m = Mac::new(RecordingGuestUi::new()).expect("machine");
    let mut code = Vec::new();
    emit_u16(&mut code, 0x303C); emit_u16(&mut code, 0x0099); // move.w #$0099, d0 (unimplemented)
    emit_u16(&mut code, 0xAAA2); // _PaletteDispatch
    emit_u16(&mut code, 0x4E75); // rts
    let r = m.run_program(&code, 0, 10_000).expect("run");
    assert!(r.stats.halted, "unknown PaletteDispatch selector must halt guest");
    assert!(m.refused.iter().any(|s| s.contains("0x0099")), "refusal names selector: {:?}", m.refused);
}

#[test]
fn calc_vis_and_paint_one_invalidate_window_and_tolerate_null() {
    let mut m = Mac::new(RecordingGuestUi::new()).expect("machine");
    // Open a window via InitGraf + InitFonts + InitWindows + NewCWindow, then call CalcVis, CalcVBehind, PaintOne, PaintBehind
    let mut code = Vec::new();
    emit_u16(&mut code, 0x486D); emit_u16(&mut code, 0xFFFC); // pea -4(a5)
    emit_u16(&mut code, 0xA86E); // _InitGraf
    emit_u16(&mut code, 0xA8FE); // _InitFonts
    emit_u16(&mut code, 0xA912); // _InitWindows
    // Call CalcVis(0), CalcVBehind(0, 0), PaintOne(0, 0), PaintBehind(0, 0) — negative/null safety
    emit_u16(&mut code, 0x42A7); // clr.l -(sp)
    emit_u16(&mut code, 0xA909); // _CalcVis
    emit_u16(&mut code, 0x42A7); emit_u16(&mut code, 0x42A7);
    emit_u16(&mut code, 0xA90A); // _CalcVBehind
    emit_u16(&mut code, 0x42A7); emit_u16(&mut code, 0x42A7);
    emit_u16(&mut code, 0xA90C); // _PaintOne
    emit_u16(&mut code, 0x42A7); emit_u16(&mut code, 0x42A7);
    emit_u16(&mut code, 0xA90D); // _PaintBehind
    // Now open a window and call PaintOne(win, 0)
    // Reserve bounds rect on stack: (top=40, left=40, bottom=140, right=240)
    emit_u16(&mut code, 0x2F3C); emit_u32(&mut code, 0x008C_00F0); // move.l #(140<<16|240), -(sp)
    emit_u16(&mut code, 0x2F3C); emit_u32(&mut code, 0x0028_0028); // move.l #(40<<16|40), -(sp)
    emit_u16(&mut code, 0x204F); // movea.l sp, a0  (bounds ptr)
    emit_u16(&mut code, 0x4267); // clr.w -(sp)     (empty Pascal string "")
    emit_u16(&mut code, 0x224F); // movea.l sp, a1  (title ptr)
    // NewCWindow(wStorage=0, boundsRect=a0, title=a1, visible=1, procID=0, behind=-1, goAway=1, refCon=0) -> result.l
    emit_u16(&mut code, 0x42A7); // result slot
    emit_u16(&mut code, 0x42A7); // wStorage = 0
    emit_u16(&mut code, 0x2F08); // move.l a0, -(sp)
    emit_u16(&mut code, 0x2F09); // move.l a1, -(sp)
    emit_u16(&mut code, 0x1F3C); emit_u16(&mut code, 0x0100); // visible = true (byte in word)
    emit_u16(&mut code, 0x4267); // procID = 0
    emit_u16(&mut code, 0x2F3C); emit_u32(&mut code, 0xFFFF_FFFF); // behind = -1
    emit_u16(&mut code, 0x1F3C); emit_u16(&mut code, 0x0100); // goAway = true
    emit_u16(&mut code, 0x42A7); // refCon = 0
    emit_u16(&mut code, 0xAA45); // _NewCWindow
    emit_u16(&mut code, 0x201F); // move.l (sp)+, d0  (win)
    emit_u16(&mut code, 0x2F00); // move.l d0, -(sp)
    emit_u16(&mut code, 0x42A7); // rgn = 0
    emit_u16(&mut code, 0xA90C); // _PaintOne(win, 0)
    emit_u16(&mut code, 0xDEFc); emit_u16(&mut code, 10); // adda.w #10, sp (clean rect+title)
    emit_u16(&mut code, 0x4E75); // rts
    let r = m.run_program(&code, 0, 50_000).expect("run");
    assert_eq!(r.stats.bus_faults, 0);
    assert!(m.refused.is_empty(), "unexpected refusals: {:?}", m.refused);
    assert!(m.log.iter().any(|l| l.contains("PaintOne")), "log: {:?}", m.log);
}
