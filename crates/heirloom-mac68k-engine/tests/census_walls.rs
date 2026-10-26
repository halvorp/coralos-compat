//! The walls the Macintosh Garden census named (ResEdit, SimpleText, WriteNow, MacPaint, Moving Van, Toolbox Showcase), each
//! against hand-assembled calls with a NEGATIVE CONTROL: null events from the host clock, the Vertical Retrace Manager, the
//! Apple Event handler registry and a handler run in the guest, the Alias Manager and FindFolder, the Palette Manager, the polled
//! mouse, the menu item accessors, GetAppParms.
mod common;
use common::{pstr, put, Prog, Rig, SCRATCH};
use heirloom_mac68k_engine::lowmem::{MB_STATE, MOUSE};
use heirloom_mac68k_engine::machine::CODE_BASE;

fn words(p: &mut Prog, w: &[u16]) { for x in w { p.0.extend_from_slice(&x.to_be_bytes()); } }
fn l(rig: &Rig, a: u32) -> u32 { rig.m.mem.read_u32(a).unwrap() }
fn w(rig: &Rig, a: u32) -> i16 { rig.m.mem.read_u16(a).unwrap() as i16 }
fn text(rig: &Rig, a: u32) -> String { let n = rig.m.mem.read_u8(a).unwrap() as u32; (0..n).map(|i| char::from(rig.m.mem.read_u8(a + 1 + i).unwrap())).collect() }
/// `move.w #sel,d0` then the trap: the selector register of the dispatch traps.
fn dispatch(p: &mut Prog, sel: u16, trap: u16) { words(p, &[0x303C, sel]); p.trap(trap); }
fn four(s: &[u8; 4]) -> u32 { u32::from_be_bytes(*s) }

const EVT: u32 = 0x3200;
const RES: u32 = 0x3100;

// ---- null events from the host clock ------------------------------------------------------------------------------------

#[test]
fn an_event_call_with_nothing_queued_completes_with_a_null_event_when_the_host_clock_reaches_its_deadline() {
    // GetNextEvent(everyEvent, &evt): a real Mac never blocks, it returns the null event; here the host's 60 Hz clock decides when.
    let mut g = Rig::new();
    g.m.start(&Prog::new().clr_w().w(-1).l(EVT).trap(0xA970).pop_w(RES).exit().0, 0).unwrap();
    assert!(g.m.run_loop(100_000).unwrap().stats.blocked, "nothing queued: the call waits");
    assert!(g.m.run_loop(100_000).unwrap().stats.blocked, "NEGATIVE CONTROL: with no tick it keeps waiting");
    g.m.advance_ticks(1);
    let r = g.m.run_loop(100_000).unwrap();
    assert!(r.stats.halted, "one tick later the call returned and the program ran on: {:?}", g.m.log);
    assert_eq!((w(&g, EVT), (w(&g, RES) as u16) >> 8), (0, 0), "what = nullEvent, the Boolean result is FALSE");
    // WaitNextEvent(everyEvent, &evt, sleep 5, NIL): due after FIVE ticks, not before
    let mut g = Rig::new();
    g.m.start(&Prog::new().clr_w().w(-1).l(EVT).l(5).l(0).trap(0xA860).pop_w(RES).exit().0, 0).unwrap();
    assert!(g.m.run_loop(100_000).unwrap().stats.blocked);
    g.m.advance_ticks(4);
    assert!(g.m.run_loop(100_000).unwrap().stats.blocked, "four ticks of a five-tick sleep");
    g.m.advance_ticks(1);
    assert!(g.m.run_loop(100_000).unwrap().stats.halted);
    // A real event still wins: a queued mouseDown is returned at once, with TRUE.
    let mut g = Rig::new();
    g.m.post(1, 0, 5, 6);
    g.run(Prog::new().clr_w().w(-1).l(EVT).trap(0xA970).pop_w(RES).exit());
    assert_eq!((w(&g, EVT), (w(&g, RES) as u16) >> 8), (1, 1));
}

// ---- the Vertical Retrace Manager ---------------------------------------------------------------------------------------

