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

/// `SysBeep` is here because the 68k engine has not registered it (its tests use it as their example of an unimplemented
/// trap); it moves to a shared handler when the 68k side gets one.
/// Host functions that have no 68k trap (the Code Fragment Manager's API, C-level helpers), or whose 68k form cannot be
/// used as it is: the 68k event calls suspend a 68k guest through its run loop, so the PowerPC adapter ([`crate::natives`]) polls
/// the handler with `sleep = 0` and parks the CPU itself (`WaitNextEvent` with a sleep); `GetNextEvent` never waits; `ModalDialog` is parked the same way, on the front dialog's item hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Native { GetSharedLibrary, FindSymbol, CloseConnection, MemError, BlockZero, Exit, WaitNextEvent, GetNextEvent, ModalDialog, SysBeep,
    /// `HasDepth` / `SetDepth`: selectors of the 68k `PaletteDispatch` trap, so they have no trap name of their own; the shared routines in `cqd::depth`.
    HasDepth, SetDepth,
    /// StdCLib `__setjmp`, `exit`, `_BreakPoint`: they need the CPU's registers, so [`crate::run`] serves them, not `glue_call`.
    SetJmp, CExit, BreakPoint,
    /// The return address of a fragment's init routine: a pseudo-call the run loop answers by starting `main`.
    InitDone,
    /// "Where is trap N?": `NGetTrapAddress(trapNum, TrapType)`, `GetOSTrapAddress(trapNum)`, `GetToolTrapAddress(trapNum)`,
    /// `GetTrapAddress(trapNum)`. C routines in InterfaceLib with no trap word of their own spelling; the answer is the shared
    /// trap registry's (`mac68k_engine::traptable::trap_address`), so a PowerPC program's availability check agrees with a
    /// 68k program's. The value is a guest 68k address: calling it is Mixed Mode's business, not a branch.
    TrapAddress(TrapQuery),
    /// Mixed Mode's constructors and destructor (`mac68k_engine::mixed_mode`): a real RoutineDescriptor in guest memory, never
    /// the procedure's own TVector. `NewRoutineDescriptor(proc, procInfo, ISA)`, `NewFatRoutineDescriptor(m68kProc, ppcProc,
    /// procInfo)`, `DisposeRoutineDescriptor(upp)`. CALLING a UPP is the callback machine's (Mixed Mode slice 3).
    NewRoutineDescriptor, NewFatRoutineDescriptor, DisposeRoutineDescriptor,
    /// The address a PowerPC callback returns to (`crate::callback`): a pseudo-call the run loop answers from the frame stack.
    CallbackReturn,
    /// A `Files.h` routine (`FSRead`, `HOpen`, `GetVol` ...): the C glue over the 68k engine's File Manager traps ([`crate::files`]).
    File(crate::files::FileCall) }

/// Which trap table a [`Native::TrapAddress`] import names, as the calling 68k trap word's type bits would.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrapQuery {
    /// `NGetTrapAddress(trapNum, tTyp)`: `tTyp` = `OSTrap` (0) or `ToolTrap` (1), the second argument.
    ByType,
    /// `GetOSTrapAddress`: `_GetOSTrapAddress` ($A346).
    Os,
    /// `GetToolTrapAddress`: `_GetToolTrapAddress` ($A746).
    Tool,
    /// `GetTrapAddress`: the old `_GetTrapAddress` ($A146), the table inferred from the number.
    Old,
}

impl TrapQuery {
    /// The calling trap word whose type bits pick the table (`tool_arg` = `NGetTrapAddress`'s `tTyp`).
    pub fn opcode(self, tool_arg: u32) -> u16 {
        match self {
            TrapQuery::ByType => if tool_arg & 0xFF == 1 { 0xA746 } else { 0xA346 },
            TrapQuery::Os => 0xA346,
            TrapQuery::Tool => 0xA746,
            TrapQuery::Old => 0xA146,
        }
    }
}

impl Native {
    pub fn by_name(name: &str) -> Option<Native> {
        Some(match name {
            "GetSharedLibrary" => Native::GetSharedLibrary, "FindSymbol" => Native::FindSymbol, "CloseConnection" => Native::CloseConnection,
            "MemError" => Native::MemError, "WaitNextEvent" => Native::WaitNextEvent, "GetNextEvent" => Native::GetNextEvent, "ModalDialog" => Native::ModalDialog, "BlockZero" => Native::BlockZero, "SysBeep" => Native::SysBeep, "ExitToShell" => Native::Exit, "HasDepth" => Native::HasDepth, "SetDepth" => Native::SetDepth,
            "NGetTrapAddress" => Native::TrapAddress(TrapQuery::ByType), "GetOSTrapAddress" => Native::TrapAddress(TrapQuery::Os),
            "GetToolTrapAddress" => Native::TrapAddress(TrapQuery::Tool), "GetTrapAddress" => Native::TrapAddress(TrapQuery::Old),
            "NewRoutineDescriptor" => Native::NewRoutineDescriptor, "NewFatRoutineDescriptor" => Native::NewFatRoutineDescriptor,
            "DisposeRoutineDescriptor" => Native::DisposeRoutineDescriptor,
            _ => return crate::files::FileCall::by_name(name).map(Native::File),
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
    /// A zero-initialised variable of this many bytes in guest memory (StdCLib's runtime state).
    CData(u32),
    /// No binding; the string says why.
    Unbound(String),
}

impl Binding {
    pub fn is_bound(&self) -> bool { !matches!(self, Binding::Unbound(_)) }
    pub fn is_data(&self) -> bool { matches!(self, Binding::Qd | Binding::CData(_)) }
    /// A short description for the census.
    pub fn describe(&self) -> String {
        match self {
            Binding::Pascal(op) => format!("Toolbox trap {}", describe(*op)),
            Binding::Os(op, s) => format!("OS trap {} as C routine {}", describe(*op), s.name),
            Binding::Native(n) => format!("native {n:?}"),
            Binding::Qd => "data: QuickDraw globals".into(),
            Binding::CData(n) => format!("data: {n} zeroed bytes of C runtime state"),
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

/// InterfaceLib exports whose C name differs from the name the 68k trap table gives the same trap (the table keeps the
/// trap-era spelling). Found by importing the real Systemless toolbox-showcase PEF and matching the unnamed imports to
/// trap words case-insensitively; `GetMenu` is the Menu Manager's `_GetRMenu` (menu.rs registers it under that word); `SetPalette` is `_NSetPalette`'s word ($AA95, which the 68k table names after the newer INTEGER form; the older Boolean `cUpdates` fits the same slot); `SetPortBits` is `_SetPBits`. The three `Invert*` C names whose traps keep the short `_Inver*` spelling;
/// InvertOval/InvertArc/InvertPoly already match.
pub const INTERFACELIB_ALIASES: &[(&str, &str)] = &[("GetMenu", "GetRMenu"), ("BeginUpdate", "BeginUpDate"), ("EndUpdate", "EndUpDate"), ("XorRgn", "XOrRgn"), ("SetPalette", "NSetPalette"), ("SetPortBits", "SetPBits"),
    ("InvertRect", "InverRect"), ("InvertRoundRect", "InverRoundRect"), ("InvertRgn", "InverRgn")];

/// How `name` would be served, given the handlers `mac` has registered.
pub fn lookup<U: GuestUi>(mac: &Mac<U>, name: &str) -> Binding {
    let name = INTERFACELIB_ALIASES.iter().find(|(from, _)| *from == name).map_or(name, |&(_, trap)| trap);
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
