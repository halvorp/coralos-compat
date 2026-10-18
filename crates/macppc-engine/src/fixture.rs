//! Test fragments built with our own PEF writer. [`startup_app`] is a REAL tiny PowerPC application (assembled with
//! [`crate::asm`]): it runs the classic startup, opens a window, draws a string and waits for the close box. [`app`]
//! builds a do-nothing fragment that only imports. Layout: section 0 code, section 1 data = the main TVector
//! `{code, toc}` at 0 (relocated by sectionC / sectionD) followed by one TOC word per import in import order (relocated
//! by an import run). The way code reaches an import is therefore the PowerPC way: load the TOC word, load the TVector's
//! code and toc, branch.
use crate::asm::Asm;
use crate::cfrg;
use alloc::vec::Vec;
use retro_pef::loader::SymbolClass;
use retro_pef::writer::PefBuilder;
use retro_resfork::writer::{appledouble, macbinary, ForkBuilder};

/// `li r3,0` ; `blr`.
pub const MAIN_CODE: [u8; 8] = [0x38, 0x60, 0x00, 0x00, 0x4E, 0x80, 0x00, 0x20];
/// Byte offset of the first import's TOC word in the data section.
pub const TOC_FIRST: u32 = 8;

/// `(library, weak library, [(symbol, class, weak)])` as the builder takes them.
pub type Lib<'a> = (&'a str, bool, &'a [(&'a str, SymbolClass, bool)]);

/// The imports a classic Mac application makes in its startup and first window (names from the Universal Interfaces).
pub const STARTUP_IMPORTS: &[(&str, SymbolClass, bool)] = &[
    ("qd", SymbolClass::Data, false),
    ("InitGraf", SymbolClass::TVector, false), ("InitFonts", SymbolClass::TVector, false), ("InitWindows", SymbolClass::TVector, false),
    ("InitMenus", SymbolClass::TVector, false), ("TEInit", SymbolClass::TVector, false), ("InitDialogs", SymbolClass::TVector, false),
    ("InitCursor", SymbolClass::TVector, false), ("NewHandle", SymbolClass::TVector, false), ("GetResource", SymbolClass::TVector, false),
    ("GetNewWindow", SymbolClass::TVector, false), ("NewWindow", SymbolClass::TVector, false), ("ShowWindow", SymbolClass::TVector, false),
    ("SetPort", SymbolClass::TVector, false), ("WaitNextEvent", SymbolClass::TVector, false), ("MoveTo", SymbolClass::TVector, false),
    ("DrawString", SymbolClass::TVector, false), ("SysBeep", SymbolClass::TVector, false), ("ExitToShell", SymbolClass::TVector, false),
    ("FindWindow", SymbolClass::TVector, false), ("TrackGoAway", SymbolClass::TVector, false),
];

/// Where the data section keeps the program's constants, after the TOC (`TOC_FIRST + 4 * imports`).
const TITLE: &str = "PPC Hello";
const MESSAGE: &str = "Hello from PowerPC";
/// The window's content rectangle (top, left, bottom, right) in global coordinates.
pub const WINDOW_RECT: [i16; 4] = [60, 40, 140, 300];
/// Where the program moves the pen before DrawString (h, v).
pub const PEN: (i16, i16) = (20, 40);

fn toc(name: &str) -> i32 {
    let i = STARTUP_IMPORTS.iter().position(|s| s.0 == name).unwrap_or_else(|| panic!("fixture: {name} is not in STARTUP_IMPORTS"));
    (TOC_FIRST + 4 * i as u32) as i32
}

fn pstring(s: &str, out: &mut Vec<u8>) { out.push(s.len() as u8); out.extend_from_slice(s.as_bytes()); }

/// The real program and the constants it addresses through r2 (the data section base), as `(code, data tail)`.
fn startup_program() -> (Vec<u8>, Vec<u8>) {
    let tail_at = (TOC_FIRST + 4 * STARTUP_IMPORTS.len() as u32) as i32;
    let mut tail = Vec::new();
    for v in WINDOW_RECT { tail.extend_from_slice(&v.to_be_bytes()); }
    let title_at = tail_at + tail.len() as i32;
    pstring(TITLE, &mut tail);
    let msg_at = tail_at + tail.len() as i32;
    pstring(MESSAGE, &mut tail);
    let mut a = Asm::new();
    // prologue: save LR in the caller's frame, push a 96-byte frame (event record at 30, window pointer at 48, r31 at 92)
    a.mflr(0).stw(0, 8, 1).stwu(1, -96, 1).stw(31, 92, 1);
    // InitGraf(&qd.thePort): `qd` is a data import, its TOC word holds the globals' address; thePort is its last field
    a.lwz(3, toc("qd"), 2).addi(3, 3, super::machine::QD_GLOBALS_SIZE as i32 - 4).call_toc(toc("InitGraf"));
    for init in ["InitFonts", "InitWindows", "InitMenus", "TEInit"] { a.call_toc(toc(init)); }
    a.li(3, 0).call_toc(toc("InitDialogs")).call_toc(toc("InitCursor"));
    // NewWindow(nil, &rect, title, visible, documentProc, behind = -1, goAway, refCon)
    a.li(3, 0).addi(4, 2, tail_at).addi(5, 2, title_at).li(6, 1).li(7, 0).li(8, -1).li(9, 1).li(10, 0).call_toc(toc("NewWindow"));
    a.mr(31, 3).call_toc(toc("SetPort"));
    a.li(3, i32::from(PEN.0)).li(4, i32::from(PEN.1)).call_toc(toc("MoveTo")).addi(3, 2, msg_at).call_toc(toc("DrawString"));
    // event loop: WaitNextEvent(everyEvent, &event, 60, nil); only a mouseDown in the go-away box ends it
    a.label("loop").li(3, -1).addi(4, 1, 30).li(5, 60).li(6, 0).call_toc(toc("WaitNextEvent"));
    a.cmpwi(3, 0).beq("loop").lhz(0, 30, 1).cmpwi(0, 1).bne("loop");
    a.lwz(3, 40, 1).addi(4, 1, 48).call_toc(toc("FindWindow")).cmpwi(3, 6).bne("loop");
    a.lwz(3, 48, 1).lwz(4, 40, 1).call_toc(toc("TrackGoAway")).cmpwi(3, 0).beq("loop");
    a.li(3, 1).call_toc(toc("SysBeep")).call_toc(toc("ExitToShell"));
    // (not reached: ExitToShell ends the program) epilogue for completeness
    a.lwz(31, 92, 1).addi(1, 1, 96).lwz(0, 8, 1).mtlr(0).blr();
    (a.finish(), tail)
}


