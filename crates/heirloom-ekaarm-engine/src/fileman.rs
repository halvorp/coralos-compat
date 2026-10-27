//! Opaque service objects the guest only holds and destroys: `CFileMan` (f32file.h: copy / move / delete files),
//! `CDocumentHandler` (commonui) and `CAknInfoPopupNoteController` (avkon tooltips). Their operations are not implemented, so
//! calling one is refused by name; the destructor is the only virtual the guest uses. Each is a CBase-derived guest block with an
//! engine vtable (`enginestub`).
use alloc::format;
use alloc::string::String;
use crate::enginestub::slots_with_gaps;
use crate::loader::Process;
use crate::natives::NativeOutcome;

const OPAQUE_BYTES: u32 = 16;
/// (constructor name, class). `NewL` of each takes whatever it takes; only `CFileMan` checks its session.
const OPAQUE: &[(&str, &str)] = &[
    ("_ZN8CFileMan4NewLER3RFsP16MFileManObserver", "CFileMan"),
    ("_ZN16CDocumentHandler4NewLEv", "CDocumentHandler"),
    ("_ZN27CAknInfoPopupNoteController4NewLEv", "CAknInfoPopupNoteController"),
];

const NOTE_SETTERS: &[&str] = &[
    "_ZN27CAknInfoPopupNoteController18SetTimePopupInViewEi",
    "_ZN27CAknInfoPopupNoteController22SetTimeDelayBeforeShowEi",
];

fn dtor(class: &str, kind: &str) -> String {
    format!("_ZN{}{}{}Ev", class.len(), class, kind)
}

pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let r0 = p.cpu.regs[0];
    let refuse = |m: String| NativeOutcome::Refused(format!("NEXT-NEEDED {name}: {m}"));
    if let Some((_, class)) = OPAQUE.iter().find(|(n, _)| *n == name) {
        if *class == "CFileMan" && p.fs_session(r0).is_none() {
            return Some(refuse(format!("the RFs at {r0:#x} is not connected")));
        }
        let Some(obj) = p.alloc_zeroed(OPAQUE_BYTES) else { return Some(refuse("no guest heap".into())) };
        let (d1, d0) = (dtor(class, "D1"), dtor(class, "D0"));
        let names = [(0, d1.as_str()), (1, d0.as_str()), (2, "_ZN5CBase10Extension_EjRPvS0_")];
        let vptr = match p.build_vtable(class, &slots_with_gaps(3, &names)) { Ok(v) => v, Err(m) => return Some(refuse(m)) };
        if p.mem.vm.write_u32(obj, vptr).is_err() { return Some(refuse("object unwritable".into())); }
        p.cpu.regs[0] = obj;
        return Some(NativeOutcome::Return);
    }
    if OPAQUE.iter().any(|(_, c)| name == dtor(c, "D1") || name == dtor(c, "D0")) {
        if name.ends_with("D0Ev") { if let Some(h) = p.heap.as_mut() { let _ = h.free(r0); } }
        return Some(NativeOutcome::Return);
    }
    // The tooltip's settings: it is never shown because the host has no pointer-hover source. DELETE WHEN hover events reach the guest
    // (then store these and show the note).
    if NOTE_SETTERS.contains(&name) {
        return Some(NativeOutcome::Return);
    }
    match name {
        // void CDocumentHandler::SetExitObserver(MAknServerAppExitObserver*): the observer is told when a launched server app exits.
        // No document is ever launched (operations are refused), so it is never called. DELETE WHEN a CDocumentHandler operation is implemented.
        "_ZN16CDocumentHandler15SetExitObserverEP25MAknServerAppExitObserver" => Some(NativeOutcome::Return),
        _ => None,
    }
}
