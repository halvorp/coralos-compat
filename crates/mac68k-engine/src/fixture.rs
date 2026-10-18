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
