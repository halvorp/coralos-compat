//! Import binding by symbol NAME.
//!
//! A PowerPC application imports `NewWindow` from `InterfaceLib`; the 68k trap table already has a handler for the trap
//! named `NewWindow`, so the name is the bridge: the Universal Interfaces name and the trap name are the same word for
//! the Toolbox and Memory Manager (the trap table is generated from metamage_1's `traps`, interface facts only).
//! Resolution order: a native (CFM Manager calls, `MemError`, ...) wins, then a data symbol (`qd`), then an OS-trap
//! routine with a C prototype ([`mac68k_engine::mixed::OS_SIGS`]), then a Pascal Toolbox trap. A name the engine
//! cannot serve is [`Binding::Unbound`] with the reason; a census prints it as NEXT-NEEDED.
use alloc::format;
use alloc::string::String;
use guestui_abi::GuestUi;
use mac68k_engine::mixed::{os_sig, trap_opcode, OsSig};
use mac68k_engine::traps::describe;
use mac68k_engine::Mac;

/// Host functions that have no 68k trap (the Code Fragment Manager's API, C-level helpers), or whose 68k form cannot be
/// used as it is: `WaitNextEvent` / `GetNextEvent` suspend a 68k guest through its run loop, so a PowerPC call polls
/// instead (the same as `WaitNextEvent` with `sleep = 0`, which real Mac OS allows to return the null event at once).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Native { GetSharedLibrary, FindSymbol, CloseConnection, MemError, BlockZero, Exit, WaitNextEvent, GetNextEvent }

impl Native {
    pub fn by_name(name: &str) -> Option<Native> {
        Some(match name {
            "GetSharedLibrary" => Native::GetSharedLibrary, "FindSymbol" => Native::FindSymbol, "CloseConnection" => Native::CloseConnection,
            "MemError" => Native::MemError, "WaitNextEvent" => Native::WaitNextEvent, "GetNextEvent" => Native::GetNextEvent, "BlockZero" => Native::BlockZero, "ExitToShell" => Native::Exit,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Binding {
    /// A Pascal-convention Toolbox trap, called with r3.. as its declared arguments.
    Pascal(u16),
    /// An OS-trap routine through its C prototype.
    Os(u16, &'static OsSig),
    Native(Native),
    /// The QuickDraw globals record `qd` (a data import).
    Qd,
    /// No binding; the string says why.
    Unbound(String),
}

impl Binding {
    pub fn is_bound(&self) -> bool { !matches!(self, Binding::Unbound(_)) }
    pub fn is_data(&self) -> bool { matches!(self, Binding::Qd) }
    /// A short description for the census.
    pub fn describe(&self) -> String {
        match self {
            Binding::Pascal(op) => format!("Toolbox trap {}", describe(*op)),
            Binding::Os(op, s) => format!("OS trap {} as C routine {}", describe(*op), s.name),
            Binding::Native(n) => format!("native {n:?}"),
            Binding::Qd => "data: QuickDraw globals".into(),
            Binding::Unbound(why) => why.clone(),
        }
    }
}

/// Handlers that need a 68k CPU's registers or run loop: named here so a PowerPC import of them is NEXT-NEEDED rather
/// than a call into a handler that would see a register-less CPU.
const NEEDS_68K_RUN_LOOP: &[(&str, &str)] = &[
    ("LoadSeg", "68k segment loader: PowerPC fragments have no CODE segments"),
    ("UnLoadSeg", "68k segment loader: PowerPC fragments have no CODE segments"),
];

/// How `name` would be served, given the handlers `mac` has registered.
pub fn lookup<U: GuestUi>(mac: &Mac<U>, name: &str) -> Binding {
    if let Some(n) = Native::by_name(name) { return Binding::Native(n); }
    if name == "qd" { return Binding::Qd; }
    if let Some((_, why)) = NEEDS_68K_RUN_LOOP.iter().find(|(n, _)| *n == name) { return Binding::Unbound(format!("{name}: {why}")); }
    let Some(op) = trap_opcode(name) else {
        return Binding::Unbound(format!("{name} is not a Toolbox or OS trap name in the engine's tables"));
    };
    if let Some(sig) = os_sig(name) {
        return if mac.is_os(op) { Binding::Os(op, sig) } else { Binding::Unbound(format!("{} has no handler yet", describe(op))) };
    }
    if mac.is_pascal(op) { return Binding::Pascal(op); }
    if mac.is_os(op) { return Binding::Unbound(format!("{} is an OS trap with no C prototype in OS_SIGS yet", describe(op))); }
    Binding::Unbound(format!("{} has no handler yet", describe(op)))
}