#[test]
fn a_vertical_retrace_task_is_counted_down_by_the_host_clock_runs_in_the_guest_and_stops_when_removed() {
    const TASK: u32 = 0x3400;
    let mut g = Rig::new();
    // the task: add 1 to a counter, re-arm itself for 3 ticks (A0 = the task), RTS
    let routine_at = 0x100u32;
    let mut main = Prog::new();
    words(&mut main, &[0x41F9, (TASK >> 16) as u16, TASK as u16]); // lea TASK,a0
    main.trap(0xA033); // _VInstall
    words(&mut main, &[0x3F00]); // move.w d0,-(sp) -- keep the result
    main.pop_w(RES);
    let loop_at = main.0.len();
    main.clr_w().w(-1).l(EVT).trap(0xA970); // GetNextEvent (blocks until the host ticks)
    words(&mut main, &[0x301F]); // move.w (sp)+,d0
    let back = -((main.0.len() + 2 - loop_at) as i32);
    words(&mut main, &[0x6000, back as u16]); // bra.w loop
    main.0.resize(routine_at as usize, 0x4E);
    words(&mut main, &[0x52B9, (SCRATCH >> 16) as u16, (SCRATCH + 8) as u16, 0x317C, 3, 10, 0x4E75]); // addq.l #1,SCRATCH+8 ; move.w #3,10(a0) ; rts
    // VBLTask: qLink 0, qType 1 (vType), vblAddr, vblCount 3, vblPhase 0
    put(&mut g.m, TASK, &[0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 3, 0, 0]);
    g.m.mem.write_u32(TASK + 6, CODE_BASE + routine_at).unwrap();
    g.m.start(&main.0, 0).unwrap();
    assert!(g.m.run_loop(100_000).unwrap().stats.blocked);
    assert_eq!(w(&g, RES), 0, "VInstall: noErr");
    for _ in 0..10 { g.m.advance_ticks(1); g.m.run_loop(100_000).unwrap(); }
    assert_eq!(l(&g, SCRATCH + 8), 3, "ran at ticks 3, 6 and 9 and not at the others: {:#?}", g.m.log);
    assert!(g.m.refused.is_empty(), "{:?}", g.m.refused);
    // _VRemove stops it
    g.m.vbl.queue.retain(|&t| t != TASK);
    for _ in 0..10 { g.m.advance_ticks(1); g.m.run_loop(100_000).unwrap(); }
    assert_eq!(l(&g, SCRATCH + 8), 3, "NEGATIVE CONTROL: a removed task does not run");
    // NEGATIVE CONTROL: a record whose qType is not vType is vTypErr (-2) and is not queued
    let mut g = Rig::new();
    put(&mut g.m, TASK, &[0, 0, 0, 0, 0, 7, 0, 0, 0, 0, 0, 3, 0, 0]);
    let mut p = Prog::new();
    words(&mut p, &[0x41F9, (TASK >> 16) as u16, TASK as u16]);
    p.trap(0xA033);
    words(&mut p, &[0x33C0, 0, RES as u16]); // move.w d0,RES
    g.run(p.exit());
    assert_eq!(w(&g, RES), -2);
    assert!(g.m.vbl.queue.is_empty());
}

// ---- the Apple Event Manager ---------------------------------------------------------------------------------------------

