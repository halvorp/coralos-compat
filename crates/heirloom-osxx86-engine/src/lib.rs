//! osxx86: the Mac OS X x86_64 personality of the Heirloom compat layer (docs/heirloom/25-OSXX86-ENGINE.md in coralos).
//!
//! A Mach-O x86_64 application's own code runs NATIVELY on the CPU: there is no CPU library. What the engine provides is the
//! operating system the app thinks it is on, as native functions:
//!
//! * [`load`]: maps the image into engine-owned memory, applies the slide, binds every import to [`libsystem`], and publishes
//!   the pages with their final protections (never writable and executable at once);
//! * [`trap`]: the admission scan. The guest must never execute a raw `syscall`, `sysenter` or `int 0x80`: its system calls go
//!   through the bound table. An image whose final executable bytes contain one of those encodings is refused before it runs;
//! * [`libsystem`]: the native table the imports bind to (rung 1: `write`, `exit`);
//! * [`enter`]: entry on the guest's own stack and an exit that restores the engine's registers instead of unwinding;
//! * [`run`]: the whole sequence, with a [`Host`] for console output and markers and a [`CodeMem`] for pages.
//!
//! Every refusal is a named [`Refusal`] with a marker line (`[OSXX86 REFUSE <NAME> ...]`); nothing is silently skipped.
#![cfg_attr(not(test), no_std)]
extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use heirloom_fmt_macho::MachoError;

pub mod enter;
pub mod libsystem;
pub mod load;
pub mod run;
pub mod trap;
#[cfg(any(test, feature = "host-mem"))]
pub mod hostmem;

pub use run::{run, Outcome};
#[cfg(test)]
mod tests;

/// The final protection of a published range. There is deliberately no write-and-execute variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prot {
    None,
    R,
    RW,
    RX,
}

/// Where the engine gets pages. Reservations are page aligned, zero filled, read-write and never executable; `publish` applies
/// each range's final protection. The product engine implements it over the OS's code-memory grant; hosts over mmap.
pub trait CodeMem {
    /// `len` bytes (rounded up to pages) of zeroed read-write memory.
    fn reserve(&mut self, len: usize) -> Option<*mut u8>;
    /// Applies `(offset, length, protection)` ranges inside the reservation at `base`; offsets and lengths are page multiples.
    fn publish(&mut self, base: *mut u8, ranges: &[(usize, usize, Prot)]) -> Result<(), &'static str>;
    /// Returns a reservation.
    fn release(&mut self, base: *mut u8, len: usize);
}

/// The outside world of one guest: its console and the engine's marker log.
pub trait Host {
    /// A marker line (`[OSXX86 ...]`), without a trailing newline.
    fn marker(&mut self, line: &str);
    /// The guest wrote `bytes` to file descriptor `fd` (1 or 2); returns the byte count written, or a negative value.
    fn write(&mut self, fd: i32, bytes: &[u8]) -> isize;
}

/// The trap encodings the admission scan refuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrapKind {
    Syscall,
    Sysenter,
    Int80,
}

impl TrapKind {
    pub fn name(self) -> &'static str {
        match self {
            TrapKind::Syscall => "syscall",
            TrapKind::Sysenter => "sysenter",
            TrapKind::Int80 => "int80",
        }
    }
}

/// Why an image did not run. Each variant has one marker line; nothing of the guest has executed when a load-time refusal is returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The container is malformed or not x86_64.
    Image(MachoError),
    /// The image uses chained fixups (LC_DYLD_CHAINED_FIXUPS), which this rung does not decode.
    UnsupportedFixups,
    /// A non-PIE executable wants its preferred base and the engine cannot map there.
    FixedBase { vmaddr: u64 },
    /// An executable byte range contains a raw trap encoding. `section` is "segment,section" or the segment name for padding.
    RawTrap { section: String, offset: u64, kind: TrapKind },
    /// Imports no table entry serves, in image order.
    Unbound { symbols: Vec<String> },
    /// An import names a library other than libSystem.
    ForeignDylib { path: String },
    /// A rebase or bind record points outside the mapped image.
    BadFixup(&'static str),
    /// The image has no entry point the engine can call.
    NoEntry,
    /// The engine could not get pages.
    OutOfMemory,
    /// Publishing the final protections failed.
    Publish(&'static str),
    /// Another guest is running in this engine process (one guest per process).
    Busy,
    /// The guest called a lazy-binding stub: an import was left for dyld_stub_binder.
    StubBinder,
    /// The Objective-C runtime or a native entry ended the guest by name (an unrecognized selector, an unsupported feature).
    Fatal(String),
}

impl Refusal {
    /// The marker line for this refusal.
    pub fn marker(&self) -> String {
        use alloc::format;
        match self {
            Refusal::Image(e) => format!("[OSXX86 REFUSE IMAGE {}]", e),
            Refusal::UnsupportedFixups => String::from("[OSXX86 REFUSE CHAINED-FIXUPS]"),
            Refusal::FixedBase { vmaddr } => format!("[OSXX86 REFUSE FIXED-BASE vmaddr={:#x}]", vmaddr),
            Refusal::RawTrap { section, offset, kind } => format!("[OSXX86 REFUSE RAW-TRAP {}+{:#x} kind={}]", section, offset, kind.name()),
            Refusal::Unbound { symbols } => format!("[OSXX86 REFUSE UNBOUND-SYMBOL {}]", symbols.join(",")),
            Refusal::ForeignDylib { path } => format!("[OSXX86 REFUSE FOREIGN-DYLIB {}]", path),
            Refusal::BadFixup(why) => format!("[OSXX86 REFUSE BAD-FIXUP {}]", why),
            Refusal::NoEntry => String::from("[OSXX86 REFUSE NO-ENTRY]"),
            Refusal::OutOfMemory => String::from("[OSXX86 REFUSE NO-MEMORY]"),
            Refusal::Publish(why) => format!("[OSXX86 REFUSE PUBLISH {}]", why),
            Refusal::Busy => String::from("[OSXX86 REFUSE BUSY]"),
            Refusal::StubBinder => String::from("[OSXX86 REFUSE STUB-BINDER]"),
            Refusal::Fatal(why) => format!("[OSXX86 RUNTIME-FATAL {}]", why),
        }
    }
}

/// Classifies the number a stray `syscall` carries in rax by the Darwin x86_64 convention (class in bits 24..32).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DarwinClass {
    Mach,
    Bsd,
    MachineDependent,
    /// Class 0 or above 3: not a Darwin x86_64 call at all.
    Invalid,
}

/// The Darwin class of a raw trap number: 1 Mach traps, 2 BSD, 3 machine-dependent.
pub fn darwin_class(rax: u64) -> DarwinClass {
    match (rax >> 24) & 0xff {
        1 => DarwinClass::Mach,
        2 => DarwinClass::Bsd,
        3 => DarwinClass::MachineDependent,
        _ => DarwinClass::Invalid,
    }
}

/// The smallest number that carries a Darwin class. Every CoralOS system call number is below it, so a Darwin trap can never
/// alias a CoralOS call; the kernel answers an unknown number with its refusal value.
pub const DARWIN_CLASS_MIN: u64 = 1 << 24;
