//! Test programs, loaded flat at [`crate::machine::CODE_BASE`] (entry offset 0).
//!
//! [`HELLO`] is `fixtures/hello.s` assembled with `m68k-linux-gnu-as -m68020` and `objcopy -O binary`
//! (`fixtures/hello.bin`): InitGraf, InitFonts, InitWindows, InitMenus, TEInit, InitDialogs, InitCursor,
//! NewWindow("Hello Mac", Rect 40,30,240,350), DisposeWindow, ExitToShell. The loaded image is a raw code blob;
//! a real program arrives as `CODE` resources in a resource fork (the next step).
pub static HELLO: &[u8] = include_bytes!("../fixtures/hello.bin");

/// Hand-encoded words: `_InitCursor`; push a word; `_SCSIDispatch` (a Toolbox trap no engine here implements: there is no SCSI bus; SysBeep used to be this example until the Sound Manager landed);
/// `_InitCursor`; `_ExitToShell`. Whether the second `_InitCursor` runs shows census mode (halts at the unknown
/// trap) against the default (a named refusal, execution continues).
pub static UNKNOWN_TRAP: &[u8] = &[
    0xA8, 0x50,             // _InitCursor
    0x3F, 0x3C, 0x00, 0x01, // move.w #1,-(sp)   (a stand-in argument; no signature is known for an unimplemented trap, so it stays on the stack)
    0xA8, 0x15,             // _SCSIDispatch
    0xA8, 0x50,             // _InitCursor
    0xA9, 0xF4,             // _ExitToShell
];

/// `fixtures/testapp/memtest.s`: Memory Manager OS traps with register conventions; results at 0x3000...
pub static MEMTEST: &[u8] = include_bytes!("../fixtures/testapp/memtest.bin");
/// `fixtures/testapp/restest.s`: Resource Manager Toolbox traps; results at 0x3000... Needs [`restest_fork`] loaded.
pub static RESTEST: &[u8] = include_bytes!("../fixtures/testapp/restest.bin");

/// The resources `restest.s` reads: TEXT 128 "Greeting" = "hello", TEXT 129 "Second" = "world!".
pub fn restest_fork() -> alloc::vec::Vec<u8> {
    heirloom_fmt_resfork::writer::ForkBuilder::new().add(b"TEXT", 128, Some("Greeting"), b"hello").add(b"TEXT", 129, Some("Second"), b"world!").build()
}

/// The test application's resource fork (`fixtures/testapp/main.s` is CODE 1, `menus.s` CODE 2): CODE 0's jump table,
/// MENU 128 (Apple) / 129 (File: New, Quit) and MBAR 128, WIND 128.
pub fn test_app_fork() -> alloc::vec::Vec<u8> {
    let menu = |id: i16, title: &[u8], items: &[(&str, u8)]| {
        let mut m = alloc::vec::Vec::new();
        m.extend_from_slice(&id.to_be_bytes());
        m.extend_from_slice(&[0; 8]); // width, height, MDEF id, filler
        m.extend_from_slice(&0xFFFF_FFFFu32.to_be_bytes());
        m.push(title.len() as u8);
        m.extend_from_slice(title);
        for (t, key) in items { m.push(t.len() as u8); m.extend_from_slice(t.as_bytes()); m.extend_from_slice(&[0, *key, 0, 0]); }
        m.push(0);
        m
    };
    let mut code0 = alloc::vec::Vec::new();
    for v in [48u32, 512, 16, 32] { code0.extend_from_slice(&v.to_be_bytes()); } // aboveA5, belowA5, jtSize, jtOffset
    for seg in [1u16, 2] { code0.extend_from_slice(&[0, 0, 0x3F, 0x3C, 0, seg as u8, 0xA9, 0xF0]); } // offset 0, move.w #seg,-(sp), _LoadSeg
    let code = |first: u16, body: &[u8]| { let mut c = alloc::vec::Vec::new(); c.extend_from_slice(&first.to_be_bytes()); c.extend_from_slice(&1u16.to_be_bytes()); c.extend_from_slice(body); c };
    let mut wind = alloc::vec::Vec::new();
    for v in [60i16, 40, 260, 360, 0, 0x0100, 0x0100] { wind.extend_from_slice(&v.to_be_bytes()); } // rect, procID, visible, goAway
    wind.extend_from_slice(&0x1234u32.to_be_bytes());
    wind.extend_from_slice(b"\x0bTest Window");
    let mbar: alloc::vec::Vec<u8> = [2i16, 128, 129].iter().flat_map(|v| v.to_be_bytes()).collect();
    heirloom_fmt_resfork::writer::ForkBuilder::new()
        .add(b"CODE", 0, None, &code0)
        .add(b"CODE", 1, Some("Main"), &code(0, TEST_APP_CODE1))
        .add(b"CODE", 2, Some("Menus"), &code(8, TEST_APP_CODE2))
        .add(b"WIND", 128, None, &wind)
        .add(b"MBAR", 128, None, &mbar)
        .add(b"MENU", 128, None, &menu(128, &[0x14], &[("About Test App...", 0), ("-", 0)]))
        .add(b"MENU", 129, None, &menu(129, b"File", &[("New", b'N'), ("Quit", b'Q')]))
        .build()
}