#[test]
fn an_oapp_handler_is_sent_the_open_application_event_and_runs_in_the_guest_and_an_unhandled_event_is_refused() {
    let mut g = Rig::new();
    let handler_at = 0x200u32;
    let mut p = Prog::new();
    p.l(0xCAFE_F00D); // a marker under the whole sequence: the stack must come back balanced
    p.clr_w().l(four(b"aevt")).l(four(b"oapp")).l(CODE_BASE + handler_at).l(0x1234).b(0);
    dispatch(&mut p, 0x091F, 0xA816); // AEInstallEventHandler -> OSErr
    p.pop_w(RES);
    p.clr_w().w(0x0400).l(EVT).trap(0xA970).pop_w(RES + 2); // GetNextEvent(highLevelEventMask): the Finder's oapp is waiting
    p.clr_w().l(EVT);
    dispatch(&mut p, 0x021B, 0xA816); // AEProcessAppleEvent(&evt) -> OSErr
    p.pop_w(RES + 4);
    p.pop_l(RES + 8); // the marker
    p.exit();
    p.0.resize(handler_at as usize, 0x4E);
    // handler(theAppleEvent, reply, refcon): OSErr, in 68k: movea.l (sp)+,a0 (the return: the trampoline) ; move.l (sp)+,d7 (the
    // refcon, the LAST argument, so it comes off first) ; move.l d7,SCRATCH+16 (record it) ; adda.w #8,sp (reply and event) ;
    // move.w #7,(sp) (the result slot) ; jmp (a0).
    let at = handler_at as usize;
    let h: Vec<u8> = [0x205F, 0x2E1F, 0x23C7, 0, SCRATCH as u16 + 16, 0xDEFC, 8, 0x3EBC, 7, 0x4ED0].iter().flat_map(|x: &u16| x.to_be_bytes()).collect();
    p.0.truncate(at);
    p.0.extend_from_slice(&h);
    g.m.start(&p.0, 0).unwrap();
    let r = g.m.run_loop(1_000_000).unwrap();
    assert!(r.stats.halted, "{:?}\n{:#?}", r.stats, g.m.log);
    assert_eq!(w(&g, RES), 0, "AEInstallEventHandler: noErr");
    assert_eq!((w(&g, RES + 2) as u16) >> 8, 1, "the oapp high-level event was queued by the install");
    assert_eq!(w(&g, EVT), 23, "what = kHighLevelEvent");
    assert_eq!(w(&g, RES + 4), 7, "the handler's OSErr is AEProcessAppleEvent's");
    assert_eq!(l(&g, SCRATCH + 16), 0x1234, "the handler ran in the guest and was given its refcon");
    assert_eq!(l(&g, RES + 8), 0xCAFE_F00D, "the stack came back balanced");
    g.no_refusals();
    // NEGATIVE CONTROL: an event nobody handles is errAEEventNotHandled and no guest code runs
    let mut g = Rig::new();
    put(&mut g.m, EVT, &[0, 23]);
    g.m.mem.write_u32(EVT + 2, four(b"aevt")).unwrap();
    g.m.mem.write_u32(EVT + 10, four(b"quit")).unwrap();
    let mut p = Prog::new();
    p.clr_w().l(EVT);
    dispatch(&mut p, 0x021B, 0xA816);
    p.pop_w(RES);
    g.run(p.exit());
    assert_eq!(w(&g, RES), -1708);
    // the registry: Get finds what Install put, Remove then Get is errAEHandlerNotFound (-1717)
    let mut g = Rig::new();
    let mut p = Prog::new();
    p.clr_w().l(four(b"aevt")).l(four(b"odoc")).l(0xABC0).l(5).b(0);
    dispatch(&mut p, 0x091F, 0xA816);
    p.pop_w(RES);
    p.clr_w().l(four(b"aevt")).l(four(b"odoc")).l(RES + 0x10).l(RES + 0x14).b(0); // AEGetEventHandler(class, id, &handler, &refcon, isSys)
    dispatch(&mut p, 0x0921, 0xA816);
    p.pop_w(RES + 2);
    p.clr_w().l(four(b"aevt")).l(four(b"odoc")).l(0xABC0).b(0); // AERemoveEventHandler(class, id, handler, isSys)
    dispatch(&mut p, 0x0720, 0xA816);
    p.pop_w(RES + 4);
    p.clr_w().l(four(b"aevt")).l(four(b"odoc")).l(RES + 0x10).l(RES + 0x14).b(0);
    dispatch(&mut p, 0x0921, 0xA816);
    p.pop_w(RES + 6);
    g.run(p.exit());
    assert_eq!((w(&g, RES), w(&g, RES + 2), l(&g, RES + 0x10), l(&g, RES + 0x14)), (0, 0, 0xABC0, 5));
    assert_eq!((w(&g, RES + 4), w(&g, RES + 6)), (0, -1717));
}

// ---- the Alias Manager and FindFolder ------------------------------------------------------------------------------------

