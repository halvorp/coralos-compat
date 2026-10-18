//! The cheap walls the corpus census named after the File Manager: `_SysEnvirons`, `_DragWindow`, trap patching
//! (`_SetTrapAddress`), the alerts, the Disk Initialization Package (`_Pack2`) and the Scrap Manager. Each has its negative control.
mod common;
use common::{pstr, put, Prog, GLOBALS};
use guestui_abi::RecordingGuestUi;
use mac68k_engine::{Mac, Step};
use retro_resfork::writer::ForkBuilder;

fn machine() -> Mac<RecordingGuestUi> {
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    mac68k_engine::quickdraw::fill_globals(&mut m, GLOBALS).unwrap();
    m.qd_globals = GLOBALS;
    m
}
fn run(m: &mut Mac<RecordingGuestUi>, p: &Prog) {
    m.start(&p.0, 0).unwrap();
    let r = m.run_loop(1_000_000).unwrap();
    assert!(r.stats.halted && r.stats.bus_faults == 0, "{:?}\n{:#?}", r.stats, m.log);
}
fn w(m: &Mac<RecordingGuestUi>, a: u32) -> i16 { m.mem.read_u16(a).unwrap() as i16 }
fn l(m: &Mac<RecordingGuestUi>, a: u32) -> u32 { m.mem.read_u32(a).unwrap() }
fn words(ws: &[u16]) -> Vec<u8> { ws.iter().flat_map(|x| x.to_be_bytes()).collect() }

// ---- _SysEnvirons -----------------------------------------------------------------------------------------------------

#[test]
fn sys_environs_describes_the_machine_the_engine_is() {
    let mut m = machine();
    let mut p = Prog::new();
    // move.w #2,d0 ; lea $3100,a0 ; _SysEnvirons ; move.w d0,$3000
    p.0.extend(words(&[0x303C, 2, 0x41F9, 0, 0x3100, 0xA090, 0x33C0, 0, 0x3000]));
    p.exit();
    run(&mut m, &p);
    assert_eq!(w(&m, 0x3000), 0, "noErr");
    let rec: Vec<i16> = (0..8).map(|i| w(&m, 0x3100 + 2 * i)).collect();
    assert_eq!(rec[0], 2, "environsVersion");
    assert_eq!(rec[2] as u16, 0x0700, "systemVersion: System 7.0.0");
    assert_eq!(rec[3], 3, "processor: env68020");
    assert_eq!(m.mem.read_u8(0x3108).unwrap(), 0, "hasFPU: FALSE (not modelled)");
    assert_eq!(m.mem.read_u8(0x3109).unwrap(), 0, "hasColorQD: FALSE (1-bit QuickDraw)");
    assert_eq!(rec[7], -1, "sysVRefNum: the boot volume");
}

#[test]
fn sys_environs_refuses_a_version_it_cannot_honour_by_name() {
    for (ask, err, version) in [(0u16, -5501i16, None), (1, 0, Some(1i16)), (3, -5502, Some(2))] {
        let mut m = machine();
        put(&mut m, 0x3100, &[0xEE; 16]);
        let mut p = Prog::new();
        p.0.extend(words(&[0x303C, ask, 0x41F9, 0, 0x3100, 0xA090, 0x33C0, 0, 0x3000]));
        p.exit();
        run(&mut m, &p);
        assert_eq!(w(&m, 0x3000), err, "version {ask}");
        match version {
            Some(v) => assert_eq!(w(&m, 0x3100), v, "environsVersion filled with min(asked, 2)"),
            None => assert_eq!(m.mem.read_u8(0x3100).unwrap(), 0xEE, "a bad version leaves the record untouched"),
        }
    }
}

// ---- _DragWindow --------------------------------------------------------------------------------------------------------

#[test]
fn drag_window_returns_and_the_host_owns_the_move() {
    let rig = common::Rig::new();
    let mut m = rig.m;
    let mut p = Prog::new();
    p.l(rig.win).l(0x0040_0050).l(0x3000).trap(0xA925).exit();
    run(&mut m, &p);
    assert!(m.refused.is_empty(), "{:?}", m.refused);
    assert!(m.log.iter().any(|x| x.contains("DragWindow") && x.contains("host moves the window")), "{:#?}", m.log);
    // negative control: a window pointer the engine does not own is a NAMED refusal
    let mut m = machine();
    let mut p = Prog::new();
    p.l(0x9990).l(0).l(0x3000).trap(0xA925).exit();
    run(&mut m, &p);
    assert_eq!(m.refused, vec!["DragWindow: DragWindow: 0x9990 is not an open window".to_string()]);
}

