//! `CAknViewAppUi` natives (aknviewappui.h): the view registry and local view activation.
//!
//! Virtual slots of `CAknView`'s primary vtable, counted from the header and CONFIRMED against the FileBrowser view's real vtable
//! (a dump of its 22 slots: the inherited ones hold import stubs whose names fix the neighbours): ~CBase 0,1, Extension_ 2, Id 3
//! (aknview.h `Id`), HandleStatusPaneSizeChange 4, ViewId 5, ProcessCommandL 6, HandleCommandL 7, MenuBar 8, HandleViewRectChange 9,
//! Redraw 10, AknViewActivatedL 11, Reserved_MtsmPosition 12, Reserved_MtsmObject 13, DoActivateL 14, DoDeactivate 15, then the
//! MCoeView overrides ViewActivatedL 16, ViewDeactivated 17, ViewScreenDeviceChangedL 18 and MopSupplyObject 19, MopNext 20,
//! SetEmphasis 21.
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use crate::appstart::{anchor, SlotSpec};
use crate::callback::Continuation;
use crate::loader::Process;
use crate::natives::NativeOutcome;
use crate::view::VIEW_I_APP_UI;

pub const SLOT_VIEW_ID: u32 = 3;
pub const SLOT_VIEW_AKN_VIEW_ACTIVATED_L: u32 = 11;
pub const SLOT_VIEW_DO_ACTIVATE_L: u32 = 14;
pub const INHERITED_AKN_VIEW_ACTIVATED_L: &[&str] = &["_ZN8CAknView17AknViewActivatedLERK10TVwsViewId4TUidRK6TDesC8"];

/// `TVwsViewId` (vwsdef.h): iAppUid, iViewUid. A `TDesC8` reference to nothing: a `TPtrC8` (type 1) of length 0.
const VWS_VIEW_ID_BYTES: u32 = 8;
const EMPTY_TPTRC8: u32 = 1 << crate::desc::KSHIFT_DES_TYPE;

fn find_view(p: &mut Process, appui: u32, uid: u32) -> Result<Option<u32>, String> {
    Ok(p.appui_state(appui)?.views.iter().find(|(_, u)| *u == uid && *u != 0).map(|(v, _)| *v))
}

/// `AddViewL`'s `Id()` returned: remember the view's UID.
pub(crate) fn view_added(p: &mut Process, appui: u32, view: u32, uid: u32) -> Result<(), crate::run::Outcome> {
    let st = p.appui_state(appui).map_err(crate::run::Outcome::Fault)?;
    if let Some(entry) = st.views.iter_mut().find(|(v, _)| *v == view) {
        entry.1 = uid;
    }
    p.log.push(format!("[viewappui] AddViewL: view {view:#x} has Id {uid:#x}"));
    Ok(())
}

/// Is there an activation to deliver for `appui`? With nothing queued and no view active, the default view's activation is
/// queued first: the view server activates it when the application comes to the foreground.
pub(crate) fn has_pending(p: &mut Process, appui: u32) -> bool {
    match p.appui_state(appui) {
        Ok(s) => {
            if s.pending.is_empty() && s.active.is_none() {
                if let Some(v) = s.default_view {
                    s.pending.push_back((v, 0, Vec::new()));
                }
            }
            !s.pending.is_empty()
        }
        // An AppUi that is not a live heap object cannot have had a view added (`AddViewL` refuses it by name), so nothing is queued.
        Err(_) => false,
    }
}

/// Start the oldest queued activation of `appui`: the view server's delivery, reduced to what it does to the view,
/// `view->AknViewActivatedL(prevViewId, customMessageId, customMessage)` (aknview.h), which calls `DoActivateL`. `cont` runs on
/// return. The scratch block holding the previous view id and the message (guest memory, as the callee reads it) is freed by
/// `finish_activation`.
pub(crate) fn start_activation(p: &mut Process, appui: u32, cont: impl FnOnce(u32) -> Continuation) -> Result<(), String> {
    let (view, msg_id, msg) = p.appui_state(appui)?.pending.pop_front().ok_or_else(|| String::from("no activation queued"))?;
    if let Some(old) = p.appui_state(appui)?.active {
        return Err(format!("NEXT-NEEDED avkon CAknViewAppUi view switch from {old:#x} to {view:#x}: DoDeactivate of the active view is not implemented"));
    }
    // The previous view id is the empty one: nothing was active (checked above).
    let block = p.alloc_zeroed(VWS_VIEW_ID_BYTES + 8 + msg.len() as u32).ok_or_else(|| String::from("no guest heap for the activation message"))?;
    let desc = block + VWS_VIEW_ID_BYTES;
    p.mem.vm.write_u32(desc, EMPTY_TPTRC8 | msg.len() as u32).map_err(|_| String::from("activation scratch block unwritable"))?;
    if !msg.is_empty() {
        // The message follows the TPtrC8 header; its iPtr (word 1) points at it.
        let data = desc + 8;
        p.mem.vm.write_u32(desc + 4, data).map_err(|_| String::from("activation scratch block unwritable"))?;
        p.mem.vm.write_bytes(data, &msg).map_err(|_| String::from("activation scratch block unwritable"))?;
    }
    let spec = SlotSpec { class: "CAknView", method: "AknViewActivatedL", slot: SLOT_VIEW_AKN_VIEW_ACTIVATED_L, inherited: INHERITED_AKN_VIEW_ACTIVATED_L };
    let target = anchor(p, view, &spec)?;
    p.appui_state(appui)?.active = Some(view);
    p.log.push(format!("[viewappui] activating view {view:#x} (message id {msg_id:#x}, {} bytes)", msg.len()));
    p.call_guest(target, &[view, block, msg_id, desc], cont(block))
}

