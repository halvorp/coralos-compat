//! The slice of `StdCLib` (MPW's C library, a shared library every MPW/CodeWarrior "MPW-style" PowerPC application links)
//! that application startup code names: the C runtime's own state (`__C_phase`, `_exit_status`, `__target_for_exit`,
//! `_IntEnv`, `__NubAt3`) as zero-initialised DATA in guest memory, and the few functions the startup and `exit` need
//! (`__setjmp`, `exit`, `_BreakPoint`). Names are what a real application imports (census of the Systemless toolbox-showcase
//! PEF, see docs/MACPPC-PLUMBING.md); the semantics are the minimum that lets a program run, each limit stated:
//!
//! * the DATA symbols start as zero: `_IntEnv == NULL` means "no MPW tool environment" (an application, not a tool),
//!   `__C_phase == 0` and `_exit_status == 0` are the C runtime's initial state; nothing here inspects them afterwards.
//! * `__setjmp(buf)` saves r1, the caller's RTOC (from its linkage-area slot at 20(r1): the glue is entered with r2 = 0), LR, CR and r13..r31 at `buf` in a PRIVATE layout (23 words) and returns 0. There is no
//!   `longjmp` yet (no fragment in the corpus imports it): a program that jumps back would be an unbound import, named.
//! * `exit(status)` stores the status in `_exit_status`, logs, and ends the run like `ExitToShell`; it does not run `atexit`
//!   handlers or flush C streams (the library's file layer is not provided).
//! * `_BreakPoint` is the MPW debugger trap: logged and ignored, as with no debugger installed.
use crate::symbols::{Binding, Native};

/// The library name as a fragment's import table spells it.
pub const NAME: &str = "StdCLib";

/// Bytes saved by `__setjmp`: r1, r2, LR, CR, r13..r31.
pub const JMP_BUF_WORDS: u32 = 4 + 19;

/// How `name` is served, if StdCLib provides it here.
pub fn lookup(name: &str) -> Option<Binding> {
    Some(match name {
        "__setjmp" => Binding::Native(Native::SetJmp),
        "exit" => Binding::Native(Native::CExit),
        "_BreakPoint" => Binding::Native(Native::BreakPoint),
        "_IntEnv" | "__C_phase" | "_exit_status" => Binding::CData(4),
        "__NubAt3" => Binding::CData(16),
        "__target_for_exit" => Binding::CData(4 * JMP_BUF_WORDS),
        _ => return None,
    })
}