// ---- _SetTrapAddress ------------------------------------------------------------------------------------------------------

/// OS trap `_FreeMem` ($A01C, number $1C) patched with a routine that counts and chains to the original through the
/// address `_GetOSTrapAddress` returned.
#[test]
fn a_patched_os_trap_runs_the_guest_routine_then_the_original() {
    let mut m = machine();
    let routine = 0x4400u32;
    // addq.l #1,$3010 ; movea.l $3014,a1 ; jmp (a1)
    put(&mut m, routine, &words(&[0x52B9, 0, 0x3010, 0x2279, 0, 0x3014, 0x4ED1]));
    let mut p = Prog::new();
    p.0.extend(words(&[
        0x303C, 0x1C, 0xA346, 0x23C8, 0, 0x3014,                     // d0 = $1C ; _GetOSTrapAddress ; save the original
        0x303C, 0x1C, 0x41F9, 0, routine as u16, 0xA247,              // lea routine,a0 ; _SetOSTrapAddress
        0xA01C, 0x23C0, 0, 0x3018,                                    // _FreeMem ; d0 -> $3018
        0x303C, 0x1C, 0x2079, 0, 0x3014, 0xA247,                      // movea.l $3014,a0 ; _SetOSTrapAddress: the original again
        0xA01C, 0x23C0, 0, 0x301C,                                    // _FreeMem again
    ]));
    p.exit();
    run(&mut m, &p);
    assert_eq!(l(&m, 0x3010), 1, "the patch ran exactly once: for the first FreeMem");
    assert!(l(&m, 0x3018) > 0 && l(&m, 0x3018) == l(&m, 0x301C), "and chained to the real FreeMem: the same free bytes both times");
    assert!(m.refused.is_empty(), "{:?}", m.refused);
}

/// Toolbox trap `_TickCount` ($A975): a PASCAL trap, so the stub it chains to is entered with the return address on top of
/// the (empty) frame.
#[test]
fn a_patched_pascal_trap_chains_through_the_stub_with_its_frame_intact() {
    let mut m = machine();
    let routine = 0x4400u32;
    m.ev.ticks = 4242;
    put(&mut m, routine, &words(&[0x52B9, 0, 0x3020, 0x2279, 0, 0x3024, 0x4ED1]));
    let mut p = Prog::new();
    p.0.extend(words(&[
        0x303C, 0x175, 0xA746, 0x23C8, 0, 0x3024,                     // _GetToolTrapAddress
        0x303C, 0x175, 0x41F9, 0, routine as u16, 0xA647,             // lea routine,a0 ; _SetToolTrapAddress
        0x42A7, 0xA975, 0x23DF, 0, 0x3028,                            // clr.l -(sp) ; _TickCount ; move.l (sp)+,$3028
    ]));
    p.exit();
    run(&mut m, &p);
    assert_eq!(l(&m, 0x3020), 1, "the patch ran");
    // CONTROL: the same call with no patch; the patched run must return exactly what the original does
    let mut c = machine();
    c.ev.ticks = 4242;
    let mut q = Prog::new();
    q.clr_l().trap(0xA975).pop_l(0x3028).exit();
    run(&mut c, &q);
    assert_eq!(l(&c, 0x3028), 4242);
    assert_eq!(l(&m, 0x3028), l(&c, 0x3028), "the original TickCount ran through the stub and its result reached the caller");
    assert_eq!(l(&c, 0x3020), 0, "control: no patch, no count");
    assert!(m.refused.is_empty(), "{:?}", m.refused);
}

#[test]
fn get_trap_address_after_a_patch_answers_the_patch() {
    let mut m = machine();
    let mut p = Prog::new();
    p.0.extend(words(&[0x303C, 0x1C, 0x41F9, 0, 0x4400, 0xA247, 0x303C, 0x1C, 0xA346, 0x23C8, 0, 0x3000]));
    p.exit();
    put(&mut m, 0x4400, &words(&[0x4E75]));
    run(&mut m, &p);
    assert_eq!(l(&m, 0x3000), 0x4400);
    assert!(m.patches.contains_key(&0xA01C), "the patch table holds it");
    // negative control: on a 32-bit machine an address outside RAM is refused by name and nothing is patched
    let mut m = machine();
    m.set_address_mode(mac68k_engine::addrmode::AddrMode::Bit32).unwrap();
    let mut p = Prog::new();
    p.0.extend(words(&[0x303C, 0x1C, 0x41F9, 0x7FFF, 0xFFF0, 0xA247]));
    p.exit();
    run(&mut m, &p);
    assert!(m.patches.is_empty() && m.refused.iter().any(|r| r.contains("SetTrapAddress routine")), "{:?}", m.refused);
}

