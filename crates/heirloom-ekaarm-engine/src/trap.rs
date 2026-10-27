//! The TRAP bracket of an EHABI build (`__LEAVE_EQUALS_THROW__`): `TRAP(err, f)` compiles to `User::MarkCleanupStack()`, a
//! `try { f; User::UnMarkCleanupStack(h) } catch (XLeaveException&)`. The mark records the cleanup stack's depth so a leave can
//! unwind to it (slice 1b); the handle it returns is the cleanup stack's `TTrapHandler` object, which the guest only hands back.
use alloc::format;
use alloc::string::String;
use crate::loader::Process;
use crate::natives::NativeOutcome;

/// Bytes of the guest block standing for the thread's `TTrapHandler` (a CBase-like object the guest never reads).
const TRAP_HANDLER_BYTES: u32 = 16;

/// The natives of this module; `None` = not ours.
pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let r0 = p.cpu.regs[0];
    let refuse = |m: String| NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: {m}"));
    Some(match name {
        // static TTrapHandler* User::MarkCleanupStack(): open a TRAP bracket at the current cleanup depth.
        "_ZN4User16MarkCleanupStackEv" => {
            let handler = match p.nat.trap_handler {
                Some(h) => h,
                None => match p.alloc_zeroed(TRAP_HANDLER_BYTES) {
                    Some(h) => { p.nat.trap_handler = Some(h); h }
                    None => return Some(refuse(String::from("no guest heap for the trap handler"))),
                },
            };
            let depth = p.nat.cleanup.len() as u32;
            p.nat.trap_marks.push(depth);
            p.cpu.regs[0] = handler;
            NativeOutcome::Return
        }
        // static void User::UnMarkCleanupStack(TTrapHandler*): close the innermost bracket. Items pushed inside it and not popped
        // are a program error (the device panics); a bracket that is not open, or another handler, is refused.
        "_ZN4User18UnMarkCleanupStackEP12TTrapHandler" => {
            if p.nat.trap_handler != Some(r0) {
                return Some(refuse(format!("{r0:#x} is not the trap handler MarkCleanupStack returned")));
            }
            let Some(mark) = p.nat.trap_marks.pop() else { return Some(refuse(String::from("no TRAP bracket is open"))) };
            let depth = p.nat.cleanup.len() as u32;
            if depth != mark {
                return Some(refuse(format!("the cleanup stack holds {depth} items at the end of a TRAP opened at {mark} (a CBASE-STRAY style panic on a device)")));
            }
            NativeOutcome::Return
        }
        _ => return None,
    })
}
