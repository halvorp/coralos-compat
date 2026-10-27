//! The telephony client (`etel`): this device has no telephony server. Handles are value-initialised; connecting or opening
//! reports `KErrNotSupported` and leaves the handle 0, which is what a phone without an etel server answers (coordinator ruling).
//! `RTelServer` is an `RSessionBase` (one handle word, e32std.h:4100); `RPhone`/`RLine`/`RCall` are `RTelSubSessionBase`, which
//! holds the session and the sub-session handle (two words). Close on a 0 handle is a no-op.
use crate::loader::Process;
use crate::natives::NativeOutcome;

const KERR_NOT_SUPPORTED: u32 = -5i32 as u32; // e32err.h

pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let this = p.cpu.regs[0];
    let words = match name {
        "_ZN10RTelServerC1Ev" => 1,
        "_ZN6RPhoneC1Ev" | "_ZN5RLineC1Ev" | "_ZN5RCallC1Ev" => 2,
        "_ZN10RTelServer7ConnectEi" | "_ZN6RPhone4OpenER10RTelServerRK7TDesC16" | "_ZN5RLine4OpenER6RPhoneRK7TDesC16" => {
            p.cpu.regs[0] = KERR_NOT_SUPPORTED;
            return Some(NativeOutcome::Return);
        }
        "_ZN10RTelServer5CloseEv" | "_ZN6RPhone5CloseEv" | "_ZN5RLine5CloseEv" | "_ZN5RCall5CloseEv" => {
            return Some(match p.mem.vm.read_u32(this) {
                Ok(0) => NativeOutcome::Return,
                _ => NativeOutcome::Refused(alloc::format!("NEXT-NEEDED etel {name}: closing an open handle (no etel handle is ever opened)")),
            });
        }
        _ => return None,
    };
    for w in 0..words {
        if p.mem.vm.write_u32(this + w * 4, 0).is_err() {
            return Some(NativeOutcome::Refused(alloc::format!("NEXT-NEEDED etel {name}: object at {this:#x} unwritable")));
        }
    }
    Some(NativeOutcome::Return)
}