// ---- alerts ---------------------------------------------------------------------------------------------------------------

fn alert_fork() -> Vec<u8> {
    let item = |top: i16, ty: u8, text: &str| {
        let mut v = vec![0, 0, 0, 0];
        for x in [top, 10, top + 20, 80] { v.extend(x.to_be_bytes()); }
        v.push(ty);
        v.push(text.len() as u8);
        v.extend(text.as_bytes());
        if text.len() % 2 == 1 { v.push(0); }
        v
    };
    let mut ditl = vec![0, 2]; // three items
    ditl.extend(item(60, 4, "OK"));
    ditl.extend(item(60, 4, "Cancel"));
    ditl.extend(item(10, 8, "Disk is full"));
    let mut alrt = vec![];
    for x in [60i16, 80, 180, 360] { alrt.extend(x.to_be_bytes()); }
    alrt.extend(300i16.to_be_bytes()); // DITL id
    alrt.extend([0x44, 0x44]); // stages
    ForkBuilder::new().add(b"ALRT", 200, None, &alrt).add(b"DITL", 300, None, &ditl).build()
}

fn alert_call(trap: u16, id: i16) -> Prog {
    let mut p = Prog::new();
    p.clr_w().w(id).clr_l().trap(trap).pop_w(0x3000).exit();
    p
}

#[test]
fn an_alert_blocks_shows_a_native_dialog_returns_the_item_hit_and_disposes_itself() {
    for trap in [0xA985u16, 0xA986, 0xA987, 0xA988] {
        let mut m = machine();
        m.rm.open_fork(mac68k_engine::resmgr::APP_REF_NUM, retro_resfork::ResFork::parse(alert_fork()).unwrap());
        m.start(&alert_call(trap, 200).0, 0).unwrap();
        assert_eq!(m.step(1_000_000).0, Step::Blocked, "{trap:#x}: the alert waits for the user: {:#?}", m.log);
        let win = m.tr.dlg.alert.expect("the alert dialog is up");
        assert_eq!(m.tr.dlg.dialogs[&win].list.len(), 3, "the DITL became the dialog's items (two buttons and static text)");
        assert!(m.windows[&win].native.is_some(), "it is shown natively");
        m.dialog_hit(win, 2); // the user presses Cancel
        assert_eq!(m.step(1_000_000).0, Step::Exited, "{:#?}", m.log);
        assert_eq!(w(&m, 0x3000), 2, "the Pascal result is the item number");
        assert!(m.tr.dlg.alert.is_none() && !m.windows.contains_key(&win), "the alert window is gone");
        assert!(m.refused.is_empty(), "{:?}", m.refused);
    }
}

#[test]
fn a_missing_alrt_returns_minus_one_without_blocking() {
    let mut m = machine();
    m.rm.open_fork(mac68k_engine::resmgr::APP_REF_NUM, retro_resfork::ResFork::parse(alert_fork()).unwrap());
    m.start(&alert_call(0xA985, 999).0, 0).unwrap();
    assert_eq!(m.step(1_000_000).0, Step::Exited, "{:#?}", m.log);
    assert_eq!(w(&m, 0x3000), -1);
    assert!(m.tr.dlg.alert.is_none());
}

// ---- _Pack2 --------------------------------------------------------------------------------------------------------------

#[test]
fn the_disk_initialization_package_loads_and_every_disk_touching_call_is_a_named_refusal() {
    let mut m = machine();
    let mut p = Prog::new();
    // DILoad ; DIFormat(1) -> $3000 ; DIVerify(1) -> $3002 ; DIZero(1, name) -> $3004 ; DIBadMount(pt, evt) -> $3006 ; DIUnload
    p.w(1).trap(0xA9E9);
    p.clr_w().w(1).w(6).trap(0xA9E9).pop_w(0x3000);
    p.clr_w().w(1).w(5).trap(0xA9E9).pop_w(0x3002);
    p.clr_w().w(1).l(0x3100).w(7).trap(0xA9E9).pop_w(0x3004);
    p.clr_w().l(0x0010_0010).l(0).w(0).trap(0xA9E9).pop_w(0x3006);
    p.w(2).trap(0xA9E9).exit();
    run(&mut m, &p);
    assert_eq!((w(&m, 0x3000), w(&m, 0x3002), w(&m, 0x3004), w(&m, 0x3006)), (-36, -36, -36, -128), "the frames were popped correctly and each result is honest");
    let names: Vec<&str> = m.refused.iter().map(|r| r.split(':').next().unwrap()).collect();
    assert_eq!(names, ["DIFormat", "DIVerify", "DIZero", "DIBadMount"], "DILoad and DIUnload are not refusals: {:?}", m.refused);
    // negative control: a selector that is not in the package is refused as such
    let mut m = machine();
    let mut p = Prog::new();
    p.w(9).trap(0xA9E9).exit();
    run(&mut m, &p);
    assert!(m.refused.iter().any(|r| r.contains("Pack2 selector 9")), "{:?}", m.refused);
}