#[test]
fn find_folder_makes_the_system_folders_and_an_alias_resolves_to_its_target_until_the_target_is_gone() {
    let mut g = Rig::new();
    let vol = g.m.fm.def_vol;
    g.m.fm.tree.put_file(vol, "Doc", b"hello".to_vec()).unwrap();
    const SPEC: u32 = 0x3500;
    const SPEC2: u32 = 0x3560;
    g.m.mem.write_u16(SPEC, g.m.fm.vref_of(vol) as u16).unwrap();
    g.m.mem.write_u32(SPEC + 2, 2).unwrap(); // fsRtDirID
    put(&mut g.m, SPEC + 6, &pstr("Doc"));
    let mut p = Prog::new();
    // FindFolder(kOnSystemDisk, 'pref', true, &vref, &dirid)
    p.clr_w().w(i16::MIN).l(four(b"pref")).b(1).l(RES + 0x20).l(RES + 0x24);
    dispatch(&mut p, 0x0000, 0xA823);
    p.pop_w(RES);
    // an unknown folder type is fnfErr
    p.clr_w().w(i16::MIN).l(four(b"zzzz")).b(1).l(RES + 0x28).l(RES + 0x2C);
    dispatch(&mut p, 0x0000, 0xA823);
    p.pop_w(RES + 2);
    // NewAlias(NIL, &spec, &alias)
    p.clr_w().l(0).l(SPEC).l(RES + 0x30);
    dispatch(&mut p, 0x0002, 0xA823);
    p.pop_w(RES + 4);
    // ResolveAlias(NIL, alias, &spec2, &changed)
    p.clr_w().l(0).rgn_arg(RES + 0x30).l(SPEC2).l(RES + 0x34);
    dispatch(&mut p, 0x0003, 0xA823);
    p.pop_w(RES + 6);
    // GetAliasInfo(alias, asiAliasName 0, &name)
    p.clr_w().rgn_arg(RES + 0x30).w(0).l(RES + 0x80);
    dispatch(&mut p, 0x0007, 0xA823);
    p.pop_w(RES + 8);
    g.run(p.exit());
    assert_eq!(w(&g, RES), 0);
    let (v, d) = (w(&g, RES + 0x20), l(&g, RES + 0x24));
    assert_eq!(v, g.m.fm.vref_of(vol));
    let pref = g.m.fm.dir_node(vol, d).unwrap();
    assert_eq!(g.m.fm.full_path(pref), b"Macintosh HD:System Folder:Preferences:".to_vec(), "made on first use inside the System Folder");
    assert_eq!(w(&g, RES + 2), -43, "NEGATIVE CONTROL: a folder type the Folder Manager lacks");
    assert_eq!((w(&g, RES + 4), w(&g, RES + 6), w(&g, RES + 8)), (0, 0, 0));
    assert_eq!(text(&g, SPEC2 + 6), "Doc", "ResolveAlias wrote the target into the FSSpec");
    assert_eq!(g.m.mem.read_u8(RES + 0x34).unwrap(), 0, "wasChanged FALSE: found where it was");
    assert_eq!(text(&g, RES + 0x80), "Doc");
    // NEGATIVE CONTROL: delete the target; the alias no longer resolves (fnfErr) and the FSSpec is left as it was
    let doc = g.m.fm.tree.find_child(vol, "Doc").unwrap();
    g.m.fm.tree.remove(doc).unwrap();
    put(&mut g.m, SPEC2 + 6, &pstr("untouched"));
    let mut p = Prog::new();
    p.clr_w().l(0).rgn_arg(RES + 0x30).l(SPEC2).l(RES + 0x34);
    dispatch(&mut p, 0x0003, 0xA823);
    p.pop_w(RES + 0xA0);
    g.run(p.exit());
    assert_eq!(w(&g, RES + 0xA0), -43);
    assert_eq!(text(&g, SPEC2 + 6), "untouched");
    // a selector the manager lacks halts the guest by name
    let mut g = Rig::new();
    let mut p = Prog::new();
    dispatch(&mut p, 0x0005, 0xA823);
    g.m.start(&p.exit().0, 0).unwrap();
    assert!(g.m.run_loop(10_000).unwrap().stats.halted);
    assert!(g.m.refused.iter().any(|r| r.contains("AliasDispatch selector 5 is not modelled")), "{:?}", g.m.refused);
}

// ---- the Palette Manager --------------------------------------------------------------------------------------------------

