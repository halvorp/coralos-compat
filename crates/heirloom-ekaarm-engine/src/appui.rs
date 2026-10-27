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
        // TRect CEikAppUi::ClientRect() const (eikappui.h): the application rect less the status pane and the CBA, returned through the
        // hidden pointer in r0 (this in r1). No status pane or CBA is constructed (view.rs), so it is the whole application rect.
        // DELETE WHEN the status pane / CBA are built (then subtract their rectangles).
        // TRect CAknView::ClientRect() const (aknview.h): the view's area is the AppUi's client rect (no toolbar is built either).
        "_ZNK9CEikAppUi10ClientRectEv" | "_ZNK13CAknAppUiBase15ApplicationRectEv" | "_ZNK8CAknView10ClientRectEv" => {
            let r = crate::cone::S60_5TH_APP_RECT;
            let tl = (r.left, r.top);
            let br = (r.left + r.width, r.top + r.height);
            let ok = [tl.0, tl.1, br.0, br.1].iter().enumerate().all(|(i, v)| p.mem.vm.write_u32(this.wrapping_add(i as u32 * 4), *v as u32).is_ok());
            if ok { NativeOutcome::Return } else { NativeOutcome::Refused(format!("NEXT-NEEDED eikcore {name}: result at {this:#x} unwritable")) }
        }
        _ => return None,
    })
}
