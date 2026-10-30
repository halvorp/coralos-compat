//! The `Invoke<Name>UPP` family (`InvokeModalFilterUPP` and friends): call a Universal Procedure Pointer the PROGRAM supplied, through the ONE callback machine
//! ([`crate::callback`]). The arguments are the callback's, the UPP is the last argument (r3.. then the UPP), and the callback's result becomes the call's result:
//! `Boolean` (low byte of r3), `OSStatus` (r3) or nothing. A UPP is a PowerPC routine descriptor made by `NewXxxUPP` ([`crate::symbols::Native::NewUpp`]) or, as CarbonLib allows,
//! a plain TVector pointer. A descriptor for 68k code only is refused BY NAME (running 68k from a PowerPC callback is not provided here); a NULL UPP is refused (CarbonLib would
//! jump through it). Which types exist is the table below: a type is added when a measured program imports it (today `InvokeModalFilterUPP`, in four programs).
use crate::callback::{Continuation, Frame, Saved};
use crate::machine::MacPpc;
use alloc::format;
use alloc::string::String;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Result_ { None, Boolean, Status }

/// (C name, argument count before the UPP, result)
pub const TYPES: &[(&str, usize, Result_)] = &[
    ("InvokeModalFilterUPP", 3, Result_::Boolean),   // Boolean (DialogPtr, EventRecord*, short* itemHit, UPP)
    ("InvokeUserItemUPP", 2, Result_::None),         // void (DialogPtr, short item, UPP)
    ("InvokeControlActionUPP", 2, Result_::None),    // void (ControlRef, ControlPartCode, UPP)
    ("InvokeEventHandlerUPP", 3, Result_::Status),   // OSStatus (EventHandlerCallRef, EventRef, void *userData, UPP)
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvokeFn(pub usize);
impl InvokeFn {
    pub fn by_name(name: &str) -> Option<InvokeFn> { TYPES.iter().position(|t| t.0 == name).map(InvokeFn) }
    pub fn name(self) -> &'static str { TYPES[self.0].0 }
}

impl<U: GuestUi> MacPpc<U> {
    /// The glue call `f` was reached at `call_pc` with the arguments in r3..: enter the program's UPP, coming back to the caller (LR) with the result.
    pub(crate) fn begin_invoke(&mut self, cpu: &mut Cpu, f: InvokeFn) -> Result<(), String> {
        let (name, n, res) = TYPES[f.0];
        let upp = if n + 3 <= 10 { cpu.gpr[3 + n] } else { return Err(format!("{name}: too many arguments")) };
        if upp == 0 { return Err(format!("{name}: the UPP is NULL")); }
        let args: alloc::vec::Vec<u32> = cpu.gpr[3..3 + n].to_vec();
        let frame = Frame { saved: Saved::of(cpu, cpu.lr & !3), cont: Continuation::Invoke(res) };
        let sp = cpu.gpr[1];
        // a routine descriptor, or (CarbonLib) a plain TVector
        let r = if heirloom_mac68k_engine::mixed_mode::parse(&self.mac.mem, upp).is_ok() { self.enter_upp(cpu, upp, &args, sp) } else { self.enter_tvector(cpu, upp, &args, sp) };
        r.map_err(|e| format!("{name}: {e}"))?;
        self.rt.callbacks.push(frame);
        Ok(())
    }

    pub(crate) fn invoke_returned(&mut self, cpu: &mut Cpu, frame: Frame, res: Result_) {
        let r = cpu.gpr[3];
        frame.saved.restore(cpu);
        cpu.gpr[3] = match res { Result_::Boolean => r & 0xFF, Result_::Status => r, Result_::None => cpu.gpr[3] };
    }
}