pub static TEST_APP_CODE1: &[u8] = include_bytes!("../fixtures/testapp/code1.bin");
pub static TEST_APP_CODE2: &[u8] = include_bytes!("../fixtures/testapp/code2.bin");

/// The test application as a MacBinary II file (type APPL, creator TEST), the way it would arrive from a download.
pub fn test_app_macbinary() -> alloc::vec::Vec<u8> {
    heirloom_fmt_resfork::writer::macbinary("Test App", b"APPL", b"TEST", &[], &test_app_fork())
}

/// `fixtures/testapp/evtest.s`: Event Manager and hit testing; see the header of the source for the cells it uses.
pub static EVTEST: &[u8] = include_bytes!("../fixtures/testapp/evtest.bin");

/// `fixtures/testapp/recttest.s`: QuickDraw Rect/Point utilities; results at 0x3000... .
pub static RECTTEST: &[u8] = include_bytes!("../fixtures/testapp/recttest.bin");

/// `fixtures/testapp/qdapp.s`: an event-loop application that draws its window with QuickDraw on every updateEvt.
pub static QDAPP: &[u8] = include_bytes!("../fixtures/testapp/qdapp.bin");

/// `fixtures/testapp/oracle.s`: draws a fixed set of QuickDraw primitives, one per 40x40 cell, on its first update event.
pub static ORACLE: &[u8] = include_bytes!("../fixtures/testapp/oracle.bin");

/// Window content origin (global) and size of the oracle application's window (WIND 128).
pub const ORACLE_TOP: i16 = 27;
pub const ORACLE_LEFT: i16 = 61;
pub const ORACLE_W: i16 = 320;
pub const ORACLE_H: i16 = 280;

/// The oracle application as a MacBinary II file: CODE 0 (one jump-table entry for segment 1), CODE 1, WIND 128 (a plain
/// bordered window, procID 3, so the content rect is exactly [`ORACLE_LEFT`], [`ORACLE_TOP`] + size).
pub fn oracle_app_macbinary() -> alloc::vec::Vec<u8> {
    let mut code0 = alloc::vec::Vec::new();
    for v in [40u32, 512, 8, 32] { code0.extend_from_slice(&v.to_be_bytes()); } // aboveA5, belowA5, jtSize, jtOffset
    code0.extend_from_slice(&[0, 0, 0x3F, 0x3C, 0, 1, 0xA9, 0xF0]); // offset 0, move.w #1,-(sp), _LoadSeg
    let mut code1 = alloc::vec::Vec::new();
    code1.extend_from_slice(&0u16.to_be_bytes()); // first jump-table entry
    code1.extend_from_slice(&1u16.to_be_bytes()); // one entry
    code1.extend_from_slice(ORACLE);
    let mut wind = alloc::vec::Vec::new();
    for v in [ORACLE_TOP, ORACLE_LEFT, ORACLE_TOP + ORACLE_H, ORACLE_LEFT + ORACLE_W, 3, 0x0100, 0x0000] { wind.extend_from_slice(&v.to_be_bytes()); }
    wind.extend_from_slice(&0u32.to_be_bytes());
    wind.extend_from_slice(b"\x06Oracle");
    let fork = heirloom_fmt_resfork::writer::ForkBuilder::new()
        .add(b"CODE", 0, None, &code0).add(b"CODE", 1, Some("Main"), &code1).add(b"WIND", 128, None, &wind).build();
    heirloom_fmt_resfork::writer::macbinary("QD Oracle", b"APPL", b"QDOR", &[], &fork)
}

// ---- the Dialog Manager / Control Manager script: ONE call list, run as 68k code AND through the PowerPC glue ----

