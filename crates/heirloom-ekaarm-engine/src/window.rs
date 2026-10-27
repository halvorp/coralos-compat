//! `RWindow` / `RWindowBase` members (w32std.h) on a control's `iWin`. The application window is the one GuestUi window
//! `BaseConstructL` opened; these attributes tune how the window server composites a window (shadows, fading, pointer grab) and
//! have no counterpart in the host seam, so they are accepted and ignored.
//! DELETE WHEN the host seam grows window attributes (then forward them).
use crate::loader::Process;
use crate::natives::NativeOutcome;

const IGNORED: &[&str] = &[
    "_ZN11RWindowBase17SetShadowDisabledEi",
];

pub(crate) fn native(_p: &mut Process, name: &str) -> Option<NativeOutcome> {
    IGNORED.contains(&name).then_some(NativeOutcome::Return)
}
