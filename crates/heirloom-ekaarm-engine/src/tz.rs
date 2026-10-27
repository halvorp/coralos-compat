//! The time zone server client (`RTz`, tz.h). `RTz` is an `RSessionBase`: its one data word is `RHandleBase::iHandle` (e32std.h:4100).
use alloc::format;
use alloc::string::String;
use crate::loader::Process;
use crate::natives::NativeOutcome;

/// A connected session to this engine's time zone server: non-zero.
const TZ_SESSION_HANDLE: u32 = 1;
const KERR_NONE: u32 = 0;

/// The natives of this module; `None` = not ours.
pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let this = p.cpu.regs[0];
    let set = |p: &mut Process, v: u32| -> NativeOutcome {
        match p.mem.vm.write_u32(this, v) {
            Ok(()) => NativeOutcome::Return,
            Err(_) => NativeOutcome::Refused(format!("NEXT-NEEDED tzclient {name}: the session at {this:#x} is unwritable")),
        }
    };
    Some(match name {
        "_ZN3RTzC1Ev" => set(p, 0),
        "_ZN3RTz7ConnectEv" => { let o = set(p, TZ_SESSION_HANDLE); p.cpu.regs[0] = KERR_NONE; o }
        "_ZN3RTz5CloseEv" | "_ZN3RTzD1Ev" => set(p, 0),
        // TInt RTz::ConvertToLocalTime(TTime&): UTC to the device's local time. This engine has no time zone source, so the device
        // is at UTC+0 and the time is left as it is. DELETE WHEN the host supplies a time zone (then apply its rules).
        "_ZNK3RTz18ConvertToLocalTimeER5TTime" => {
            if p.mem.vm.read_u32(this).map_or(true, |h| h == 0) {
                return Some(NativeOutcome::Refused(String::from("NEXT-NEEDED tzclient ConvertToLocalTime: the session is not connected")));
            }
            p.cpu.regs[0] = KERR_NONE;
            NativeOutcome::Return
        }
        _ => return None,
    })
}
