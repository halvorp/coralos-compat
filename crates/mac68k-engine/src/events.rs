//! The Event Manager over the resumable step model: `GetNextEvent`, `WaitNextEvent`, `EventAvail`, `FlushEvents`,
//! `TickCount`, and the bridge from native [`GuestUiEvent`]s to `EventRecord`s.
//!
//! `EventRecord` (16 bytes): `what` i16, `message` i32, `when` u32 (ticks), `where` Point (v then h), `modifiers` u16.
//!
//! **Blocking.** The guest is a state machine the host steps. An event call with nothing queued *blocks*: the trap
//! leaves its frame popped, records where the Boolean result and the EventRecord go ([`Wait`]) and calls `Cpu::block`;
//! [`Mac::step`] then reports `Step::Blocked` without running a single instruction until a matching event exists, and
//! completes the call (writes the record, stores TRUE) before resuming. `GetNextEvent` and `WaitNextEvent` with a
//! non-zero `sleep` block; `WaitNextEvent(sleep = 0)` and `EventAvail` return the null event (FALSE) at once.
//!
//! **Native events become the events a Mac program already handles** (choices, documented here once):
//! * `CloseRequested` -> `mouseDown` at the window's go-away box ([`go_away_point`]). The program's own
//!   `FindWindow` answers `inGoAway` for exactly that point and its `TrackGoAway` answers TRUE for a point in the box.
//! * `MenuCommand` -> `mouseDown` in the menu bar plus a queued menu choice; the program's `FindWindow` answers
//!   `inMenuBar` and its `MenuSelect` returns the choice as `menuID << 16 | item` (instead of tracking the mouse).
//! * `Resized` -> the window's rect is updated and an `updateEvt` is queued.
//! * A window becoming visible queues `activateEvt` then `updateEvt` (message = the WindowPtr), as the Window Manager does.
//! * `Intent` -> the exact guest input the original OS produced for that gesture ([`crate::translate`]: a `mouseDown` in
//!   the control, then the part code `TrackControl` returns; an `itemHit` for a modal dialog).
//! * `Pointer` / `Key` -> `mouseDown`/`mouseUp`/`keyDown`/`autoKey` in global coordinates, the low-memory `Mouse`/`MBState`
//!   ([`crate::translate::input`]).
use crate::lowmem::TICKS;
use crate::machine::{Call, Done, Mac};
use crate::osconv::Regs;
use crate::pascal::{frame_size, Arg, Rect};
use crate::Refusal;
use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use guestui_abi::{GuestUi, GuestUiEvent};
use retro_m68k::Cpu;

pub const MOUSE_DOWN: u16 = 1;
pub const MOUSE_UP: u16 = 2;
pub const KEY_DOWN: u16 = 3;
pub const AUTO_KEY: u16 = 5;
pub const UPDATE_EVT: u16 = 6;
pub const ACTIVATE_EVT: u16 = 8;
pub const EVENT_RECORD_SIZE: u32 = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MacEvent { pub what: u16, pub message: u32, pub when: u32, pub v: i16, pub h: i16, pub modifiers: u16 }

/// A blocked event call: what it is waiting for and where to deliver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wait { pub mask: u16, pub ev_ptr: u32, pub result_at: u32, pub deadline: Option<u32> }

#[derive(Default)]
pub struct EventMgr {
    pub queue: VecDeque<MacEvent>,
    pub waiting: Option<Wait>,
    pub ticks: u32,
    /// Every tick the host has ever fed [`Mac::advance_ticks`]: `ticks` without the 2^32 wrap, so the virtual-microsecond clock (clock.rs) never loses its place.
    pub ticks_total: u64,
    /// Menu choices (`menuID << 16 | item`) for `MenuSelect` to return, oldest first.
    pub choices: VecDeque<u32>,
}

/// The centre of a window's go-away box in global coordinates: the title bar is the 18 rows above the content,
/// and the box starts 8 pixels in from the left edge and is 13 wide.
pub fn go_away_point(r: &Rect) -> (i16, i16) { (r.top - 9, r.left + 14) }

