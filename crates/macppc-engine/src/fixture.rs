//! Test fragments built with our own PEF writer: a PowerPC "application" that does nothing but import, so the engine
//! can be exercised without a CPU. Layout: section 0 code (`li r3,0; blr`), section 1 data = the main TVector
//! `{code, toc}` at 0 (relocated by sectionC / sectionD) followed by one TOC word per import in import order (relocated
//! by an import run). The way code reaches an import is therefore the PowerPC way: load the TOC word, load the TVector's
//! code and toc, branch.
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
];

/// An application fragment importing `libs`, with `main` at data offset 0.
pub fn app(libs: &[Lib]) -> Vec<u8> {
    let total: usize = libs.iter().map(|l| l.2.len()).sum();
    let mut b = PefBuilder::new();
    b.code(&MAIN_CODE);
    let data = b.data(&[0; 8], TOC_FIRST + 4 * total as u32);
    for (name, weak, syms) in libs { b.import_library(name, *weak, syms); }
    let mut words = alloc::vec![0x4600u16]; // RelocTVector8 x1: main's code += sectionC, toc += sectionD
    let mut left = total;
    while left > 0 { let n = left.min(512); words.push(0x4A00 | (n as u16 - 1)); left -= n; } // RelocImportRun
    b.reloc(data as u16, &words);
    b.main(data as i32, 0);
    b.build()
}

/// The classic startup imports, all from `InterfaceLib`.
pub fn startup_app() -> Vec<u8> { app(&[("InterfaceLib", false, STARTUP_IMPORTS)]) }

/// The application's resource fork: a `cfrg` 0 naming the whole data fork as the PowerPC application fragment.
pub fn cfrg_fork() -> Vec<u8> { ForkBuilder::new().add(b"cfrg", 0, None, &cfrg::build(b"pwpc", cfrg::USAGE_APPLICATION, cfrg::WHERE_DATA_FORK, 0, 0, "Startup")).build() }

/// MacBinary II holding both forks.
pub fn macbinary_app(pef: &[u8]) -> Vec<u8> { macbinary("Startup", b"APPL", b"STUP", pef, &cfrg_fork()) }

/// AppleDouble header file (the data fork is a separate file).
pub fn appledouble_app() -> Vec<u8> { appledouble("Startup", &cfrg_fork()) }
