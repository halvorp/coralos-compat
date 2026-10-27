//! Constructors of the mixin interfaces (`M...` classes): each is a vptr (written by the derived constructor, never here) and the
//! reserved data words the SDK declares after it, which the constructor zeroes.
use crate::loader::Process;
use crate::natives::NativeOutcome;

/// (constructor, bytes of reserved data after the vptr, header that declares them)
const MIXINS: &[(&str, u32, &str)] = &[
    ("_ZN28MCoeViewDeactivationObserverC2Ev", 4, "coeview.h:63 iMCoeViewDeactivationObserver_Reserved1"),
    ("_ZN19MCoeControlObserverC2Ev", 4, "coecobs.h iMCoeControlObserver_Reserved1"),
    ("_ZN25MApaServerAppExitObserverC2Ev", 4, "apaserverapp.h:106 iMApaServerAppExitObserver_Reserved1"),
];

/// The natives of this module; `None` = not ours.
pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let (_, bytes, _) = MIXINS.iter().find(|(n, _, _)| *n == name)?;
    let this = p.cpu.regs[0];
    for off in (0..*bytes).step_by(4) {
        if p.mem.vm.write_u32(this.wrapping_add(4 + off), 0).is_err() {
            return Some(NativeOutcome::Refused(alloc::format!("NEXT-NEEDED {name}: the object at {this:#x} is unwritable")));
        }
    }
    Some(NativeOutcome::Return)
}
