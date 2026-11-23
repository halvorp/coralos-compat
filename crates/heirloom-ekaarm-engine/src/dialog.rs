//! Modal Avkon dialogs (`CAknMessageQueryDialog`, `CEikDialog`, `CAknPopupHeadingPane`):
//! projected through `GuestUi::request` as native modal alert dialogs, parking the CPU until
//! `GuestUiEvent::RequestResponded` returns the user's button choice.
//!
//! Verified virtual slots (`CAknMessageQueryDialog` inherits `CAknQueryDialog` -> `CAknDialog` -> `CEikDialog` -> `CCoeControl`):
//! - slot 46 (`+184`): `CEikDialog::PrepareLC(TInt aResourceId)`
//! - slot 47 (`+188`): `CEikDialog::RunLD()`
//! - slot 77 (`+308`): `CAknQueryDialog::QueryHeading() const`
use alloc::format;
use alloc::string::String;
use alloc::vec;
use heirloom_ui_abi::{RequestKind, RequestModel, RequestRef};
use crate::cleanup::CleanupItem;
use crate::loader::Process;
use crate::natives::NativeOutcome;
use crate::objects::{DialogState, HostObj};

/// `EEikBidOk` (eikon.hrh): return value of `CEikDialog::RunLD()` when confirmed.
pub const EEIK_BID_OK: u32 = 1;

const CLASS: &str = "CAknMessageQueryDialog";
/// Slots past `CCoeControl`'s 44: up to slot 77 (`QueryHeading`) plus headroom.
const TAIL: usize = 40;
const OVERRIDES: &[(usize, &str)] = &[
    (46, "_ZN10CEikDialog9PrepareLCEi"),
    (47, "_ZN10CEikDialog5RunLDEv"),
    (77, "_ZNK15CAknQueryDialog12QueryHeadingEv"),
];

impl Process {
    fn dialog_construct(&mut self, this: u32) -> Result<(), String> {
        self.attach_control(this, CLASS, TAIL, OVERRIDES)?;
        self.obj_set(this, HostObj::Dialog(DialogState::default()))
    }

    /// Complete a parked modal dialog (`CEikDialog::RunLD`) once the host responds to `GuestUi::request`.
    pub fn deliver_request_response(&mut self, req: RequestRef, button: Option<u32>) -> Result<(), String> {
        let Some((pending_req, dlg)) = self.nat.pending_request else {
            return Err(format!("no modal dialog is waiting for request {req:?}"));
        };
        if pending_req != req {
            return Err(format!("request response {req:?} does not match pending {pending_req:?}"));
        }
        self.nat.pending_request = None;
        // RunLD owns the dialog on the cleanup stack and deletes it before returning.
        if let Some(pos) = self.nat.cleanup.iter().rposition(|it| matches!(it, CleanupItem::Base(a) if *a == dlg)) {
            self.nat.cleanup.remove(pos);
        }
        let heading = match self.obj_get(dlg) {
            Some(HostObj::Dialog(d)) => d.heading_pane,
            _ => 0,
        };
        if heading != 0 {
            self.obj_remove(heading);
            if let Some(h) = self.heap.as_mut() { let _ = h.free(heading); }
        }
        self.obj_remove(dlg);
        if let Some(h) = self.heap.as_mut() { let _ = h.free(dlg); }
        self.cpu.regs[0] = if button == Some(0) { EEIK_BID_OK } else { 0 };
        Ok(())
    }
}

pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let (r0, r1, r2) = (p.cpu.regs[0], p.cpu.regs[1], p.cpu.regs[2]);
    let refuse = |m: String| NativeOutcome::Refused(format!("NEXT-NEEDED avkon {name}: {m}"));
    Some(match name {
        "_ZN22CAknMessageQueryDialogC1Ev" | "_ZN22CAknMessageQueryDialogC2Ev" => {
            match p.dialog_construct(r0) {
                Ok(()) => { p.cpu.regs[0] = r0; NativeOutcome::Return }
                Err(m) => refuse(m),
            }
        }
        // CAknQueryDialog* CAknQueryDialog::NewL(const TTone& aTone)
        "_ZN15CAknQueryDialog4NewLERKNS_5TToneE" => {
            let res = (|| -> Result<u32, String> {
                let dlg = p.alloc_zeroed(0x100).ok_or_else(|| String::from("no heap for CAknQueryDialog"))?;
                p.dialog_construct(dlg)?;
                Ok(dlg)
            })();
            match res {
                Ok(dlg) => { p.cpu.regs[0] = dlg; NativeOutcome::Return }
                Err(m) => refuse(m),
            }
        }
        // CAknMessageQueryDialog* CAknMessageQueryDialog::NewL(TDesC16& aMessage, const TTone& aTone)
        "_ZN22CAknMessageQueryDialog4NewLER7TDesC16RKN15CAknQueryDialog5TToneE" => {
            let res = (|| -> Result<u32, String> {
                let msg = crate::desc::read16(p, r0).map(|u| crate::desc::to_utf8(&u))?;
                let dlg = p.alloc_zeroed(0x100).ok_or_else(|| String::from("no heap for CAknMessageQueryDialog"))?;
                p.dialog_construct(dlg)?;
                if let Some(HostObj::Dialog(d)) = p.obj_get(dlg) {
                    d.text = msg;
                }
                Ok(dlg)
            })();
            match res {
                Ok(dlg) => { p.cpu.regs[0] = dlg; NativeOutcome::Return }
                Err(m) => refuse(m),
            }
        }
        "_ZN22CAknMessageQueryDialogD1Ev" | "_ZN22CAknMessageQueryDialogD0Ev" => {
            p.obj_remove(r0);
            if name.ends_with("D0Ev") {
                if let Some(h) = p.heap.as_mut() { let _ = h.free(r0); }
            }
            p.cpu.regs[0] = r0;
            NativeOutcome::Return
        }
        // void CEikDialog::PrepareLC(TInt aResourceId): pushes `this` onto the CleanupStack.
        "_ZN10CEikDialog9PrepareLCEi" => {
            if !matches!(p.obj_get(r0), Some(HostObj::Dialog(_))) {
                return Some(refuse(format!("{r0:#x} is not a dialog this engine made")));
            }
            p.nat.cleanup.push(CleanupItem::Base(r0));
            NativeOutcome::Return
        }
        // CAknPopupHeadingPane* CAknQueryDialog::QueryHeading() const
        "_ZNK15CAknQueryDialog12QueryHeadingEv" => {
            let existing = match p.obj_get(r0) {
                Some(HostObj::Dialog(d)) => d.heading_pane,
                _ => return Some(refuse(format!("{r0:#x} is not a dialog this engine made"))),
            };
            if existing != 0 {
                p.cpu.regs[0] = existing;
                return Some(NativeOutcome::Return);
            }
            let res = (|| -> Result<u32, String> {
                let hp = p.plain_object("CAknPopupHeadingPane", 0x40)?;
                p.obj_set(hp, HostObj::HeadingPane(r0))?;
                if let Some(HostObj::Dialog(d)) = p.obj_get(r0) {
                    d.heading_pane = hp;
                }
                Ok(hp)
            })();
            match res {
                Ok(hp) => { p.cpu.regs[0] = hp; NativeOutcome::Return }
                Err(m) => refuse(m),
            }
        }
        // void CAknPopupHeadingPane::SetTextL(TPtrC16 aText): TPtrC16 passed by value in r1 (type|len), r2 (ptr).
        "_ZN20CAknPopupHeadingPane8SetTextLE7TPtrC16" => {
            let dlg = match p.obj_get(r0) {
                Some(HostObj::HeadingPane(d)) => *d,
                _ => return Some(refuse(format!("{r0:#x} is not a CAknPopupHeadingPane this engine made"))),
            };
            let len = (r1 & 0x0fff_ffff) as usize;
            let mut units = alloc::vec::Vec::with_capacity(len);
            for i in 0..len {
                match p.mem.vm.read_u16(r2 + i as u32 * 2) {
                    Ok(u) => units.push(u),
                    Err(_) => return Some(refuse(format!("heading text ptr {r2:#x} unreadable"))),
                }
            }
            let text = crate::desc::to_utf8(&units);
            if let Some(HostObj::Dialog(d)) = p.obj_get(dlg) {
                d.title = text;
                NativeOutcome::Return
            } else {
                refuse(format!("owning dialog {dlg:#x} no longer exists"))
            }
        }
        // void CAknMessageQueryDialog::SetMessageTextL(const TDesC16& aMessage)
        "_ZN22CAknMessageQueryDialog15SetMessageTextLERK7TDesC16" => {
            let text = match crate::desc::read16(p, r1) {
                Ok(u) => crate::desc::to_utf8(&u),
                Err(m) => return Some(refuse(m)),
            };
            if let Some(HostObj::Dialog(d)) = p.obj_get(r0) {
                d.text = text;
                NativeOutcome::Return
            } else {
                refuse(format!("{r0:#x} is not a dialog this engine made"))
            }
        }
        // TInt CEikDialog::RunLD(): present native GuestUi::request and block until RequestResponded.
        "_ZN10CEikDialog5RunLDEv" => {
            let (title, text) = match p.obj_get(r0) {
                Some(HostObj::Dialog(d)) => (d.title.clone(), d.text.clone()),
                _ => return Some(refuse(format!("{r0:#x} is not a dialog this engine made"))),
            };
            let win = p.host.window;
            let Some(ui) = p.host.ui.as_mut() else {
                return Some(refuse(String::from("no GuestUi for modal dialog")));
            };
            let model = RequestModel {
                title,
                text,
                buttons: vec![String::from("OK")],
                default: Some(0),
                cancel: Some(0),
                kind: RequestKind::Note,
            };
            match ui.request(win, model) {
                Ok(req_ref) => {
                    p.nat.pending_request = Some((req_ref, r0));
                    NativeOutcome::Blocked
                }
                Err(e) => refuse(format!("GuestUi::request refused: {e}")),
            }
        }
        _ => return None,
    })
}
