//! The Mixed Mode seam: the Toolbox handlers called by VALUE, from a processor that is not the 68k.
//!
//! Classic Mac OS ran 68k and PowerPC code side by side and let each call the other's system software through Mixed
//! Mode. Here the "system software" is one set of Rust handlers, and the two calling conventions are thin adapters
//! over the same value-form entry points:
//!
//! * A Toolbox (Pascal) trap's handler takes a [`crate::machine::Call`] whose [`crate::pascal::Args`] are plain
//!   values in declaration order. The 68k adapter fills them from the stack frame (`machine.rs`, `Host::aline`); the
//!   PowerPC adapter fills them from r3..r10 ([`Mac::pascal_call`]). Same handler function, same log, same refusals.
//! * An Operating System trap's handler takes [`Regs`] (D0/A0/A1/D1 values). The 68k adapter reads them from the
//!   CPU; the PowerPC adapter maps the C prototype's arguments onto them with an [`OsSig`] ([`Mac::os_call`]).
//!
//! Handlers that need a 68k CPU's registers or its run loop (`LoadSeg`, the blocking event calls) are 68k-only; they
//! see a detached [`Cpu`] with no registers, so a caller must keep PowerPC code away from them (the macppc engine
//! lists them by name and refuses them as NEXT-NEEDED).
use crate::machine::{Call, Conv, Mac};
use crate::osconv::Regs;
use crate::pascal::{Arg, Args};
use crate::trap_names::TRAPS;
use crate::traps::{trap_key, trap_name};
use alloc::string::String;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

/// What came back from a by-value call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Return {
    /// The function result: zero-extended for Boolean/Byte and INTEGER (the caller sign-extends), as-is for pointers.
    pub value: u32,
    /// The handler asked to end the program (`ExitToShell`).
    pub halted: bool,
    /// The handler tried to suspend the caller for an event: not possible without the caller's run loop.
    pub blocked: bool,
}

/// Why a by-value call could not be made at all (a handler's own refusal is NOT this: it is logged in
/// `Mac::refused`, exactly as for the 68k, and the call returns 0).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MixedError {
    /// No trap of that name in the name table.
    UnknownName(String),
    /// The trap is named but this engine has no handler for it yet (the census's NEXT-NEEDED).
    Unimplemented(String),
    /// A Pascal entry was called as an OS trap or the other way round.
    WrongConvention(String),
    /// The wrong number of arguments for the signature.
    ArgCount { name: String, want: usize, got: usize },
    /// The handler needs 68k registers (a stack pointer, the PC) that a PowerPC caller does not have: the call is refused
    /// and any wait it started is cancelled.
    NeedsCpuRegisters(String),
}

impl core::fmt::Display for MixedError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            MixedError::UnknownName(n) => write!(f, "{n} is not a trap name"),
            MixedError::Unimplemented(n) => write!(f, "unimplemented trap _{n}"),
            MixedError::WrongConvention(n) => write!(f, "{n} is registered with the other calling convention"),
            MixedError::ArgCount { name, want, got } => write!(f, "{name} takes {want} arguments, {got} given"),
            MixedError::NeedsCpuRegisters(n) => write!(f, "{n} needs 68k registers (it blocks or reads the stack), which a PowerPC caller cannot give it"),
        }
    }
}

/// Where a PowerPC argument goes in an OS trap's register file, and which register carries the result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reg { D0, D1, A0, A1 }

/// The C prototype of an OS-trap routine: argument `i` is passed in `args[i]`; the result comes back in `result`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OsSig { pub name: &'static str, pub args: &'static [Reg], pub result: Option<Reg> }

use Reg::{A0, A1, D0};
/// The Memory Manager routines as C prototypes (Universal Interfaces `MacMemory.h`), mapped onto the register
/// contracts their traps already implement. The `Sys`/`Clear` variants differ from the base trap only in flag bits
/// that the handler reads from the trap word.
pub const OS_SIGS: &[OsSig] = &[
    OsSig { name: "NewHandle", args: &[D0], result: Some(A0) },
    OsSig { name: "NewHandleClear", args: &[D0], result: Some(A0) },
    OsSig { name: "NewHandleSys", args: &[D0], result: Some(A0) },
    OsSig { name: "NewHandleSysClear", args: &[D0], result: Some(A0) },
    OsSig { name: "NewPtr", args: &[D0], result: Some(A0) },
    OsSig { name: "NewPtrClear", args: &[D0], result: Some(A0) },
    OsSig { name: "NewPtrSys", args: &[D0], result: Some(A0) },
    OsSig { name: "NewPtrSysClear", args: &[D0], result: Some(A0) },
    OsSig { name: "DisposeHandle", args: &[A0], result: None },
    OsSig { name: "DisposePtr", args: &[A0], result: None },
    OsSig { name: "GetHandleSize", args: &[A0], result: Some(D0) },
    OsSig { name: "SetHandleSize", args: &[A0, D0], result: None },
    OsSig { name: "GetPtrSize", args: &[A0], result: Some(D0) },
    OsSig { name: "HLock", args: &[A0], result: None },
    OsSig { name: "HUnlock", args: &[A0], result: None },
    OsSig { name: "HGetState", args: &[A0], result: Some(D0) },
    OsSig { name: "HSetState", args: &[A0, D0], result: None },
    OsSig { name: "HPurge", args: &[A0], result: None },
    OsSig { name: "HNoPurge", args: &[A0], result: None },
    OsSig { name: "BlockMove", args: &[A0, A1, D0], result: None },
    OsSig { name: "BlockMoveData", args: &[A0, A1, D0], result: None },
    OsSig { name: "MoreMasters", args: &[], result: None },
    OsSig { name: "FreeMem", args: &[], result: Some(D0) },
    OsSig { name: "CompactMem", args: &[D0], result: Some(D0) },
    OsSig { name: "PurgeMem", args: &[D0], result: None },
    OsSig { name: "StripAddress", args: &[D0], result: Some(D0) },
    OsSig { name: "RecoverHandle", args: &[A0], result: Some(A0) },
    OsSig { name: "MaxApplZone", args: &[], result: None },
];