impl<U: GuestUi> Mac<U> {
    pub fn post(&mut self, what: u16, message: u32, v: i16, h: i16) { self.post_with(what, message, v, h, 0); }

    /// Queue an event with its modifier word (`EventRecord.modifiers`).
    pub fn post_with(&mut self, what: u16, message: u32, v: i16, h: i16, modifiers: u16) {
        let when = self.ev.ticks;
        self.ev.queue.push_back(MacEvent { what, message, when, v, h, modifiers });
    }

    /// The host's 60 Hz clock: advance `Ticks` (and so `TickCount` and event timestamps).
    pub fn advance_ticks(&mut self, n: u32) {
        self.ev.ticks = self.ev.ticks.wrapping_add(n);
        self.ev.ticks_total += u64::from(n);
        let _ = self.mem.write_u32(TICKS, self.ev.ticks);
        let _ = self.update_time();
        for _ in 0..n.min(600) { self.vbl_tick(); } // at most ten seconds of retraces per call: a host that sleeps does not owe them all
    }

    /// Take events the host UI has for this guest into the queue.
    pub fn pump_ui(&mut self) {
        while let Some(ev) = self.ui.poll_event() {
            if let Err(Refusal(why)) = self.deliver(ev) { self.note_refusal("deliver", why); }
        }
    }

    /// A native event for the guest. See the module docs for the mapping.
    pub fn deliver(&mut self, ev: GuestUiEvent) -> Result<Delivered, Refusal> {
        let find = |m: &Self, native| m.windows.iter().find(|(_, w)| w.native == Some(native)).map(|(&p, w)| (p, w.rect));
        match ev {
            GuestUiEvent::CloseRequested { win, .. } => {
                let Some((p, r)) = find(self, win) else { return Ok(Delivered::NotForAGuestWindow) };
                let (v, h) = go_away_point(&r);
                self.post(MOUSE_DOWN, p, v, h);
                Ok(Delivered::Queued)
            }
            GuestUiEvent::MenuCommand { win, token } => {
                if find(self, win).is_none() { return Ok(Delivered::NotForAGuestWindow); }
                let choice = self.menus.choice_for_token(&self.mem, &self.mm, token.0)?;
                self.ev.choices.push_back(choice);
                self.post(MOUSE_DOWN, 0, MENU_BAR_HIT.0, MENU_BAR_HIT.1);
                Ok(Delivered::Queued)
            }
            GuestUiEvent::Resized { win, rect } => {
                let Some((p, _)) = find(self, win) else { return Ok(Delivered::NotForAGuestWindow) };
                let r = Rect { top: rect.top as i16, left: rect.left as i16, bottom: (rect.top + rect.height) as i16, right: (rect.left + rect.width) as i16 };
                self.set_window_rect(p, r)?;
                crate::qd::lifecycle::window_resized(self, p)?;
                self.post(UPDATE_EVT, p, 0, 0);
                Ok(Delivered::Queued)
            }
            GuestUiEvent::Intent { win, control, token, generation, intent } => self.deliver_intent(win, control, token, generation, intent),
            GuestUiEvent::Pointer { win, x, y, button, phase, clicks } => self.deliver_pointer(win, x, y, button, phase, clicks),
            GuestUiEvent::Key { win, text, modifiers, down, code } => self.deliver_key(win, code, &text, modifiers, down),
            ev @ GuestUiEvent::RequestAnswered { .. } => Err(Refusal(format!("deliver: {ev:?}: the Mac engine asks no requests (Alert is a later lane)"))),
        }
    }

    /// Is an event the `mask` accepts queued? (The same predicate [`Mac::take_event`] removes by; a non-event code of 16 or
    /// more matches no mask bit rather than overflowing the shift.)
    pub fn event_pending(&self, mask: u16) -> bool { self.ev.queue.iter().any(|e| mask_accepts(mask, e.what)) }

