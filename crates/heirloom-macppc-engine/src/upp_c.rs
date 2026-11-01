//! `CallUniversalProc(upp, procInfo, ...)` (Mixed Mode, InterfaceLib; Inside Macintosh: PowerPC System Software 2-31): call the routine
//! a UPP names with the arguments `procInfo` describes. Served by the run loop through the ONE callback machine ([`crate::callback`]):
//! * a PowerPC routine descriptor, or a plain TVector -> the program's own code runs; its r3 comes back as the result;
//! * a 68k TRAP's stub (the address `GetTrapAddress` gave, heirloom-mac68k-engine `traptable::in_stub`) whose trap the engine serves BY
//!   VALUE -> the same body the 68k trap runs (today `_CursorDeviceDispatch` $AADB, D0-dispatched Pascal: Warcraft III's CursorDeviceLib);
//! * anything else (other 68k code, the `_Unimplemented` stub, NIL) -> refused BY NAME: running arbitrary 68k code from a PowerPC call
//!   is not provided here, and a real Mac calling `_Unimplemented` stops with a system error.
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
            let why = format!("[HEIRLOOM-MACPPC CallUniversalProc REFUSED trap={word:#06x} procInfo={info:#x}] the 68k trap behind {upp:#x} is not served by value from PowerPC");
            self.mac.refused.push(why.clone());
            return Err(why);
        }
        // the program's own PowerPC code: a routine descriptor, or (CarbonLib) a plain TVector
        let n = param_count(info, if matches!(conv, 8 | 9 | 12) { 8 } else { 6 }).min(8);
        let args: Vec<u32> = cpu.gpr[5..5 + n].to_vec();
        let res = match (info >> 4) & 3 { 0 => Result_::None, 1 => Result_::Boolean, _ => Result_::Status };
        let frame = Frame { saved: Saved::of(cpu, cpu.lr & !3), cont: Continuation::Invoke(res) };
        let sp = cpu.gpr[1];
        let r = if heirloom_mac68k_engine::mixed_mode::parse(&self.mac.mem, upp).is_ok() { self.enter_upp(cpu, upp, &args, sp) } else { self.enter_tvector(cpu, upp, &args, sp) };
        r.map_err(|e| { let why = format!("[HEIRLOOM-MACPPC CallUniversalProc REFUSED upp={upp:#x}] {e}"); self.mac.refused.push(why.clone()); why })?;
        self.rt.callbacks.push(frame);
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
