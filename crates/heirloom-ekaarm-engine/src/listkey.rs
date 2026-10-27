//! `CEikListBox::OfferKeyEventL` (EIKLBX.CPP:3233-3803) for the keys the engine implements: the up and down arrows move the current item
//! (`CListBoxView::MoveCursorL(ECursorPreviousItem / ECursorNextItem)` is `iCurrentItemIndex` +- 1, clamped, with the top item following so
//! the current one stays in view), the key-down and key-up events change nothing, and the list then tells the host (`list_sync`: one
//! `SetControl` when the rows or the selection changed) and its observer (`ReportEventL(EEventStateChanged)`, EIKLBX.CPP:3736-3739)
//! when the current item changed. The guest's view (`CListBoxView`, eiklbv.h:888) is where the current item lives; nothing is kept here.
//! Not modelled, and accepted as no-ops by name: `UpdateMarkUnmarkMSKL` (the CBA's middle softkey label, EIKLBX.CPP:3300+) and
//! `UpdateScrollBarThumbs` (the guest's scroll bar thumbs; the host keeps its own, `listbox.rs` `UpdateScrollBarsL`).
//! DELETE WHEN the CBA's middle softkey is projected to the host / the host's list takes its scroll extent from the guest's scroll bar frame.
//! Every other key (Enter / OK, page and loop scrolling, marking, incremental matching) is a named refusal, not a guess.
//! `LafListBox::MapKeyCode` (EIKLBX.CPP:3237) is a LAF library whose source is not in the Foundation tree: it is taken as the identity on
//! the arrow keys (the switch has `case EKeyUpArrow` / `case EKeyDownArrow`, "in Series 60 we always eat uparrow and downarrow").
//! DELETE WHEN a measured S60 run shows MapKeyCode changing an arrow's code.
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use crate::appstart::{anchor, SlotSpec};
use crate::listbox::{LB_I_FLAGS, LB_I_ITEM_HEIGHT, LB_I_VIEW, VIEW_I_CURRENT, VIEW_I_TOP, VIEW_I_VIEW_RECT};
use crate::loader::Process;
use crate::natives::NativeOutcome;

pub const LIST_OFFER_KEY_EVENT_L: &str = "_ZN11CEikListBox14OfferKeyEventLERK9TKeyEvent10TEventCode";
const EKEY_UP_ARROW: u32 = 0xf809;
const EKEY_DOWN_ARROW: u32 = 0xf80a;
/// `EStdKeyYes` (e32keys.h:273): the one key the list hands back unconsumed on an empty list.
const ESTD_KEY_YES: u32 = 0xc4;
/// `TKeyResponse` (w32std.h).
const EKEY_WAS_NOT_CONSUMED: u32 = 0;
const EKEY_WAS_CONSUMED: u32 = 1;
/// The list box construction flags the arrow keys' route depends on (lafpublc.h:122 `EPopout`; eiklbx.h:218 `ELoopScrolling`, :241 `EPageAtOnceScrolling`).
const FLAG_POPOUT: u32 = 0x0008;
const FLAG_LOOP_SCROLLING: u32 = 0x1000;
const FLAG_PAGE_AT_ONCE: u32 = 0x8000;
/// `MCoeControlObserver::EEventStateChanged` (coecobs.h: RequestExit 0, RequestCancel, RequestFocus, PrepareFocusTransition, StateChanged 4).
const EEVENT_STATE_CHANGED: u32 = 4;
/// `MCoeControlObserver::HandleControlEventL` is the mixin's first virtual (coecobs.h): slot 0.
const SLOT_HANDLE_CONTROL_EVENT_L: u32 = 0;