/// The trap word named `name` (`"NewWindow"` -> `0xA913`); aliases carry their flag bits, so `NewHandleClear` keeps its
/// clear bit and the handler reads it from the word.
pub fn trap_opcode(name: &str) -> Option<u16> { TRAPS.iter().find(|&&(_, n)| n == name).map(|&(w, _)| w) }

/// The signature of the OS-trap routine `name`, when it has a C-callable form.
pub fn os_sig(name: &str) -> Option<&'static OsSig> { OS_SIGS.iter().find(|s| s.name == name) }

impl<U: GuestUi> Mac<U> {
    /// True if the table has a handler for `opcode` and it uses the Pascal convention.
    pub fn is_pascal(&self, opcode: u16) -> bool { matches!(self.table.get(&trap_key(opcode)), Some(e) if matches!(e.conv, Conv::Pascal { .. })) }
    /// The argument layout and result kind of a Pascal trap.
    pub fn pascal_signature(&self, opcode: u16) -> Option<(&'static [Arg], Option<Arg>)> {
        match self.table.get(&trap_key(opcode))?.conv { Conv::Pascal { args, result, .. } => Some((args, result)), Conv::Os(_) => None }
    }
    /// True if the table has an OS (register-convention) handler for `opcode`.
    pub fn is_os(&self, opcode: u16) -> bool { matches!(self.table.get(&trap_key(opcode)), Some(e) if matches!(e.conv, Conv::Os(_))) }

    /// Call a Pascal-convention Toolbox trap with its arguments as values (declaration order). Runs the SAME handler the
    /// 68k A-line bridge runs.
    pub fn pascal_call(&mut self, opcode: u16, values: &[u32]) -> Result<Return, MixedError> {
        let name = trap_name(opcode).unwrap_or("?");
        let entry = self.table.get(&trap_key(opcode)).copied().ok_or_else(|| MixedError::Unimplemented(name.into()))?;
        let Conv::Pascal { args: layout, result: rkind, handler } = entry.conv else { return Err(MixedError::WrongConvention(name.into())) };
        if layout.len() != values.len() { return Err(MixedError::ArgCount { name: name.into(), want: layout.len(), got: values.len() }); }
        let mut cpu = Cpu::detached();
        let outcome = Args::from_values(layout, values).and_then(|args| handler(self, &mut cpu, &Call { opcode, pc: 0, name, args }));
        if cpu.touched_registers() { self.ev.waiting = None; return Err(MixedError::NeedsCpuRegisters(name.into())); }
        let raw = self.finish_pascal(name, outcome);
        let value = match rkind { Some(Arg::B) => raw & 0xFF, Some(Arg::W) => raw & 0xFFFF, _ => raw };
        Ok(Return { value, halted: cpu.is_halted(), blocked: cpu.is_blocked() })
    }

    /// Call an OS-convention trap by its C prototype ([`OsSig`]). Runs the SAME handler the 68k A-line bridge runs.
    pub fn os_call(&mut self, opcode: u16, sig: &OsSig, values: &[u32]) -> Result<Return, MixedError> {
        let name = trap_name(opcode).unwrap_or(sig.name);
        if sig.args.len() != values.len() { return Err(MixedError::ArgCount { name: name.into(), want: sig.args.len(), got: values.len() }); }
        let mut regs = Regs { opcode, d0: 0, a0: 0, d1: 0, a1: 0 };
        for (slot, &v) in sig.args.iter().zip(values) {
            match slot { Reg::D0 => regs.d0 = v, Reg::D1 => regs.d1 = v, Reg::A0 => regs.a0 = v, Reg::A1 => regs.a1 = v }
        }
        let (regs, mut ret) = self.os_call_regs(regs)?;
        ret.value = match sig.result { Some(Reg::D0) => regs.d0, Some(Reg::A0) => regs.a0, Some(Reg::A1) => regs.a1, Some(Reg::D1) => regs.d1, None => 0 };
        Ok(ret)
    }

    /// Call an OS-convention trap with its register file given whole, and get the register file back: for routines whose
    /// C prototype returns more than one value (`PurgeSpace`: D0 and A0) or packs two arguments into one register
    /// (`FlushEvents`: D0 = stopMask << 16 | eventMask). `Return::value` is D0. Runs the SAME handler the 68k A-line bridge runs.
    pub fn os_call_regs(&mut self, mut regs: Regs) -> Result<(Regs, Return), MixedError> {
        let opcode = regs.opcode;
        let name = trap_name(opcode).unwrap_or("?");
        let entry = self.table.get(&trap_key(opcode)).copied().ok_or_else(|| MixedError::Unimplemented(name.into()))?;
        let Conv::Os(handler) = entry.conv else { return Err(MixedError::WrongConvention(name.into())) };
        let mut cpu = Cpu::detached();
        let outcome = handler(self, &mut cpu, &mut regs);
        if cpu.touched_registers() { self.ev.waiting = None; return Err(MixedError::NeedsCpuRegisters(name.into())); }
        self.finish_os(name, outcome, &mut regs);
        Ok((regs, Return { value: regs.d0, halted: cpu.is_halted(), blocked: cpu.is_blocked() }))
    }

    /// `MemError()`: the last Memory Manager result (a read of the low-memory word, not a trap on any CPU).
    pub fn mem_error(&self) -> i16 { self.mem.read_u16(crate::lowmem::MEM_ERR).unwrap_or(0) as i16 }
}
