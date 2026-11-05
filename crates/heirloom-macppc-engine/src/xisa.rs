//! Cross-ISA calls: PowerPC code calling 68k code through Mixed Mode (`CallUniversalProc` of a 68k routine descriptor, or of a plain
//! 68k ProcPtr: [`heirloom_mac68k_engine::mixed_mode::resolve`]). The 68k routine runs on the machine's 68k CPU (heirloom-mac68k-engine
//! `cpu`: the static core on the host, cpu-68k.library in macppc.engine) over the SAME guest memory and the SAME Toolbox, through the
//! ONE callback machine ([`crate::callback`]): the PowerPC call is saved in a [`Frame`] whose continuation is [`Continuation::M68k`],
//! the 68k frame is built from the procInfo (heirloom-mac68k-engine `procinfo`: Pascal or C, stack-based) with its return address at
//! `CROSS_ISA_RETURN`, and the run loop steps the 68k CPU instead of the PowerPC one until the routine returns there. Then the result
//! (the reserved space, or D0) goes to r3 and every PowerPC register of the interrupted call is put back. No handler re-enters a CPU.
//!
//! The routine runs with A5 = `CurrentA5` (the application's A5 world, as the 68k emulator of a PowerPC Mac enters 68k code), on a stack
//! carved below the caller's PowerPC frame (past the red zone). Named refusals, before anything changes: no 68k CPU in this build,
//! a convention not built (register-based, dispatched), the wrong argument count. Named faults while it runs: it waits for an event
//! (a 68k routine called from PowerPC parks nothing here), it calls an unimplemented trap (the call stops on it), a bus fault, a stop,
//! a Pascal routine that returns with its parameters not popped (an unbalanced stack is a wrong convention, never ignored). A 68k routine
//! that itself calls a routine descriptor (`_MixedModeMagic` back into PowerPC) is such an unimplemented trap: nesting PowerPC inside it
//! is not built (trigger: the first program that does).
use crate::callback::{Continuation, Frame, Saved};
use crate::cpu::{Cpu, RunStats};
use crate::machine::MacPpc;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_common::Step;
use heirloom_mac68k_engine::guestcall::CROSS_ISA_RETURN;
use heirloom_mac68k_engine::procinfo::{push_frame, read_result, stack_call, Frame68k, StackCall, StackConv};
use heirloom_ui_abi::GuestUi;

/// Bytes below the interrupted call's r1 the 68k stack starts at: past the PowerPC red zone (224 bytes) and the callback frames'
/// gap, so neither the caller's saved words nor a PowerPC callback frame are under it.
const M68K_STACK_GAP: u32 = 1024;

/// A 68k routine in progress: its decoded convention and frame (the continuation's state).
#[derive(Debug, Clone)]
pub(crate) struct M68kCall { pub(crate) code: u32, call: StackCall, frame: Frame68k, stop_on_unimplemented: bool }

/// The 68k half of the run loop's state: the 68k CPU's context while a routine runs.
#[derive(Default)]
pub(crate) struct Xisa { pub(crate) ctx: Option<heirloom_mac68k_engine::cpu::CpuContext> }

impl core::fmt::Debug for Xisa {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { write!(f, "Xisa {{ running: {} }}", self.ctx.is_some()) }
}

/// The `i`-th argument word of a glue call (r3 is word 0): r3..r10, then the caller's parameter area at 24(r1) (the glue makes no frame).
pub(crate) fn arg_word(m: &heirloom_mem_flat32::GuestMem, cpu: &Cpu, i: usize) -> Result<u32, String> {
    if i < 8 { return Ok(cpu.gpr[3 + i]); }
    let at = cpu.gpr[1].wrapping_add(24 + 4 * i as u32);
    m.read_u32(at).map_err(|e| format!("argument {i} at {at:#x} (the caller's parameter area): {e}"))
}

impl<U: GuestUi> MacPpc<U> {
    /// Enter the 68k routine at `code` with `args`, as `proc_info` describes it; the PowerPC caller resumes at its LR with the result in r3.
    pub(crate) fn enter_68k(&mut self, cpu: &mut Cpu, code: u32, proc_info: u32, args: &[u32]) -> Result<(), String> {
        if !heirloom_mac68k_engine::cpu::available() {
            return Err(String::from("no 68k CPU in this build (cpu-68k.library is not bound: the engine opens it beside cpu-ppc.library)"));
        }
        if self.rt.xisa.ctx.is_some() { return Err(String::from("a 68k routine is already running (nesting 68k calls through PowerPC is not built)")); }
        let call = stack_call(proc_info)?;
        if args.len() < call.params.len() { return Err(format!("procInfo {proc_info:#x} has {} parameters, {} were passed", call.params.len(), args.len())); }
        let args = &args[..call.params.len()];
        let top = cpu.gpr[1].wrapping_sub(M68K_STACK_GAP) & !0xF;
        let frame = push_frame(&mut self.mac.mem, top, &call, args, CROSS_ISA_RETURN).map_err(|e| format!("the 68k frame below {top:#x}: {e}"))?;
        let a5 = self.mac.mem.read_u32(heirloom_mac68k_engine::lowmem::CURRENT_A5).unwrap_or(0);
        let model = self.mac.profile.cpu;
        // the 68k reset reads its SP and PC from the vectors at 0 and 4 (and leaves 0 written): low memory is the program's, so both go back
        let low = (self.mac.mem.read_u32(0).unwrap_or(0), self.mac.mem.read_u32(4).unwrap_or(0));
        let ctx = heirloom_mac68k_engine::cpu::start_model(&mut self.mac, code, frame.sp, &[(heirloom_mac68k_engine::cpu::REG_A0 + 5, a5)], model);
        let _ = self.mac.mem.write_u32(0, low.0);
        let _ = self.mac.mem.write_u32(4, low.1);
        let stop_on_unimplemented = core::mem::replace(&mut self.mac.stop_on_unimplemented, true);
        self.mac.cross_isa_running = true;
        self.mac.cross_isa_returned = None;
        self.mac.log.push(format!("[mixedmode] 68k routine at {code:#x} (procInfo {proc_info:#x}, {:?}, {} parameter(s)) runs on the 68k CPU, A5={a5:#x}", call.conv, call.params.len()));
        self.rt.callbacks.push(Frame { saved: Saved::of(cpu, cpu.lr & !3), cont: Continuation::M68k(M68kCall { code, call, frame, stop_on_unimplemented }) });
        self.rt.xisa.ctx = Some(ctx);
        cpu.block(); // the PowerPC CPU stops here; the run loop steps the 68k one ([`MacPpc::step_68k`])
        Ok(())
    }