#[test]
fn palettes_are_real_records_attached_to_windows_and_a_bad_entry_is_refused_by_name() {
    let mut g = Rig::new();
    let win = g.win;
    put(&mut g.m, RES + 0x40, &[0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC]);
    let mut p = Prog::new();
    p.clr_l().w(4).l(0).w(1).w(0).trap(0xAA91).pop_l(RES); // NewPalette(4 entries, no colour table, pmTolerant 1, 0)
    p.rgn_arg(RES).w(2).l(RES + 0x40).trap(0xAA9C); // SetEntryColor(pal, 2, &rgb)
    p.rgn_arg(RES).w(2).l(RES + 0x50).trap(0xAA9B); // GetEntryColor(pal, 2, &out)
    p.clr_l().l(win).trap(0xAA96).pop_l(RES + 4); // GetPalette(win) before: none
    p.l(win).rgn_arg(RES).w(0x0100).trap(0xAA95); // NSetPalette(win, pal, pmAllUpdates)
    p.clr_l().l(win).trap(0xAA96).pop_l(RES + 8); // GetPalette(win) after
    p.rgn_arg(RES).w(9).l(RES + 0x60).trap(0xAA9B); // NEGATIVE CONTROL: entry 9 of 4
    g.run(p.exit());
    let h = l(&g, RES);
    assert_ne!(h, 0);
    assert_eq!(g.m.mem.read_u16(g.m.mm.block(h).unwrap().ptr).unwrap(), 4, "pmEntries");
    assert_eq!(g.m.mem.bytes(g.m.mem.span(RES + 0x50, 6).unwrap()).unwrap(), &[0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC]);
    assert_eq!((l(&g, RES + 4), l(&g, RES + 8)), (0, h), "GetPalette: NIL until NSetPalette");
    assert!(g.m.refused.iter().any(|r| r.contains("GetEntryColor: entry 9 is outside the palette's 4")), "{:?}", g.m.refused);
}

// ---- the polled mouse -----------------------------------------------------------------------------------------------------

#[test]
fn button_get_mouse_still_down_and_wait_mouse_up_follow_the_low_memory_mouse_and_the_event_queue() {
    let mut g = Rig::new();
    // global (v 150, h 80), button down; the rig's window content starts at (100, 50), so local = (50, 30)
    g.m.mem.write_u16(MOUSE, 150).unwrap();
    g.m.mem.write_u16(MOUSE + 2, 80).unwrap();
    g.m.mem.write_u8(MB_STATE, 0).unwrap();
    let win = g.win;
    let mut p = Prog::new();
    p.l(win).trap(0xA873); // SetPort
    p.clr_w().trap(0xA974).pop_w(RES); // Button
    p.l(RES + 0x10).trap(0xA972); // GetMouse(&pt)
    p.clr_w().trap(0xA973).pop_w(RES + 2); // StillDown
    g.run(&p.exit().0.clone().into_prog());
    assert_eq!(((w(&g, RES) as u16) >> 8, (w(&g, RES + 2) as u16) >> 8), (1, 1));
    assert_eq!((w(&g, RES + 0x10), w(&g, RES + 0x12)), (50, 30));
    // NEGATIVE CONTROLS: a queued mouseUp ends StillDown even while MBState still says down; WaitMouseUp takes it out of the queue
    let mut g = Rig::new();
    g.m.mem.write_u8(MB_STATE, 0).unwrap();
    g.m.post(2, 0, 1, 1);
    let mut p = Prog::new();
    p.clr_w().trap(0xA973).pop_w(RES).clr_w().trap(0xA977).pop_w(RES + 2).clr_w().trap(0xA977).pop_w(RES + 4);
    g.run(p.exit());
    assert_eq!((w(&g, RES) as u16) >> 8, 0, "StillDown FALSE: a mouseUp is queued");
    assert_eq!((w(&g, RES + 2) as u16) >> 8, 0, "WaitMouseUp FALSE and it took the mouseUp");
    assert_eq!((w(&g, RES + 4) as u16) >> 8, 1, "the queue is empty now: the button (MBState) is still down");
    // Button up
    let mut g = Rig::new();
    g.m.mem.write_u8(MB_STATE, 0x80).unwrap();
    g.run(Prog::new().clr_w().trap(0xA974).pop_w(RES).exit());
    assert_eq!((w(&g, RES) as u16) >> 8, 0);
}

trait IntoProg { fn into_prog(self) -> Prog; }
impl IntoProg for Vec<u8> { fn into_prog(self) -> Prog { Prog(self) } }