/// What [`dialog_app`] imports from `InterfaceLib`, in TOC order.
pub const DIALOG_IMPORTS: &[(&str, SymbolClass, bool)] = &[("GetNewDialog", SymbolClass::TVector, false), ("ModalDialog", SymbolClass::TVector, false), ("ExitToShell", SymbolClass::TVector, false)];
/// Byte offset (from r2, the data section base) of the word [`dialog_app`] leaves `ModalDialog`'s `itemHit` in.
pub const DIALOG_HIT_AT: i32 = (TOC_FIRST + 4 * 3) as i32;

/// A REAL PowerPC dialog application: `GetNewDialog(128, nil, -1)`, `ModalDialog(nil, &itemHit)`, store `itemHit` (a 16-bit
/// halfword) at [`DIALOG_HIT_AT`](r2), `ExitToShell`. Every instruction runs in retro-ppc; the Toolbox calls go through the glue.
pub fn dialog_app() -> Vec<u8> {
    let toc = |i: i32| (TOC_FIRST as i32) + 4 * i;
    let mut a = Asm::new();
    a.mflr(0).stw(0, 8, 1).stwu(1, -64, 1);
    a.li(3, 128).li(4, 0).li(5, -1).call_toc(toc(0)); // GetNewDialog(128, nil, behind = -1)
    a.li(3, 0).addi(4, 1, 30).call_toc(toc(1)); // ModalDialog(nil, &itemHit at 30(r1))
    a.lhz(0, 30, 1).sth(0, DIALOG_HIT_AT + 2, 2); // the low halfword of the big-endian word
    a.call_toc(toc(2)); // ExitToShell
    a.addi(1, 1, 64).lwz(0, 8, 1).mtlr(0).blr();
    app_with(&[("InterfaceLib", false, DIALOG_IMPORTS)], &a.finish(), &[0; 4])
}

/// An application fragment importing `libs`, with `main` at data offset 0 (`li r3,0; blr`).
pub fn app(libs: &[Lib]) -> Vec<u8> { app_with(libs, &MAIN_CODE, &[]) }

/// An application fragment importing `libs` whose code is `code` and whose data section is the main TVector, the TOC
/// (one word per import) and then `tail`.
pub fn app_with(libs: &[Lib], code: &[u8], tail: &[u8]) -> Vec<u8> {
    let total: usize = libs.iter().map(|l| l.2.len()).sum();
    let mut b = PefBuilder::new();
    b.code(code);
    let mut init = alloc::vec![0u8; (TOC_FIRST + 4 * total as u32) as usize];
    init.extend_from_slice(tail);
    let data = b.data(&init, init.len() as u32);
    for (name, weak, syms) in libs { b.import_library(name, *weak, syms); }
    let mut words = alloc::vec![0x4600u16]; // RelocTVector8 x1: main's code += sectionC, toc += sectionD
    let mut left = total;
    while left > 0 { let n = left.min(512); words.push(0x4A00 | (n as u16 - 1)); left -= n; } // RelocImportRun
    b.reloc(data as u16, &words);
    b.main(data as i32, 0);
    b.build()
}

/// The classic startup application: its imports all come from `InterfaceLib`.
pub fn startup_app() -> Vec<u8> {
    let (code, tail) = startup_program();
    app_with(&[("InterfaceLib", false, STARTUP_IMPORTS)], &code, &tail)
}

/// The application's resource fork: a `cfrg` 0 naming the whole data fork as the PowerPC application fragment.
pub fn cfrg_fork() -> Vec<u8> { ForkBuilder::new().add(b"cfrg", 0, None, &cfrg::build(b"pwpc", cfrg::USAGE_APPLICATION, cfrg::WHERE_DATA_FORK, 0, 0, "Startup")).build() }

/// MacBinary II holding both forks.
pub fn macbinary_app(pef: &[u8]) -> Vec<u8> { macbinary("Startup", b"APPL", b"STUP", pef, &cfrg_fork()) }

/// AppleDouble header file (the data fork is a separate file).
pub fn appledouble_app() -> Vec<u8> { appledouble("Startup", &cfrg_fork()) }