impl Process {
    /// Returns the key response and, when the current item changed, the observer to tell.
    fn list_offer_key(&mut self, lb: u32, key: u32) -> Result<(u32, Option<u32>), String> {
        let (code, scan) = (self.rd(key)?, self.rd(key + 4)?);
        let count = self.list_rows(lb)?.len() as i32;
        if count == 0 {
            // ItemExists(topItemIndex) is false: the key is eaten, except Yes.
            return Ok((if scan == ESTD_KEY_YES { EKEY_WAS_NOT_CONSUMED } else { EKEY_WAS_CONSUMED }, None));
        }
        let flags = self.rd(lb + LB_I_FLAGS)?;
        if flags & (FLAG_POPOUT | FLAG_LOOP_SCROLLING | FLAG_PAGE_AT_ONCE) != 0 {
            return Err(format!("list flags {flags:#x}: pop-out, loop and page-at-once scrolling are not implemented"));
        }
        let view = self.rd(lb + LB_I_VIEW)?;
        if view == 0 { return Err(String::from("OfferKeyEventL before ConstructL: no view")); }
        let delta = match code {
            EKEY_DOWN_ARROW => 1,
            EKEY_UP_ARROW => -1,
            0 => return Ok((EKEY_WAS_CONSUMED, None)), // a key-down / key-up event: the switch's default branch
            other => return Err(format!("key code {other:#x}: only the up and down arrows are implemented (Enter / OK, marking and matching are not)")),
        };
        let old = self.rd(view + VIEW_I_CURRENT)? as i32;
        let new = (old.max(0) + delta + i32::from(old < 0 && delta < 0)).clamp(0, count - 1);
        if new == old {
            return Ok((EKEY_WAS_CONSUMED, None));
        }
        self.wr(view + VIEW_I_CURRENT, new as u32)?;
        self.list_scroll_to_current(lb, view, new)?;
        self.list_sync(lb)?;
        let observer = self.control_state(lb)?.observer;
        Ok((EKEY_WAS_CONSUMED, (observer != 0).then_some(observer)))
    }

    /// Keep the current item inside the view: the top item moves just far enough.
    fn list_scroll_to_current(&mut self, lb: u32, view: u32, current: i32) -> Result<(), String> {
        let item_height = (self.rd(view + crate::listbox::VIEW_I_ITEM_HEIGHT)? as i32).max(self.rd(lb + LB_I_ITEM_HEIGHT)? as i32);
        let height = self.rd(view + VIEW_I_VIEW_RECT + 12)? as i32 - self.rd(view + VIEW_I_VIEW_RECT + 4)? as i32;
        let fit = if item_height > 0 { (height / item_height).max(1) } else { 1 };
        let top = self.rd(view + VIEW_I_TOP)? as i32;
        let new_top = if current < top { current } else if current >= top + fit { current - fit + 1 } else { top };
        self.wr(view + VIEW_I_TOP, new_top as u32)
    }
}

pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    if name != LIST_OFFER_KEY_EVENT_L { return None; }
    let (lb, key) = (p.cpu.regs[0], p.cpu.regs[1]);
    let refuse = |m: String| NativeOutcome::Refused(format!("NEXT-NEEDED eikcoctl {name}: {m}"));
    let (response, observer) = match p.list_offer_key(lb, key) { Ok(r) => r, Err(m) => return Some(refuse(m)) };
    let Some(obs) = observer else {
        p.cpu.regs[0] = response;
        return Some(NativeOutcome::Return);
    };
    // ReportEventL(EEventStateChanged): the observer's HandleControlEventL(this, EEventStateChanged), then OfferKeyEventL returns `response`.
    let spec = SlotSpec { class: "MCoeControlObserver", method: "HandleControlEventL", slot: SLOT_HANDLE_CONTROL_EVENT_L, inherited: &[] };
    let calls: Vec<(u32, [u32; 4])> = match anchor(p, obs, &spec) { Ok(t) => alloc::vec![(t, [obs, lb, EEVENT_STATE_CHANGED, 0])], Err(m) => return Some(refuse(m)) };
    Some(match p.call_chain(calls, response) { Ok(()) => NativeOutcome::Return, Err(m) => refuse(m) })
}
