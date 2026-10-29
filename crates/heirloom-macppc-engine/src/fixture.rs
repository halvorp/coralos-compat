//! Test fragments built with our own PEF writer. [`startup_app`] is a REAL tiny PowerPC application (assembled with
//! [`crate::asm`]): it runs the classic startup, opens a window, draws a string and waits for the close box. [`app`]
//! builds a do-nothing fragment that only imports. Layout: section 0 code, section 1 data = the main TVector
//! `{code, toc}` at 0 (relocated by sectionC / sectionD) followed by one TOC word per import in import order (relocated
//! by an import run). The way code reaches an import is therefore the PowerPC way: load the TOC word, load the TVector's
//! code and toc, branch.
use crate::asm::Asm;
use crate::cfrg;
use alloc::vec::Vec;
use heirloom_fmt_pef::loader::SymbolClass;
use heirloom_fmt_pef::writer::PefBuilder;
use heirloom_fmt_resfork::writer::{appledouble, macbinary, ForkBuilder};

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
/// halfword) at [`DIALOG_HIT_AT`](r2), `ExitToShell`. Every instruction runs in heirloom-cpu-ppc; the Toolbox calls go through the glue.
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

/// What [`gl_spinner_app`] imports from `InterfaceLib` (TOC words first) and `OpenGLLibrary` (after them), in TOC order.
pub const SPINNER_TOOLBOX: &[(&str, SymbolClass, bool)] = &[
    ("qd", SymbolClass::Data, false),
    ("InitGraf", SymbolClass::TVector, false), ("InitFonts", SymbolClass::TVector, false), ("InitWindows", SymbolClass::TVector, false),
    ("InitMenus", SymbolClass::TVector, false), ("TEInit", SymbolClass::TVector, false), ("InitDialogs", SymbolClass::TVector, false),
    ("InitCursor", SymbolClass::TVector, false), ("NewWindow", SymbolClass::TVector, false), ("SetPort", SymbolClass::TVector, false),
    ("WaitNextEvent", SymbolClass::TVector, false), ("FindWindow", SymbolClass::TVector, false), ("TrackGoAway", SymbolClass::TVector, false),
    ("ExitToShell", SymbolClass::TVector, false),
];
pub const SPINNER_GL: &[(&str, SymbolClass, bool)] = &[
    ("aglChoosePixelFormat", SymbolClass::TVector, false), ("aglCreateContext", SymbolClass::TVector, false),
    ("aglSetCurrentContext", SymbolClass::TVector, false), ("aglSetDrawable", SymbolClass::TVector, false),
    ("aglSwapBuffers", SymbolClass::TVector, false), ("glViewport", SymbolClass::TVector, false),
    ("glEnableClientState", SymbolClass::TVector, false), ("glVertexPointer", SymbolClass::TVector, false),
    ("glColorPointer", SymbolClass::TVector, false), ("glClear", SymbolClass::TVector, false), ("glDrawElements", SymbolClass::TVector, false),
];
/// The spinner window's content rectangle (top, left, bottom, right) in global coordinates: 192 x 128.
pub const SPINNER_RECT: [i16; 4] = [80, 60, 208, 252];

