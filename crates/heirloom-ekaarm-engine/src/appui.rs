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
        // hidden pointer in r0 (this in r1). Once the layout data is loaded and the app switched its status pane layout this is the
        // main pane of that layout (avkonlayout.rs), from the CDL data. Before that the status pane layout is the application's core
        // resource, which is not read, so the whole screen. DELETE WHEN the core resource's STATUS_PANE_LAYOUT is read (then the
        // initial layout is known and this always answers from the data).
        // (`ApplicationRect()` stays the whole application window.) TRect CAknView::ClientRect() const (aknview.h): the view's area is the AppUi's client rect (no toolbar is built either).
        "_ZNK9CEikAppUi10ClientRectEv" | "_ZNK13CAknAppUiBase15ApplicationRectEv" | "_ZNK8CAknView10ClientRectEv" => {
            let (tl, br) = match crate::avkonlayout::client_rect(p, name.contains("ApplicationRect")) {
                Some(Ok(r)) => ((r.x0, r.y0), (r.x1, r.y1)),
                Some(Err(m)) => return Some(NativeOutcome::Refused(format!("NEXT-NEEDED avkon {name}: {m}"))),
                None => {
                    let r = crate::cone::S60_5TH_APP_RECT;
                    ((r.left, r.top), (r.left + r.width, r.top + r.height))
                }
            };
            let ok = [tl.0, tl.1, br.0, br.1].iter().enumerate().all(|(i, v)| p.mem.vm.write_u32(this.wrapping_add(i as u32 * 4), *v as u32).is_ok());
            if ok { NativeOutcome::Return } else { NativeOutcome::Refused(format!("NEXT-NEEDED eikcore {name}: result at {this:#x} unwritable")) }
        }
        _ => return None,
    })
}
