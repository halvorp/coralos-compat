//! `RTimer` (e32std.h): a handle (`RHandleBase::iHandle`, word 0, e32std.h:4100) to a kernel timer object. A timer completes a
//! request after a time; nothing here advances time or completes one yet, so `After` / `At` are not implemented (named refusals).
use alloc::format;
use crate::loader::Process;
use crate::natives::NativeOutcome;

/// The handle of a created timer: non-zero. One counter per process is enough, the engine never looks a timer up by handle yet.
const TIMER_HANDLE: u32 = 1;

/// The natives of this module; `None` = not ours.
pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let this = p.cpu.regs[0];
    let set = |p: &mut Process, v: u32, ret: u32| -> NativeOutcome {
        match p.mem.vm.write_u32(this, v) {
            Ok(()) => { p.cpu.regs[0] = ret; NativeOutcome::Return }
            Err(_) => NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: the timer at {this:#x} is unwritable")),
        }
    };
    Some(match name {
        // TInt RTimer::CreateLocal(): a timer in the current thread.
        "_ZN6RTimer11CreateLocalEv" => set(p, TIMER_HANDLE, 0),
        "_ZN6RTimer5CloseEv" => set(p, 0, this),
        _ => return None,
    })
}
