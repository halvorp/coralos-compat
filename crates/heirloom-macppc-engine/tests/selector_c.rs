//! The List Manager (`_Pack0`) and the styled TextEdit dispatch (`_TEDispatch`) by C name from PowerPC: Mixed Mode builds the 68k
//! frame and runs the SAME selector body, so a list made here is the one a 68k program's `LNew` makes (the Warcraft III Installer's
//! install-location list: LNew, LAddRow, LSetCell, LSetSelect, LGetSelect, LClick, LUpdate, LSetDrawingMode, LDispose).
use heirloom_ui_abi::RecordingGuestUi;
use heirloom_macppc_engine::{GlueOutcome, MacPpc, PpcHost};
use heirloom_mac68k_engine::pascal::Rect;
use heirloom_mac68k_engine::toolbox::WindowSpec;

type M = MacPpc<RecordingGuestUi>;
const VIEW: u32 = 0x3100;
const BOUNDS: u32 = 0x3110;
const CELL: u32 = 0x3120;
const TEXT: u32 = 0x3130;

fn machine() -> (M, u32) {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    heirloom_mac68k_engine::quickdraw::fill_globals(&mut m.mac, 0x3F00).unwrap();
    m.mac.qd_globals = 0x3F00;
    let spec = WindowSpec { storage: 0, rect: Rect { top: 100, left: 50, bottom: 300, right: 350 }, title: "W".into(), visible: true, proc_id: 0, behind: 0xFFFF_FFFF, go_away: true, ref_con: 0 };
    let win = m.mac.make_window(spec).unwrap();
    for (i, v) in [10i16, 10, 110, 210].into_iter().enumerate() { m.mac.mem.write_u16(VIEW + 2 * i as u32, v as u16).unwrap(); }
    for (i, v) in [0i16, 0, 0, 1].into_iter().enumerate() { m.mac.mem.write_u16(BOUNDS + 2 * i as u32, v as u16).unwrap(); }
    m.mac.mem.write_bytes(TEXT, b"Macintosh HD").unwrap();
    (m, win)
}
fn call(m: &mut M, name: &str, args: &[u32]) -> u32 {
    m.bind(name);
    let mut regs = [0u32; 32];
    regs[3..3 + args.len()].copy_from_slice(args);
    assert_eq!(m.glue_call(name, &mut regs), GlueOutcome::Return, "{name}: {:?} {:?}", m.mac.refused, m.mac.log.last());
    regs[3]
}
fn cell(v: i16, h: i16) -> u32 { (v as u16 as u32) << 16 | h as u16 as u32 }

#[test]
fn a_list_made_from_powerpc_holds_rows_cells_and_a_selection() {
    let (mut m, win) = machine();
    // LNew(rView, dataBounds, cSize (0,0) = default, theProc 0, theWindow, drawIt, hasGrow, scrollH, scrollV)
    let lh = call(&mut m, "LNew", &[VIEW, BOUNDS, 0, 0, win, 1, 0, 0, 1]);
    assert_ne!(lh, 0, "{:?}", m.mac.refused);
    assert!(m.mac.tr.lists.contains_key(&lh), "the 68k List Manager holds it");
    assert_eq!(call(&mut m, "LAddRow", &[2, 0, lh]) as i32, 0, "two rows inserted at row 0: the first new row");
    call(&mut m, "LSetCell", &[TEXT, 12, cell(1, 0), lh]);
    assert_eq!(m.mac.tr.lists[&lh].cells.get(&(1, 0)).map(|c| c.as_slice()), Some(&b"Macintosh HD"[..]));
    call(&mut m, "LSetSelect", &[1, cell(1, 0), lh]);
    m.mac.mem.write_u32(CELL, 0).unwrap();
    assert_eq!(call(&mut m, "LGetSelect", &[1, CELL, lh]), 1, "a selected cell is found from (0,0) on");
    assert_eq!(m.mac.mem.read_u32(CELL).unwrap(), cell(1, 0));
    call(&mut m, "LSetDrawingMode", &[0, lh]);
    call(&mut m, "LUpdate", &[0, lh]);
    call(&mut m, "LSetSelect", &[0, cell(1, 0), lh]);
    m.mac.mem.write_u32(CELL, 0).unwrap();
    assert_eq!(call(&mut m, "LGetSelect", &[1, CELL, lh]), 0, "NEGATIVE CONTROL: deselected, nothing is found");
    // a cell outside dataBounds is the body's named refusal: the program goes on with 0 and the refusal is named
    call(&mut m, "LSetCell", &[TEXT, 12, cell(9, 0), lh]);
    assert!(m.mac.refused.iter().any(|r| r.starts_with("LSetCell")), "{:?}", m.mac.refused);
    call(&mut m, "LDispose", &[lh]);
    assert!(!m.mac.tr.lists.contains_key(&lh));
}

