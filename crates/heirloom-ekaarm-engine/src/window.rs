//! `RWindow` / `RWindowBase` members (w32std.h) on a control's `iWin`. The application window is the one GuestUi window
//! `BaseConstructL` opened; these attributes tune how the window server composites a window (shadows, fading, pointer grab) and
//! have no counterpart in the host seam, so they are accepted and ignored.
//! DELETE WHEN the host seam grows window attributes (then forward them).
use crate::loader::Process;
use crate::natives::NativeOutcome;

const IGNORED: &[&str] = &[
    "_ZN11RWindowBase17SetShadowDisabledEi",
    "_ZN11RWindowBase14SetPointerGrabEi",
    "_ZN11RWindowBase15SetShadowHeightEi",
    "_ZN7RWindow10InvalidateEv",
    "_ZN7RWindow10InvalidateERK5TRect",
    "_ZN7RWindow11BeginRedrawEv",
    "_ZN7RWindow11BeginRedrawERK5TRect",
    "_ZN7RWindow9EndRedrawEv",
    "_ZN15RWindowTreeNode18SetOrdinalPositionEi",
    "_ZN15RWindowTreeNode27EnableGroupListChangeEventsEv",
    "_ZN15RWindowTreeNode29DisableVisibilityChangeEventsEv",
    "_ZN15RWindowTreeNode24DisableFocusChangeEventsEv",
];

/// Symbian `TDisplayMode::EColor16MU` (32-bit xRGB8888, `gdi.h`): matches our 32-bit host surface and HAL `EDisplayBitsPerPixel = 32`.
const ECOLOR_16MU: u32 = 10;

pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    if name == "_ZN11RWindowBase11DisplayModeEv" {
        p.cpu.regs[0] = ECOLOR_16MU;
        return Some(NativeOutcome::Return);
    }
    if name == "_ZN15RWindowTreeNode28EnableVisibilityChangeEventsEv" {
        p.cpu.regs[0] = 0; // KErrNone
        return Some(NativeOutcome::Return);
    }
    IGNORED.contains(&name).then_some(NativeOutcome::Return)
}
