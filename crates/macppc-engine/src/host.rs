//! The contract with the PowerPC CPU lane.
//!
//! The CPU needs two things from this crate and nothing else: *is this pc a glue stub?* and *run the host call*. When
//! an instruction fetch (or a branch target) lands on a glue address, the interpreter must NOT execute guest code
//! there. It builds the register file as a `[u32; 32]` (r0..r31), calls [`PpcHost::glue_call`], then acts on the
//! outcome: [`GlueOutcome::Return`] means "behave as `blr`" (pc := LR; r3 already holds the result, other registers
//! are untouched); [`GlueOutcome::Exit`] ends the guest program; [`GlueOutcome::Refused`] ends it with a named reason.
//! [`PpcHost::glue_symbol_at`] is the cheap test (`None` for every pc that is not a stub) the CPU can use as its
//! per-branch hook; the stub bytes in memory are a `blr` as a belt-and-braces fallback.
use alloc::string::String;

/// What the CPU does after a host call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GlueOutcome {
    /// Continue as if the callee executed `blr`; the result (if any) is in `regs[3]`.
    Return,
    /// The program called ExitToShell (or returned from main): stop, successfully.
    Exit,
    /// The call has nothing to return yet (a `WaitNextEvent` with no event): park the CPU on the call and retry it when
    /// the machine can make progress. `regs` is untouched.
    Idle,
    /// The call cannot be made (an import with no binding, a floating-point signature ...). The reason names the
    /// library/symbol; the guest cannot continue past it. This is the census's NEXT-NEEDED.
    Refused(String),
}

pub trait PpcHost {
    /// Run the host function bound to `symbol`. `regs` is r0..r31 at the moment of the call (arguments in r3..r10).
    fn glue_call(&mut self, symbol: &str, regs: &mut [u32; 32]) -> GlueOutcome;
    /// The symbol whose glue stub is at `pc`, if `pc` is one.
    fn glue_symbol_at(&self, pc: u32) -> Option<String>;
}