// ---- the Scrap Manager --------------------------------------------------------------------------------------------------

#[test]
fn the_desk_scrap_holds_typed_data_and_zeroing_it_changes_the_count() {
    let mut m = machine();
    put(&mut m, 0x3200, b"clipped");
    let text = u32::from_be_bytes(*b"TEXT");
    let mut p = Prog::new();
    p.clr_l().trap(0xA9FC).pop_l(0x3000);                                          // ZeroScrap
    p.clr_l().l(7).l(text).l(0x3200).trap(0xA9FE).pop_l(0x3004);                    // PutScrap(7, 'TEXT', src)
    p.clr_l().trap(0xA9F9).pop_l(0x3008);                                          // InfoScrap -> PScrapStuff
    // GetScrap(NIL, 'TEXT', &offset) -> length ; GetScrap(NIL, 'PICT', &offset) -> noTypeErr
    p.clr_l().l(0).l(text).l(0x3300).trap(0xA9FD).pop_l(0x300C);
    p.clr_l().l(0).l(u32::from_be_bytes(*b"PICT")).l(0x3300).trap(0xA9FD).pop_l(0x3010);
    p.clr_l().trap(0xA9FA).pop_l(0x3014).clr_l().trap(0xA9FB).pop_l(0x3018);        // Unload / Load: noErr
    p.clr_l().trap(0xA9FC).pop_l(0x301C);                                          // ZeroScrap again
    p.clr_l().l(0).l(text).l(0x3300).trap(0xA9FD).pop_l(0x3020);                    // ... the text is gone
    p.exit();
    run(&mut m, &p);
    assert_eq!((l(&m, 0x3000), l(&m, 0x3004)), (0, 0), "ZeroScrap and PutScrap return noErr");
    assert_eq!(l(&m, 0x300C), 7, "GetScrap returns the length");
    assert_eq!(l(&m, 0x3010) as i32, -102, "a type that is not there: noTypeErr");
    assert_eq!((l(&m, 0x3014), l(&m, 0x3018)), (0, 0));
    assert_eq!(l(&m, 0x3020) as i32, -102, "after ZeroScrap the text is gone");
    let info = l(&m, 0x3008);
    assert_eq!(l(&m, info), 15, "scrapSize: 7 data bytes + the 8-byte type/length header");
    assert_eq!(w(&m, info + 8), 1, "scrapCount after the first ZeroScrap (the info record is live: re-read it below)");
    assert_eq!(m.scrap.count, 2, "two ZeroScraps");
    assert!(m.refused.is_empty(), "{:?}", m.refused);
}

#[test]
fn get_scrap_copies_into_a_handle_and_resizes_it() {
    let mut m = machine();
    put(&mut m, 0x3200, b"0123456789");
    let text = u32::from_be_bytes(*b"TEXT");
    let mut p = Prog::new();
    p.clr_l().trap(0xA9FC).pop_l(0x3000);
    p.clr_l().l(10).l(text).l(0x3200).trap(0xA9FE).pop_l(0x3004);
    // d0 = 2 ; _NewHandle ; the handle in A0 -> $3100 ; GetScrap(handle, 'TEXT', &off)
    p.0.extend(words(&[0x7002, 0xA122, 0x23C8, 0, 0x3100]));
    p.clr_l().rgn_arg(0x3100).l(text).l(0x3300).trap(0xA9FD).pop_l(0x3008);
    p.exit();
    run(&mut m, &p);
    let h = l(&m, 0x3100);
    assert_eq!(l(&m, 0x3008), 10);
    assert_eq!(m.handle_bytes(h).unwrap(), b"0123456789", "the handle grew from 2 to 10 bytes and holds the scrap");
    let _ = pstr("x");
}