/// An argument of a scripted Toolbox call: a literal, or the word/long currently stored at a guest address (a result an earlier call saved).
#[derive(Debug, Clone, Copy)]
pub enum ScriptArg { Lit(u32), Mem(u32) }

/// One Toolbox call by name. `save`: where the result goes (a long, a word, or a Boolean byte in the high half of a word, as a 68k
/// `pop` leaves it), so the two CPUs leave the same bytes in guest memory.
#[derive(Debug, Clone)]
pub struct ScriptCall { pub name: &'static str, pub args: alloc::vec::Vec<ScriptArg>, pub save: Option<u32> }

/// Where [`dialog_script`] keeps its cells.
pub mod cells {
    pub const DIALOG: u32 = 0x3000; // long
    pub const HIT1: u32 = 0x3010; // word
    pub const TYPE: u32 = 0x3012; // word
    pub const ITEM: u32 = 0x3014; // long
    pub const BOX: u32 = 0x3018; // Rect, 8 bytes
    pub const TEXT: u32 = 0x3100; // Str255 read back from the edit field
    pub const HIT2: u32 = 0x3020; // word
    pub const CHECK: u32 = 0x3022; // word: GetControlValue of the check box
    pub const CHECK_ITEM: u32 = 0x3024; // long
    pub const NEW_TEXT: u32 = 0x3200; // Str255 the program writes into the edit field
}

/// A dialog application's resources: `DLOG` 128 (dBoxProc, visible), `DITL` 128: 1 OK button, 2 Cancel button, 3 check box
/// "Remember", 4 static text "Name:", 5 edit text "Hello", 6 a disabled user item.
pub fn dialog_fork() -> alloc::vec::Vec<u8> { dialog_fork_with(false) }

/// [`dialog_fork`] plus a `dctb` 128 (a dialog colour table): on a machine with Color QuickDraw `GetNewDialog` then makes a COLOUR window.
pub fn color_dialog_fork() -> alloc::vec::Vec<u8> { dialog_fork_with(true) }

fn dialog_fork_with(dctb: bool) -> alloc::vec::Vec<u8> {
    let item = |rect: [i16; 4], ty: u8, data: &[u8]| {
        let mut v = alloc::vec![0u8; 4];
        for r in rect { v.extend_from_slice(&r.to_be_bytes()); }
        v.push(ty);
        v.push(data.len() as u8);
        v.extend_from_slice(data);
        if data.len() & 1 == 1 { v.push(0); }
        v
    };
    // A DITL item's data is the text itself (its length byte counts it); only the DLOG title is a Pascal string.
    let ps = |s: &str| { let mut v = alloc::vec![s.len() as u8]; v.extend_from_slice(s.as_bytes()); v };
    let mut ditl = 5i16.to_be_bytes().to_vec(); // count - 1
    for it in [item([80, 200, 100, 260], 4, "OK".as_bytes()), item([80, 120, 100, 180], 4, "Cancel".as_bytes()), item([50, 20, 66, 140], 5, "Remember".as_bytes()),
               item([10, 20, 26, 200], 8, "Name:".as_bytes()), item([30, 20, 46, 200], 16, "Hello".as_bytes()), item([60, 220, 76, 260], 128, &[])] { ditl.extend_from_slice(&it); }
    let mut dlog = alloc::vec::Vec::new();
    for v in [60i16, 40, 180, 300, 1, 0x0100, 0x0000] { dlog.extend_from_slice(&v.to_be_bytes()); } // rect, procID, visible, goAway
    dlog.extend_from_slice(&0u32.to_be_bytes()); // refCon
    dlog.extend_from_slice(&128i16.to_be_bytes()); // itemsID
    dlog.extend_from_slice(&ps("Dialog"));
    let mut b = heirloom_fmt_resfork::writer::ForkBuilder::new().add(b"DLOG", 128, None, &dlog).add(b"DITL", 128, None, &ditl);
    if dctb { b = b.add(b"dctb", 128, None, &[0u8; 8]); } // an empty window colour table: its presence is what the Dialog Manager looks for
    b.build()
}

/// The dialog program: open `DLOG` 128, wait for a hit, read item 5's text, the check box and the item's type/box, rewrite the
/// edit text, wait for a second hit, dispose. Calls 1 and 6 (`ModalDialog`) are where the host acts.
pub fn dialog_script() -> alloc::vec::Vec<ScriptCall> {
    use ScriptArg::{Lit, Mem};
    use cells::*;
    let c = |name, args: &[ScriptArg], save| ScriptCall { name, args: args.to_vec(), save };
    alloc::vec![
        c("GetNewDialog", &[Lit(128), Lit(0), Lit(0xFFFF_FFFF)], Some(DIALOG)),
        c("ModalDialog", &[Lit(0), Lit(HIT1)], None),
        c("GetDialogItem", &[Mem(DIALOG), Lit(5), Lit(TYPE), Lit(ITEM), Lit(BOX)], None),
        c("GetDialogItemText", &[Mem(ITEM), Lit(TEXT)], None),
        c("SetDialogItemText", &[Mem(ITEM), Lit(NEW_TEXT)], None),
        c("GetDialogItem", &[Mem(DIALOG), Lit(3), Lit(0x3040), Lit(CHECK_ITEM), Lit(0x3044)], None),
        c("ModalDialog", &[Lit(0), Lit(HIT2)], None),
        c("GetControlValue", &[Mem(CHECK_ITEM)], Some(CHECK)),
        c("DisposeDialog", &[Mem(DIALOG)], None),
    ]
}

/// What [`dialog_script`] left behind, with every host address and token left out, so the 68k run and the PowerPC run can be
/// compared line for line: the cells, then each host call in order (kind, label, geometry and value; never a ref or token).
pub fn dialog_observation(m: &crate::Mac<heirloom_ui_abi::RecordingGuestUi>) -> alloc::vec::Vec<alloc::string::String> {
    use alloc::format;
    use heirloom_ui_abi::UiCall;
    let w = |a: u32| m.mem.read_u16(a).map_or(-1, i32::from);
    let pstr = |a: u32| m.mem.read_u8(a).map_or("?".into(), |n| (0..u32::from(n)).map(|i| char::from(m.mem.read_u8(a + 1 + i).unwrap_or(b'?'))).collect::<alloc::string::String>());
    let mut out = alloc::vec![
        format!("hit1={} type={} box=({},{},{},{})", w(cells::HIT1), w(cells::TYPE), w(cells::BOX), w(cells::BOX + 2), w(cells::BOX + 4), w(cells::BOX + 6)),
        format!("text={:?} check={} hit2={}", pstr(cells::TEXT), w(cells::CHECK), w(cells::HIT2)),
    ];
    for c in &m.ui.calls {
        out.push(match c {
            UiCall::OpenWindow(x) => format!("open {:?} {:?} {:?}", x.title, x.rect, x.role),
            UiCall::AddControl(_, x) => format!("add {:?} {:?} {:?} enabled={}", x.kind, x.label, x.rect, x.enabled),
            UiCall::SetControl(_, _, v) => format!("set {v:?}"),
            UiCall::RemoveControl(..) => "remove".into(),
            UiCall::PresentPlane(_, p) => format!("plane {:?} {} op(s)", p.rect, p.ops.len()),
            UiCall::CloseWindow(_) => "close".into(),
            other => format!("{other:?}"),
        });
    }
    out
}

/// The observation every run of [`dialog_script`] must produce (68k code, PowerPC glue): the first `ModalDialog` returned the
/// edit item (5: the user typed "World", read back by the program), the second returned item 3 (the check box was clicked and
/// its value left alone, as on a Mac).
pub static DIALOG_EXPECTED: &[&str] = &[
    "hit1=5 type=16 box=(30,20,46,200)",
    "text=\"World\" check=0 hit2=3",
    "open \"Dialog\" GuestRect { left: 40, top: 60, width: 260, height: 120 } Dialog",
    "add Button \"OK\" GuestRect { left: 200, top: 80, width: 60, height: 20 } enabled=true",
    "add Button \"Cancel\" GuestRect { left: 120, top: 80, width: 60, height: 20 } enabled=true",
    "add Checkbox { checked: false } \"Remember\" GuestRect { left: 20, top: 50, width: 120, height: 16 } enabled=true",
    "add Label { text: \"Name:\", framed: false } \"\" GuestRect { left: 20, top: 10, width: 180, height: 16 } enabled=true",
    "add Field { text: \"Hello\", max_len: 255, editable: true, integer: false } \"\" GuestRect { left: 20, top: 30, width: 180, height: 16 } enabled=true",
    "plane GuestRect { left: 0, top: 0, width: 260, height: 120 } 0 op(s)",
    "set Text(\"Changed\")",
    "set Checked(false)",
    "close",
];
