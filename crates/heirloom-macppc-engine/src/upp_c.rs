//! `CallUniversalProc(upp, procInfo, ...)` (Mixed Mode, InterfaceLib; Inside Macintosh: PowerPC System Software 2-31): call the routine
//! a UPP names with the arguments `procInfo` describes. Served by the run loop through the ONE callback machine ([`crate::callback`]).
//! Which routine, by Mixed Mode's rule ([`heirloom_mac68k_engine::mixed_mode::resolve`]: a routine descriptor names its records' ISAs;
//! any other pointer is a 68k ProcPtr):
//! * a PowerPC record -> the program's own PowerPC code runs; its r3 comes back as the result;
//! * a 68k TRAP's stub (the address `GetTrapAddress` gave, heirloom-mac68k-engine `traptable::in_stub`) whose trap the engine serves BY
//!   VALUE -> the same body the 68k trap runs (today `_CursorDeviceDispatch` $AADB, D0-dispatched Pascal: Warcraft III's CursorDeviceLib);
//!   the `_Unimplemented` stub -> refused by name (a real Mac calling it stops with a system error);
//! * any other 68k code (a 68k record, a plain ProcPtr, another trap's stub) -> runs on the 68k CPU ([`crate::xisa`]), marshalled by
//!   `procInfo` (stack-based Pascal or C; other conventions are named refusals);
//! * NIL -> refused by name.
//! `procInfo`: bits 0-3 the calling convention, 4-5 the result size; for a D0-dispatched convention bits 6-7 the selector size and
//! the parameters from bit 8, two bits each (Universal Interfaces MixedMode.h).
use crate::callback::{Continuation, Frame, Saved};
use crate::cpu::Cpu;
use crate::invoke_c::Result_;
use crate::machine::MacPpc;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_ui_abi::GuestUi;

/// kD0DispatchedPascalStackBased.
const D0_DISPATCHED_PASCAL: u32 = 8;
/// `_Unimplemented` (the stub every unimplemented trap's address is).
const UNIMPLEMENTED_TRAP: u16 = 0xA89F;
/// `_CursorDeviceDispatch`.
const CURSOR_DEVICE_DISPATCH: u16 = 0xAADB;

/// The parameter sizes `procInfo` gives from bit `from` (0 = none from there on): how many parameters.
fn param_count(proc_info: u32, from: u32) -> usize {
    (0..13).take_while(|k| from + 2 * k < 32 && (proc_info >> (from + 2 * k)) & 3 != 0).count()
}

impl<U: GuestUi> MacPpc<U> {
    /// The glue call was reached with r3 = upp, r4 = procInfo, r5.. the routine's arguments.
    pub(crate) fn call_universal_proc(&mut self, cpu: &mut Cpu) -> Result<(), String> {
        let (upp, info) = (cpu.gpr[3], cpu.gpr[4]);
        if upp == 0 { return Err(String::from("[HEIRLOOM-MACPPC CallUniversalProc REFUSED upp=NIL] the program called a NIL procedure")); }
        let conv = info & 0xF;
        if heirloom_mac68k_engine::traptable::in_stub(upp) {
            let word = self.mac.mem.read_u16(upp).map_err(|e| format!("CallUniversalProc: the trap stub at {upp:#x}: {e}"))?;
            if word == CURSOR_DEVICE_DISPATCH && conv == D0_DISPATCHED_PASCAL {
                let n = param_count(info, 8);
                let args: Vec<u32> = cpu.gpr[6..6 + n.min(5)].to_vec();
                let e = self.mac.cursor_device_call(cpu.gpr[5], &args).map_err(|r| format!("CallUniversalProc: {}", r.0))?;
                cpu.gpr[3] = e as i32 as u32;
                return Ok(());
            }
            if word == UNIMPLEMENTED_TRAP {
                let why = format!("[HEIRLOOM-MACPPC CallUniversalProc REFUSED trap={word:#06x} procInfo={info:#x}] the stub behind {upp:#x} is _Unimplemented (a real Mac stops with a system error)");
                self.mac.refused.push(why.clone());
                return Err(why);
            }
            // any other trap's stub is 68k code: it runs on the 68k CPU below, its trap served by the Toolbox like a 68k program's
        }
        let refuse = |m: &mut Self, e: String| { let why = format!("[HEIRLOOM-MACPPC CallUniversalProc REFUSED upp={upp:#x}] {e}"); m.mac.refused.push(why.clone()); why };
        let routine = heirloom_mac68k_engine::mixed_mode::resolve(&self.mac.mem, upp).map_err(|e| refuse(self, format!("{e}")))?;
        let n = param_count(info, if matches!(conv, 8 | 9 | 12) { 8 } else { 6 });
        let args = crate::xisa::universal_args(&self.mac.mem, cpu, n).map_err(|e| refuse(self, e))?;
        match routine {
            heirloom_mac68k_engine::mixed_mode::Routine::PowerPc(record) => {
                if n > 8 { return Err(refuse(self, format!("a PowerPC routine with {n} parameters (more than r3..r10) is not entered yet"))); }
                let res = match (info >> 4) & 3 { 0 => Result_::None, 1 => Result_::Boolean, _ => Result_::Status };
                let frame = Frame { saved: Saved::of(cpu, cpu.lr & !3), cont: Continuation::Invoke(res) };
                let sp = cpu.gpr[1];
                self.enter_tvector(cpu, record.proc_descriptor, &args, sp).map_err(|e| refuse(self, e))?;
                self.rt.callbacks.push(frame);
            }
            // the caller's procInfo describes the call (Mixed Mode marshals by it, not by the record's)
            heirloom_mac68k_engine::mixed_mode::Routine::M68k { code, .. } => {
                self.enter_68k(cpu, code, info, &args).map_err(|e| refuse(self, format!("68k code at {code:#x}: {e}")))?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::param_count;

    #[test]
    fn procinfo_parameter_fields_are_counted_until_the_first_empty_one() {
        // Warcraft III's CursorDeviceNextDevice: kD0DispatchedPascalStackBased, OSErr, a 4-byte selector, one 4-byte parameter
        assert_eq!(param_count(0x3E8, 8), 1);
        assert_eq!(param_count(0x0, 6), 0);
        assert_eq!(param_count(0xFFC1, 6), 5);
    }
}
