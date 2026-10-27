//! Avkon static utility natives (`AknLayoutUtils`, ...): device-profile queries. The device this engine presents is the S60 5th
//! edition nHD touch phone (`cone::S60_5TH_APP_RECT`), so these answer for that device.
use crate::loader::Process;
use crate::natives::NativeOutcome;

/// The natives of this module; `None` = not ours.
pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    Some(match name {
        // static TBool AknLayoutUtils::PenEnabled(): the 5th edition device has a touch screen (pointer events are the host's
        // pointer input through the UI seam).
        "_ZN14AknLayoutUtils10PenEnabledEv" => { p.cpu.regs[0] = 1; NativeOutcome::Return }
        _ => return None,
    })
}