    /// One run-loop step while a 68k routine runs: the 68k CPU gets the budget; its return finishes the PowerPC call (`cpu` is the
    /// PowerPC CPU whose call is waiting; public for tests that drive a CPU by hand).
    pub fn step_68k(&mut self, cpu: &mut Cpu, budget: u64) -> (Step, RunStats) {
        let Some(mut ctx) = self.rt.xisa.ctx.take() else { return (Step::BudgetSpent, RunStats { insns: 0, halted: false, blocked: false, trap: None }) };
        let s = heirloom_mac68k_engine::cpu::resume(&mut self.mac, &mut ctx, budget);
        heirloom_mac68k_engine::qd::flush(&mut self.mac);
        let stats = RunStats { insns: s.insns, halted: false, blocked: false, trap: None };
        let code = self.rt.callbacks.last().and_then(|f| if let Continuation::M68k(c) = &f.cont { Some(c.code) } else { None }).unwrap_or(0);
        let outcome = if let Some((d0, a7)) = self.mac.cross_isa_returned.take() {
            self.m68k_returned(cpu, d0, a7)
        } else if s.bus_faults > 0 {
            Err(format!("[HEIRLOOM-MACPPC XISA-68K-FAULT code={code:#x}] the 68k routine called from PowerPC took {} bus fault(s)", s.bus_faults))
        } else if s.halted {
            let last = self.mac.refused.last().cloned().unwrap_or_default();
            Err(format!("[HEIRLOOM-MACPPC XISA-68K-STOPPED code={code:#x}] the 68k routine called from PowerPC stopped before it returned (last refusal: {last})"))
        } else if s.blocked {
            Err(format!("[HEIRLOOM-MACPPC XISA-68K-BLOCKED code={code:#x}] the 68k routine called from PowerPC waited for an event: a 68k routine's wait is not parked here"))
        } else if let Some(pc) = s.stalled_at {
            Err(format!("[HEIRLOOM-MACPPC XISA-68K-STOPPED code={code:#x}] the 68k CPU stopped making progress at pc={pc:#x}"))
        } else {
            self.rt.xisa.ctx = Some(ctx); // budget spent: it goes on next step
            Ok(())
        };
        if let Err(why) = outcome {
            self.end_68k();
            self.mac.refused.push(why.clone());
            self.mac.log.push(why.clone());
            self.rt.fault = Some(why.clone());
            return (Step::Fault(why), stats);
        }
        (Step::BudgetSpent, stats)
    }

    fn end_68k(&mut self) {
        self.mac.cross_isa_running = false;
        self.rt.xisa.ctx = None;
    }

    /// The 68k routine returned to `CROSS_ISA_RETURN` with these D0 and A7: check the stack it left, put the PowerPC call back, r3 = the result.
    fn m68k_returned(&mut self, cpu: &mut Cpu, d0: u32, a7: u32) -> Result<(), String> {
        self.end_68k();
        let Some(frame) = self.rt.callbacks.pop() else { return Err(String::from("a 68k routine returned with no callback in progress")) };
        let Continuation::M68k(c) = frame.cont else { return Err(format!("a 68k routine returned while the innermost callback is {:?}", frame.cont)) };
        self.mac.stop_on_unimplemented = c.stop_on_unimplemented;
        // RTS popped the return address: A7 must be what the convention leaves (Pascal: the callee popped its parameters)
        if a7 != c.frame.sp_after {
            return Err(format!("[HEIRLOOM-MACPPC XISA-68K-STACK code={:#x}] the {:?} routine returned with A7={a7:#x}, not {:#x}: its procInfo does not describe it", c.code, c.call.conv, c.frame.sp_after));
        }
        let r = read_result(&self.mac.mem, &c.call, &c.frame, d0).map_err(|e| format!("the 68k routine's result: {e}"))?;
        frame.saved.restore(cpu);
        cpu.gpr[3] = r;
        self.mac.log.push(format!("[mixedmode] 68k routine at {:#x} returned {r:#x} ({})", c.code, if c.call.conv == StackConv::Pascal { "Pascal result space" } else { "D0" }));
        Ok(())
    }

    /// Is a 68k routine running (for the run loop and tests)?
    pub fn m68k_running(&self) -> bool { self.rt.xisa.ctx.is_some() }
}

/// The parameters `CallUniversalProc(upp, procInfo, ...)` passes on: argument words 2.. of the glue call, as many as `proc_info`'s
/// stack-based parameter list names (at most 13).
pub(crate) fn universal_args(m: &heirloom_mem_flat32::GuestMem, cpu: &Cpu, n: usize) -> Result<Vec<u32>, String> {
    (0..n).map(|k| arg_word(m, cpu, 2 + k)).collect()
}
