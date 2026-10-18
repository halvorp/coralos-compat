//! Test programs, loaded flat at [`crate::machine::CODE_BASE`] (entry offset 0).
//!
//! [`HELLO`] is `fixtures/hello.s` assembled with `m68k-linux-gnu-as -m68020` and `objcopy -O binary`
//! (`fixtures/hello.bin`): InitGraf, InitFonts, InitWindows, InitMenus, TEInit, InitDialogs, InitCursor,
//! NewWindow("Hello Mac", Rect 40,30,240,350), DisposeWindow, ExitToShell. The loaded image is a raw code blob;
//! a real program arrives as `CODE` resources in a resource fork (the next step).
pub static HELLO: &[u8] = include_bytes!("../fixtures/hello.bin");

/// Hand-encoded words: `_InitCursor`; push a word; `_SysBeep` (a Toolbox trap this skeleton does not implement);
/// `_InitCursor`; `_ExitToShell`. Whether the second `_InitCursor` runs shows census mode (halts at the unknown
/// trap) against the default (a named refusal, execution continues).
pub static UNKNOWN_TRAP: &[u8] = &[
    0xA8, 0x50,             // _InitCursor
    0x3F, 0x3C, 0x00, 0x01, // move.w #1,-(sp)   (SysBeep's duration; no signature known, so it stays on the stack)
    0xA9, 0xC8,             // _SysBeep
    0xA8, 0x50,             // _InitCursor
    0xA9, 0xF4,             // _ExitToShell
];

/// `fixtures/testapp/memtest.s`: Memory Manager OS traps with register conventions; results at 0x3000...
pub static MEMTEST: &[u8] = include_bytes!("../fixtures/testapp/memtest.bin");
/// `fixtures/testapp/restest.s`: Resource Manager Toolbox traps; results at 0x3000... Needs [`restest_fork`] loaded.
pub static RESTEST: &[u8] = include_bytes!("../fixtures/testapp/restest.bin");

/// The resources `restest.s` reads: TEXT 128 "Greeting" = "hello", TEXT 129 "Second" = "world!".
pub fn restest_fork() -> alloc::vec::Vec<u8> {
    retro_resfork::writer::ForkBuilder::new().add(b"TEXT", 128, Some("Greeting"), b"hello").add(b"TEXT", 129, Some("Second"), b"world!").build()
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
    retro_resfork::writer::ForkBuilder::new()
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
    retro_resfork::writer::macbinary("Test App", b"APPL", b"TEST", &[], &test_app_fork())
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
    let fork = retro_resfork::writer::ForkBuilder::new()
        .add(b"CODE", 0, None, &code0).add(b"CODE", 1, Some("Main"), &code1).add(b"WIND", 128, None, &wind).build();
    retro_resfork::writer::macbinary("QD Oracle", b"APPL", b"QDOR", &[], &fork)
}