/// A REAL PowerPC OpenGL application (guest-GL on-screen slice 5's witness on the Z83): the classic startup, ONE window, an AGL
/// context whose drawable is that window, then a loop that draws one vertex-coloured triangle from big-endian arrays, changes
/// the first vertex's red/green bytes every frame (a picture that visibly changes), swaps, and waits in `WaitNextEvent` (sleep 1
/// tick) - until a click in the window's go-away box. Every instruction runs on heirloom-cpu-ppc; every GL call goes through
/// the engine's GL glue to the worker.
pub fn gl_spinner_app() -> Vec<u8> {
    let n_tb = SPINNER_TOOLBOX.len();
    let toc = |name: &str| -> i32 {
        let i = SPINNER_TOOLBOX.iter().position(|s| s.0 == name).or_else(|| SPINNER_GL.iter().position(|s| s.0 == name).map(|i| i + n_tb));
        (TOC_FIRST + 4 * i.unwrap_or_else(|| panic!("spinner: {name} is not imported")) as u32) as i32
    };
    let tail_at = (TOC_FIRST + 4 * (n_tb + SPINNER_GL.len()) as u32) as i32;
    let mut tail = Vec::new();
    let mut put = |b: &[u8], tail: &mut Vec<u8>| -> i32 {
        while tail.len() % 4 != 0 { tail.push(0); }
        let at = tail_at + tail.len() as i32;
        tail.extend_from_slice(b);
        at
    };
    let rect: Vec<u8> = SPINNER_RECT.iter().flat_map(|v| v.to_be_bytes()).collect();
    let rect_at = put(&rect, &mut tail);
    let mut title = Vec::new();
    pstring("GL Spinner", &mut title);
    let title_at = put(&title, &mut tail);
    let verts: Vec<u8> = [-0.8f32, -0.7, 0.0, 0.8, -0.7, 0.0, 0.0, 0.8, 0.0].iter().flat_map(|v| v.to_be_bytes()).collect();
    let verts_at = put(&verts, &mut tail);
    let cols_at = put(&[255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255], &mut tail);
    let idx: Vec<u8> = [0u16, 1, 2].iter().flat_map(|v| v.to_be_bytes()).collect();
    let idx_at = put(&idx, &mut tail);
    let (w, h) = (i32::from(SPINNER_RECT[3] - SPINNER_RECT[1]), i32::from(SPINNER_RECT[2] - SPINNER_RECT[0]));

    let mut a = Asm::new();
    // prologue: LR in the caller's frame, a 112-byte frame (event record at 30, window found at 48, r28..r31 at 96..108)
    a.mflr(0).stw(0, 8, 1).stwu(1, -112, 1).stw(31, 108, 1).stw(30, 104, 1).stw(29, 100, 1).stw(28, 96, 1);
    a.lwz(3, toc("qd"), 2).addi(3, 3, super::machine::QD_GLOBALS_SIZE as i32 - 4).call_toc(toc("InitGraf"));
    for init in ["InitFonts", "InitWindows", "InitMenus", "TEInit"] { a.call_toc(toc(init)); }
    a.li(3, 0).call_toc(toc("InitDialogs")).call_toc(toc("InitCursor"));
    a.li(3, 0).addi(4, 2, rect_at).addi(5, 2, title_at).li(6, 1).li(7, 0).li(8, -1).li(9, 1).li(10, 0).call_toc(toc("NewWindow"));
    a.mr(30, 3).call_toc(toc("SetPort"));
    a.li(3, 0).li(4, 0).li(5, 0).call_toc(toc("aglChoosePixelFormat")).mr(29, 3);
    a.mr(3, 29).li(4, 0).call_toc(toc("aglCreateContext")).mr(28, 3);
    a.mr(3, 28).call_toc(toc("aglSetCurrentContext"));
    a.mr(3, 28).mr(4, 30).call_toc(toc("aglSetDrawable"));
    a.li(3, 0).li(4, 0).li(5, w).li(6, h).call_toc(toc("glViewport"));
    a.lis(3, 0).ori(3, 3, 0x8074).call_toc(toc("glEnableClientState")); // GL_VERTEX_ARRAY
    a.lis(3, 0).ori(3, 3, 0x8076).call_toc(toc("glEnableClientState")); // GL_COLOR_ARRAY
    a.li(3, 3).li(4, 0x1406).li(5, 0).addi(6, 2, verts_at).call_toc(toc("glVertexPointer"));
    a.li(3, 4).li(4, 0x1401).li(5, 0).addi(6, 2, cols_at).call_toc(toc("glColorPointer"));
    a.li(31, 0);
    a.label("frame").addi(31, 31, 5).sth(31, cols_at, 2); // vertex 0's red, green = the frame counter's two low bytes
    a.li(3, 0x4000).call_toc(toc("glClear")); // GL_COLOR_BUFFER_BIT
    a.li(3, 4).li(4, 3).li(5, 0x1403).addi(6, 2, idx_at).call_toc(toc("glDrawElements"));
    a.mr(3, 28).call_toc(toc("aglSwapBuffers"));
    a.label("wait").li(3, -1).addi(4, 1, 30).li(5, 1).li(6, 0).call_toc(toc("WaitNextEvent"));
    a.cmpwi(3, 0).beq("frame").lhz(0, 30, 1).cmpwi(0, 1).bne("frame");
    a.lwz(3, 40, 1).addi(4, 1, 48).call_toc(toc("FindWindow")).cmpwi(3, 6).bne("frame");
    a.lwz(3, 48, 1).lwz(4, 40, 1).call_toc(toc("TrackGoAway")).cmpwi(3, 0).beq("frame");
    a.call_toc(toc("ExitToShell"));
    a.lwz(28, 96, 1).lwz(29, 100, 1).lwz(30, 104, 1).lwz(31, 108, 1).addi(1, 1, 112).lwz(0, 8, 1).mtlr(0).blr();
    let pef = app_with(&[("InterfaceLib", false, SPINNER_TOOLBOX), ("OpenGLLibrary", false, SPINNER_GL)], &a.finish(), &tail);
    macbinary("GL Spinner", b"APPL", b"GLSP", &pef, &cfrg_fork())
}
