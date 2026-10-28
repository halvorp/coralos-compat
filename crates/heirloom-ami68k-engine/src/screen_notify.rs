//! intuition.library StartScreenNotifyTagList / EndScreenNotify (rom/intuition/startscreennotifytaglist.c, endscreennotify.c).
//!
//! A program asks to be told when a public screen opens or closes, the Workbench screen opens or closes, or a screen's pens change
//! (MultiView, Wanderer). CoralOS has ONE fixed screen that nothing opens or closes, so such a notification never fires; the registration is
//! still real: Start returns a non-zero token (a zeroed `IntScreenNotify`-sized block, the private value the AROS caller may only test
//! against ZERO) and End gives it back. `EndScreenNotify(NULL)` is TRUE (endscreennotify.c: `if (notify == NULL) return TRUE`); a token
//! that is not outstanding is a named refusal rather than a silent TRUE.
use alloc::{format, string::String};
use crate::{machine::Amiga, memory, Refusal};
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

/// A stand-in for AROS's `struct IntScreenNotify` (node, port, sigbit, sigtask, flags, userdata, hook, pubname).
const NOTIFY_SIZE: u32 = 64;

pub fn register<U: GuestUi>(b: crate::libtable::LibraryBuilder<U>) -> crate::libtable::LibraryBuilder<U> {
    b.named("StartScreenNotifyTagList", start::<U>).named("EndScreenNotify", end::<U>)
}

/// StartScreenNotifyTagList(a0 = tags) -> token, 0 on failure (no memory is a named refusal, not a silent 0).
fn start<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let tags = cpu.areg(0);
    let flags = crate::utility::find(a, SNA_NOTIFY, tags)?.map_or(0, |t| t.1);
    let tok = memory::alloc_zeroed(a, NOTIFY_SIZE, "StartScreenNotifyTagList: IntScreenNotify")?;
    a.screen_notifies.insert(tok);
    cpu.set_d0(tok);
    Ok(format!("[intuition] StartScreenNotifyTagList SNA_Notify={flags:#x} -> {tok:#x} (the one screen never opens or closes: nothing will fire)"))
}

/// EndScreenNotify(a0 = token) -> TRUE.
fn end<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let tok = cpu.areg(0);
    if tok != 0 {
        if !a.screen_notifies.remove(&tok) { return Err(Refusal(format!("EndScreenNotify {tok:#x}: not an outstanding StartScreenNotifyTagList token"))); }
        a.heap.free(tok, NOTIFY_SIZE).map_err(|e| Refusal(format!("EndScreenNotify {tok:#x}: {e}")))?;
    }
    cpu.set_d0(1);
    Ok(format!("[intuition] EndScreenNotify {tok:#x} -> TRUE"))
}

/// SNA_Notify = TAG_USER + 2 (compiler/include/intuition/intuition.h:957).
const SNA_NOTIFY: u32 = 0x8000_0002;