#[test]
fn te_get_height_from_powerpc_is_the_styled_dispatch_body() {
    let (mut m, _) = machine();
    for (i, v) in [0i16, 0, 100, 200].into_iter().enumerate() { m.mac.mem.write_u16(VIEW + 2 * i as u32, v as u16).unwrap(); }
    let te = call(&mut m, "TEStyleNew", &[VIEW, VIEW]);
    assert_ne!(te, 0, "{:?}", m.mac.refused);
    let one = call(&mut m, "TEGetHeight", &[1, 1, te]);
    assert!(one > 0, "one line has a height: {one}");
    assert!(!m.mac.refused.iter().any(|r| r.contains("TEGetHeight")), "{:?}", m.mac.refused);
}

/// `SndChannelStatus` from PowerPC runs the `_SoundDispatch` body: NULL is paramErr, a pointer that is no channel is badChannel, a
/// real idle channel reports scChannelBusy FALSE (the Warcraft III Installer polls its music channel this way).
#[test]
fn snd_channel_status_from_powerpc_is_the_sound_dispatch_body() {
    let (mut m, _) = machine();
    const CHAN: u32 = 0x3400;
    const STATUS: u32 = 0x3500;
    m.mac.mem.write_u32(CHAN, 0).unwrap();
    assert_eq!(call(&mut m, "SndNewChannel", &[CHAN, 5, 0, 0]) as i16, 0, "{:?}", m.mac.refused);
    let chan = m.mac.mem.read_u32(CHAN).unwrap();
    m.mac.mem.write_bytes(STATUS, &[0xAA; 24]).unwrap();
    assert_eq!(call(&mut m, "SndChannelStatus", &[chan, 24, STATUS]) as i32, 0);
    assert_eq!(m.mac.mem.read_u8(STATUS + 12).unwrap(), 0, "idle: not busy");
    assert_eq!(call(&mut m, "SndChannelStatus", &[0, 24, STATUS]) as i32, -50, "NULL channel");
    assert_eq!(call(&mut m, "SndChannelStatus", &[0x3600, 24, STATUS]) as i32, -205, "not a channel: badChannel");
    assert_eq!(call(&mut m, "SndChannelStatus", &[chan, 8, STATUS]) as i32, -50, "too short for an SCStatus");
}

/// The Carbon Scrap Manager over the one desk scrap: GetCurrentScrap's reference reads what PutScrap put (GetScrapFlavorSize), an absent
/// flavor is noTypeErr, and a reference taken before ZeroScrap is refused (paramErr), never answered from the new scrap.
#[test]
fn carbon_scrap_reads_the_desk_scrap_and_a_stale_reference_is_refused() {
    let (mut m, _) = machine();
    const REF: u32 = 0x3700;
    const SIZE: u32 = 0x3704;
    m.mac.mem.write_bytes(TEXT, b"KEY").unwrap();
    call(&mut m, "ZeroScrap", &[]);
    call(&mut m, "PutScrap", &[3, u32::from_be_bytes(*b"TEXT"), TEXT]);
    assert_eq!(call(&mut m, "GetCurrentScrap", &[REF]), 0);
    let r = m.mac.mem.read_u32(REF).unwrap();
    assert_eq!(call(&mut m, "GetScrapFlavorSize", &[r, u32::from_be_bytes(*b"TEXT"), SIZE]), 0);
    assert_eq!(m.mac.mem.read_u32(SIZE).unwrap(), 3);
    assert_eq!(call(&mut m, "GetScrapFlavorSize", &[r, u32::from_be_bytes(*b"PICT"), SIZE]) as i32, -102, "absent flavor");
    call(&mut m, "ZeroScrap", &[]);
    assert_eq!(call(&mut m, "GetScrapFlavorSize", &[r, u32::from_be_bytes(*b"TEXT"), SIZE]) as i32, -50, "NEGATIVE CONTROL: a reference to the old scrap");
    assert_eq!(call(&mut m, "GetCurrentScrap", &[0]) as i32, -50);
}
