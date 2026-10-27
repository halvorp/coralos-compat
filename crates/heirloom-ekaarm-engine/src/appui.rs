//! `CEikAppUi` / `CAknAppUi` natives that read or write the AppUi object's own fields (layout in `appstart`).
use alloc::format;
use crate::appstart::EIKAPPUI_I_DOCUMENT;
use crate::loader::Process;
use crate::natives::NativeOutcome;

/// The natives of this module; `None` = not ours.
pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let this = p.cpu.regs[0];
    Some(match name {
        // CEikAppUi::Document() const (eikappui.h:62): returns iDocument (:147).
        "_ZNK9CEikAppUi8DocumentEv" => match p.mem.vm.read_u32(this.wrapping_add(EIKAPPUI_I_DOCUMENT)) {
            Ok(d) => { p.cpu.regs[0] = d; NativeOutcome::Return }
            Err(_) => NativeOutcome::Refused(format!("NEXT-NEEDED eikcore {name}: the AppUi at {this:#x} is unreadable")),
        },
        _ => return None,
    })
}