    fn take_event(&mut self, mask: u16, remove: bool) -> Option<MacEvent> {
        let i = self.ev.queue.iter().position(|e| mask_accepts(mask, e.what))?;
        if remove { self.ev.queue.remove(i) } else { self.ev.queue.get(i).copied() }
    }

    fn write_event(&mut self, at: u32, e: &MacEvent) -> Result<(), Refusal> {
        let r = Refusal::mem("EventRecord");
        self.mem.write_u16(at, e.what).map_err(&r)?;
        self.mem.write_u32(at + 2, e.message).map_err(&r)?;
        self.mem.write_u32(at + 6, e.when).map_err(&r)?;
        self.mem.write_u16(at + 10, e.v as u16).map_err(&r)?;
        self.mem.write_u16(at + 12, e.h as u16).map_err(&r)?;
        self.mem.write_u16(at + 14, e.modifiers).map_err(&r)
    }

    /// Called by [`Mac::step`] before resuming: finish a blocked event call if a matching event has arrived.
    /// Returns false while the guest must stay blocked.
    pub(crate) fn complete_wait(&mut self) -> bool {
        if self.tr.dlg.modal.is_some() { return self.complete_modal(); }
        let Some(w) = self.ev.waiting else { return true };
        self.pump_ui();
        let Some(e) = self.take_event(w.mask, true) else {
            // Nothing queued. A real Mac would have handed the program a NULL event by now (GetNextEvent returns at once; WaitNextEvent
            // returns when its sleep has run out): once the host's clock reaches the deadline the call completes with `what = 0`.
            if !w.deadline.is_some_and(|d| self.ev.ticks.wrapping_sub(d) < 0x8000_0000) { return false; }
            let null = MacEvent { what: 0, message: 0, when: self.ev.ticks, v: 0, h: 0, modifiers: 0 };
            let done = self.write_event(w.ev_ptr, &null).and_then(|_| self.mem.write_u8(w.result_at, 0).map_err(Refusal::mem("event result")));
            if let Err(Refusal(why)) = done { self.note_refusal("WaitNextEvent", why); }
            self.ev.waiting = None;
            return true;
        };
        let done = self.write_event(w.ev_ptr, &e).and_then(|_| self.mem.write_u8(w.result_at, 1).map_err(Refusal::mem("event result")));
        if let Err(Refusal(why)) = done { self.note_refusal("WaitNextEvent", why); }
        self.ev.waiting = None;
        true
    }
}

/// Does an event mask accept event code `what`?
fn mask_accepts(mask: u16, what: u16) -> bool {
    // A high-level (Apple) event has `what` 23, which no 16-bit mask can name: `highLevelEventMask` (0x0400) is its bit.
    if what == crate::appleevents::HIGH_LEVEL_EVENT { return mask & 0x0400 != 0; }
    what < 16 && mask & (1u16 << what) != 0
}

/// What [`Mac::deliver`] did with an event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivered { Queued, NotForAGuestWindow,
    /// Accepted and deliberately not turned into an event (pointer motion, a key release: classic Mac programs poll those).
    Ignored }

/// Where a menu command's `mouseDown` is reported: inside the menu bar (rows 0..20).
pub const MENU_BAR_HIT: (i16, i16) = (5, 10);

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    m.register(0xA970, &[Arg::W, Arg::L], Some(Arg::B), get_next_event::<U>)?;
    m.register(0xA971, &[Arg::W, Arg::L], Some(Arg::B), event_avail::<U>)?;
    m.register(0xA860, &[Arg::W, Arg::L, Arg::L, Arg::L], Some(Arg::B), wait_next_event::<U>)?;
    m.register(0xA975, &[], Some(Arg::L), tick_count::<U>)?;
    m.register(0xA976, &[Arg::L], None, get_keys::<U>)?;
    m.register_os(0xA032, flush_events::<U>)
}