// ---- GetAppParms, the menu item accessors ----------------------------------------------------------------------------------

#[test]
fn menu_item_accessors_and_insert_menu_item_edit_the_menus_own_block() {
    let mut g = Rig::new();
    put(&mut g.m, RES + 0x40, &pstr("One;Two;Three"));
    put(&mut g.m, RES + 0x60, &pstr("Mid"));
    put(&mut g.m, RES + 0x70, &pstr("File"));
    let mut p = Prog::new();
    p.clr_l().w(300).l(RES + 0x70).trap(0xA931).pop_l(RES); // NewMenu(300, "File")
    p.rgn_arg(RES).l(RES + 0x40).trap(0xA933); // AppendMenu(menu, "One;Two;Three")
    p.rgn_arg(RES).w(2).ch(b'K').trap(0xA84F); // SetItemCmd(menu, 2, 'K')
    p.rgn_arg(RES).w(2).l(RES + 0x10).trap(0xA84E); // GetItemCmd(menu, 2, &c)
    p.rgn_arg(RES).w(3).ch(0x12).trap(0xA944); // SetItmMark(menu, 3, checkMark)
    p.rgn_arg(RES).w(3).l(RES + 0x12).trap(0xA943); // GetItmMark
    p.rgn_arg(RES).l(RES + 0x60).w(1).trap(0xA826); // InsertMenuItem(menu, "Mid", afterItem 1)
    p.rgn_arg(RES).w(2).l(RES + 0x80).trap(0xA946); // GetItem(menu, 2, &s)
    p.rgn_arg(RES).w(4).l(RES + 0x100).trap(0xA946); // GetItem(menu, 4): the old item 3
    p.rgn_arg(RES).w(4).l(RES + 0x14).trap(0xA943); // GetItmMark(menu, 4): the mark moved with its item
    p.rgn_arg(RES).w(9).ch(b'Z').trap(0xA84F); // NEGATIVE CONTROL: item 9 of 4
    g.run(p.exit());
    assert_eq!((g.m.mem.read_u8(RES + 0x10).unwrap(), g.m.mem.read_u8(RES + 0x12).unwrap()), (b'K', 0x12));
    assert_eq!(text(&g, RES + 0x80), "Mid", "inserted after item 1");
    assert_eq!(text(&g, RES + 0x100), "Three");
    assert_eq!(g.m.mem.read_u8(RES + 0x14).unwrap(), 0x12);
    assert!(g.m.refused.iter().any(|r| r.contains("SetItemCmd: menu 300 has 4 items, not 9")), "{:?}", g.m.refused);
}

#[test]
fn delete_menu_item_removes_the_item_moves_the_later_ones_up_and_their_enable_bits_with_them() {
    let mut g = Rig::new();
    put(&mut g.m, RES + 0x40, &pstr("One;Two;Three;Four"));
    put(&mut g.m, RES + 0x70, &pstr("File"));
    let mut p = Prog::new();
    p.clr_l().w(300).l(RES + 0x70).trap(0xA931).pop_l(RES); // NewMenu(300, "File")
    p.rgn_arg(RES).l(RES + 0x40).trap(0xA933); // AppendMenu(menu, "One;Two;Three;Four")
    p.rgn_arg(RES).w(3).trap(0xA93A); // DisableItem(menu, 3): "Three" is off
    p.rgn_arg(RES).w(2).trap(0xA952); // DeleteMenuItem(menu, 2): "Two" goes
    p.rgn_arg(RES).w(2).l(RES + 0x80).trap(0xA946); // GetItem(menu, 2): the old item 3
    p.rgn_arg(RES).w(9).trap(0xA952); // NEGATIVE CONTROL: item 9 of 3 is not an item
    p.rgn_arg(RES).w(0).trap(0xA952); // and neither is item 0
    p.rgn_arg(RES).w(3).l(RES + 0x100).trap(0xA946); // GetItem(menu, 3): "Four"
    g.run(p.exit());
    g.no_refusals();
    assert_eq!(text(&g, RES + 0x80), "Three");
    assert_eq!(text(&g, RES + 0x100), "Four");
    let h = g.m.mem.read_u32(RES).unwrap();
    let view = g.m.menu_view(h).unwrap();
    assert_eq!(view.items.len(), 3, "one item deleted; the two out-of-range calls deleted nothing");
    assert_eq!(view.flags & 0b1110, 0b1010, "Three (now item 2) is still disabled; One and Four (items 1 and 3) are enabled");
}

