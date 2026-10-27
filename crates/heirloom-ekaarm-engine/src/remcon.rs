//! Remote Control (media keys) natives: `CRemConInterfaceSelector` and `CRemConCoreApiTarget`. The guest never reads their
//! fields (both are NONSHARABLE, remconinterfaceselector.h / remconcoreapitarget.h), so each is a zeroed guest block (the guest
//! holds the pointer) plus host state in the side table (`objects.rs`).
use alloc::format;
use alloc::string::String;
use crate::loader::Process;
use crate::natives::NativeOutcome;
use crate::objects::{HostObj, RemConSelector};

/// Bytes of the placeholder guest block of an interface selector / target: a CBase vptr slot and spare, never read by the guest.
const REMCON_BLOCK_BYTES: u32 = 16;

/// The natives of this module; `None` = not ours.
pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let (r0, r1) = (p.cpu.regs[0], p.cpu.regs[1]);
    let refuse = |m: String| NativeOutcome::Refused(format!("NEXT-NEEDED remcon {name}: {m}"));
    Some(match name {
        // CRemConInterfaceSelector::NewL() (remconinterfaceselector.h): a new selector, owned by the caller.
        "_ZN24CRemConInterfaceSelector4NewLEv" => {
            let Some(a) = p.alloc_zeroed(REMCON_BLOCK_BYTES) else { return Some(refuse(String::from("no guest heap"))) };
            if let Err(m) = p.obj_set(a, HostObj::RemCon(RemConSelector::default())) { return Some(refuse(m)) }
            p.cpu.regs[0] = a;
            NativeOutcome::Return
        }
        // CRemConCoreApiTarget::NewL(CRemConInterfaceSelector&, MRemConCoreApiTargetObserver&) (remconcoreapitarget.h:~20): "owned by
        // the interface selector": registered with it, so the guest never frees it.
        "_ZN20CRemConCoreApiTarget4NewLER24CRemConInterfaceSelectorR28MRemConCoreApiTargetObserver" => {
            let Some(t) = p.alloc_zeroed(REMCON_BLOCK_BYTES) else { return Some(refuse(String::from("no guest heap"))) };
            match p.obj_get(r0) {
                Some(HostObj::RemCon(s)) => s.targets.push((t, r1)),
                _ => return Some(refuse(format!("{r0:#x} is not a live interface selector"))),
            }
            p.cpu.regs[0] = t;
            NativeOutcome::Return
        }
        // CRemConInterfaceSelector::OpenTargetL() (remconinterfaceselector.h:~95): opens the target session to the RemCon server. This
        // engine has no RemCon server and no media-key source, so the session is recorded as open and nothing ever calls the observer.
        // DELETE WHEN the host UI seam delivers media-key events (then route them to the registered observers).
        "_ZN24CRemConInterfaceSelector11OpenTargetLEv" => match p.obj_get(r0) {
            Some(HostObj::RemCon(s)) => { s.target_open = true; NativeOutcome::Return }
            _ => refuse(format!("{r0:#x} is not a live interface selector")),
        },
        _ => return None,
    })
}