/// `GetKeys(VAR theKeys: KeyMap)`: the 128 key-down bits (16 bytes, Inside Macintosh: Toolbox Event Manager). This engine
/// does not follow key RELEASES (a key press arrives as `keyDown`/`autoKey`; the release is `Ignored`), so it cannot claim
/// any key is held: the map is all zero, and the log says that is the answer rather than a measurement.
fn get_keys<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    m.mem.zero(c.args.get(0), 16).map_err(Refusal::mem("KeyMap"))?;
    Ok(Done { result: 0, log: String::from("[event] GetKeys -> no key held (key releases are not tracked)") })
}

const GET_LAYOUT: [Arg; 2] = [Arg::W, Arg::L];
const WAIT_LAYOUT: [Arg; 4] = [Arg::W, Arg::L, Arg::L, Arg::L];

/// The shared body: return a queued event, or the null event / block.
fn next<U: GuestUi>(m: &mut Mac<U>, cpu: &mut Cpu, c: &Call, layout: &[Arg], remove: bool, block: Option<u32>) -> Result<Done, Refusal> {
    let (mask, ev_ptr) = (c.args.get(0) as u16, c.args.get(1));
    m.pump_ui();
    if let Some(e) = m.take_event(mask, remove) {
        m.write_event(ev_ptr, &e)?;
        return Ok(Done { result: 1, log: format!("[event] {} -> what={} message={:#x}", c.name, e.what, e.message) });
    }
    let null = MacEvent { what: 0, message: 0, when: m.ev.ticks, v: 0, h: 0, modifiers: 0 };
    m.write_event(ev_ptr, &null)?;
    if let Some(sleep) = block {
        let result_at = cpu.areg(7) + frame_size(layout);
        // GetNextEvent has no sleep: its null event is due on the next tick. WaitNextEvent's is due when `sleep` ticks are up
        // (a sleep of 0xFFFFFFFF, or anything past a day, is "until an event comes").
        let deadline = if sleep >= 0x0005_0000 { None } else { Some(m.ev.ticks.wrapping_add(sleep.max(1))) };
        m.ev.waiting = Some(Wait { mask, ev_ptr, result_at, deadline });
        cpu.block();
        return Ok(Done { result: 0, log: format!("[event] {} blocks (mask {mask:#06x})", c.name) });
    }
    Ok(Done { result: 0, log: format!("[event] {} -> null event", c.name) })
}

fn get_next_event<U: GuestUi>(m: &mut Mac<U>, cpu: &mut Cpu, c: &Call) -> Result<Done, Refusal> { next(m, cpu, c, &GET_LAYOUT, true, Some(1)) }
fn event_avail<U: GuestUi>(m: &mut Mac<U>, cpu: &mut Cpu, c: &Call) -> Result<Done, Refusal> { next(m, cpu, c, &GET_LAYOUT, false, None) }
fn wait_next_event<U: GuestUi>(m: &mut Mac<U>, cpu: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let sleep = c.args.get(2);
    next(m, cpu, c, &WAIT_LAYOUT, true, (sleep != 0).then_some(sleep))
}

fn tick_count<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    Ok(Done { result: m.ev.ticks, log: format!("[event] TickCount -> {}", m.ev.ticks) })
}

/// `_FlushEvents`: D0 = `stopMask << 16 | eventMask`. Removes queued events in `eventMask`, stopping at the first one in
/// `stopMask`; D0 comes back 0.
fn flush_events<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> Result<String, Refusal> {
    let (event_mask, stop_mask) = (r.d0 as u16, (r.d0 >> 16) as u16);
    let before = m.ev.queue.len();
    let mut kept = VecDeque::new();
    let mut stopped = false;
    while let Some(e) = m.ev.queue.pop_front() {
        let bit = 1u16 << e.what;
        if !stopped && stop_mask & bit != 0 { stopped = true; }
        if !stopped && event_mask & bit != 0 { continue; }
        kept.push_back(e);
    }
    m.ev.queue = kept;
    r.err(0);
    Ok(format!("[event] FlushEvents mask {event_mask:#06x} stop {stop_mask:#06x}: removed {}", before - m.ev.queue.len()))
}