#[test]
fn get_app_parms_names_the_application_and_hands_over_an_empty_finder_startup_list() {
    let fixture = heirloom_mac68k_engine::fixture::test_app_macbinary();
    let mut m = heirloom_mac68k_engine::Mac::new(heirloom_ui_abi::RecordingGuestUi::new()).unwrap();
    m.launch(&fixture).unwrap();
    let h = m.mem.read_u32(heirloom_mac68k_engine::lowmem::APP_PARM_HANDLE).unwrap();
    assert_ne!(h, 0, "launch builds AppParmHandle");
    let b = m.handle_bytes(h).unwrap();
    assert_eq!(b, vec![0, 0, 0, 0], "message appOpen (0), no documents");
    // and the application's file is on the boot volume with its resource fork open as an FCB
    let name = m.mem.read_u8(heirloom_mac68k_engine::lowmem::CUR_AP_NAME).unwrap() as usize;
    assert!(name > 0);
    assert!(m.fm.open_fork(heirloom_mac68k_engine::resmgr::APP_REF_NUM).is_some_and(|f| f.rsrc && !f.perm.write));
}

#[test]
fn ae_get_attribute_ptr_answers_the_events_class_and_id_and_finds_nothing_else() {
    // SimpleText's open-application handler calls Pack8 selector $0E15 (14 argument words): AEGetAttributePtr(event, key, desiredType,
    // VAR typeCode, dataPtr, maxSize, VAR actualSize): OSErr. The event here is the 'aevt'/'oapp' this host delivers.
    const DESC: u32 = 0x3300;
    let mut g = Rig::new();
    let data = g.m.mm.new_handle(&mut g.m.mem, &mut g.m.heap, 8, false).unwrap();
    let p = g.m.mm.block(data).unwrap().ptr;
    g.m.mem.write_u32(p, four(b"aevt")).unwrap();
    g.m.mem.write_u32(p + 4, four(b"oapp")).unwrap();
    g.m.mem.write_u32(DESC, four(b"aevt")).unwrap();
    g.m.mem.write_u32(DESC + 4, data).unwrap();
    let call = |p: &mut Prog, key: &[u8; 4], desired: &[u8; 4], max: u32, at: u32| {
        p.clr_w().l(DESC).l(four(key)).l(four(desired)).l(at).l(at + 4).l(max).l(at + 8);
        dispatch(p, 0x0E15, 0xA816);
        p.pop_w(at + 12);
    };
    let mut p = Prog::new();
    p.l(0xCAFE_F00D);
    call(&mut p, b"evcl", b"****", 4, 0x3400);   // wildcard: any type
    call(&mut p, b"evid", b"type", 4, 0x3420);   // the attribute's own type
    call(&mut p, b"miss", b"****", 0, 0x3440);   // keyMissedKeywordAttr: nothing was missed because there are no parameters
    call(&mut p, b"evid", b"TEXT", 4, 0x3460);   // a type it cannot be coerced to
    call(&mut p, b"evcl", b"type", 2, 0x3480);   // a buffer shorter than the value gets the first bytes
    p.pop_l(0x34A0);
    g.run(p.exit());
    g.no_refusals();
    assert_eq!((l(&g, 0x3400), l(&g, 0x3404), l(&g, 0x3408), w(&g, 0x340C)), (four(b"type"), four(b"aevt"), 4, 0));
    assert_eq!((l(&g, 0x3424), l(&g, 0x3428), w(&g, 0x342C)), (four(b"oapp"), 4, 0));
    assert_eq!(w(&g, 0x344C), -1701, "errAEDescNotFound");
    assert_eq!(w(&g, 0x346C), -1700, "errAECoercionFail");
    assert_eq!((g.m.mem.read_u16(0x3484).unwrap(), w(&g, 0x348C)), (0x6165, 0), "two bytes of 'aevt'");
    assert_eq!(l(&g, 0x34A0), 0xCAFE_F00D, "every 14-word frame was popped exactly");
}