/// The activation call returned: release its scratch block.
pub(crate) fn finish_activation(p: &mut Process, block: u32) {
    if let Some(h) = p.heap.as_mut() {
        let _ = h.free(block);
    }
}

/// The natives of this module; `None` = not ours.
pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let (this, r1, r2, r3) = (p.cpu.regs[0], p.cpu.regs[1], p.cpu.regs[2], p.cpu.regs[3]);
    let refuse = |m: String| NativeOutcome::Refused(format!("NEXT-NEEDED avkon {name}: {m}"));
    Some(match name {
        // CAknViewAppUi::AddViewL(CAknView*) (aknviewappui.h): registers the view (RegisterViewL) and takes ownership. The registry
        // needs the view's UID, which only its own `Id()` knows: a guest call.
        "_ZN13CAknViewAppUi8AddViewLEP8CAknView" => {
            let r = p.appui_state(this).map(|s| s.views.push((r1, 0)));
            if let Err(m) = r { return Some(refuse(m)) }
            let spec = SlotSpec { class: "CAknView", method: "Id", slot: SLOT_VIEW_ID, inherited: &[] };
            match anchor(p, r1, &spec).and_then(|t| p.call_guest(t, &[r1], Continuation::ViewAdded { appui: this, view: r1 })) {
                Ok(()) => NativeOutcome::Return,
                Err(m) => refuse(m),
            }
        }
        // CAknViewAppUi::ActivateLocalViewL(TUid) and (TUid, TUid aCustomMessageId, const TDesC8&) (aknviewappui.h): queue the
        // activation for the event loop. An unknown view leaves KErrNotFound, which needs the EHABI unwinder (slice 1b).
        "_ZN13CAknViewAppUi17ActivateLocalViewLE4TUid" | "_ZN13CAknViewAppUi17ActivateLocalViewLE4TUidS0_RK6TDesC8" => {
            let (msg_id, msg) = if name.ends_with("TUid") {
                (0, Ok(Vec::new()))
            } else {
                (r2, crate::desc::read8(p, r3))
            };
            let msg = match msg { Ok(m) => m, Err(m) => return Some(refuse(m)) };
            match find_view(p, this, r1) {
                Ok(Some(view)) => match p.appui_state(this) {
                    Ok(s) => { s.pending.push_back((view, msg_id, msg)); NativeOutcome::Return }
                    Err(m) => refuse(m),
                },
                Ok(None) => refuse(format!("no view with Id {r1:#x}: leaves KErrNotFound (EHABI unwinding is slice 1b)")),
                Err(m) => refuse(m),
            }
        }
        // CCoeAppUi::SetDefaultViewL(const MCoeView&) (coeaui.h): `r1` is the MCoeView subobject of a CAknView (offset 4).
        "_ZN9CCoeAppUi15SetDefaultViewLERK8MCoeView" => {
            let view = r1.wrapping_sub(crate::view::VIEW_MCOE_VIEW);
            match p.appui_state(this) {
                Ok(s) if s.views.iter().any(|(v, _)| *v == view) => { s.default_view = Some(view); NativeOutcome::Return }
                Ok(_) => refuse(format!("MCoeView {r1:#x} is not a registered view (a CAknView's MCoeView is at +{})", crate::view::VIEW_MCOE_VIEW)),
                Err(m) => refuse(m),
            }
        }
        // CAknView::AknViewActivatedL(prev, msgId, msg) (aknview.h): the framework's part of an activation is the menu and CBA
        // (not built, see view.rs) and then the application's own `DoActivateL`, called with the same arguments.
        "_ZN8CAknView17AknViewActivatedLERK10TVwsViewId4TUidRK6TDesC8" => {
            let spec = SlotSpec { class: "CAknView", method: "DoActivateL", slot: SLOT_VIEW_DO_ACTIVATE_L, inherited: &[] };
            match anchor(p, this, &spec).and_then(|t| p.call_guest(t, &[this, r1, r2, r3], Continuation::Resume)) {
                Ok(()) => NativeOutcome::Return,
                Err(m) => refuse(m),
            }
        }
        // CAknView::AppUi() const (aknview.h): iAppUi.
        "_ZNK8CAknView5AppUiEv" => match p.mem.vm.read_u32(this.wrapping_add(VIEW_I_APP_UI)) {
            Ok(a) => { p.cpu.regs[0] = a; NativeOutcome::Return }
            Err(_) => refuse(format!("view at {this:#x} is unreadable")),
        },
        _ => return None,
    })
}
