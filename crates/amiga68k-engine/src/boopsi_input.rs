//! Pointer input for guest-drawn (BOOPSI) gadgets, the way Intuition drives them: a button press is hit-tested against
//! the window's custom gadgets (GM_HITTEST), the gadget that answers GMR_GADGETHIT gets GM_GOACTIVE, then every pointer
//! event goes to it as GM_HANDLEINPUT until it answers something other than GMR_MEACTIVE (0); then GM_GOINACTIVE, and a
//! GMR_VERIFY answer posts IDCMP_GADGETUP with the gadget as IAddress and `*gpi_Termination` as Code.
//! All of that is guest code (the class dispatcher), so it needs a guest call, and guest calls start only from inside a
//! trapped call. [`Amiga::deliver`](crate::Amiga::deliver) therefore only QUEUES the event; the guest's next Wait/WaitPort
//! runs it ([`pump`]) and then performs the wait it was making. A guest already blocked in a Wait is sent back through
//! its own Wait trap by `Amiga::step` (the trap address is recorded when it blocks), which is the pump's entry.
//! Facts: AROS rom/intuition inputhandler gadget code, gadgetclass.h (gpHitTest/gpInput/gpGoInactive, GMR_*), classusr.
use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use alloc::format;
use crate::boopsi_ids::*;
use crate::exec_wait::Waiting;
use crate::guest_call::{self, Next};
use crate::idcmp::IDCMP_GADGETUP;
use crate::{machine::Amiga, memory, Delivered, Refusal};
use guestui_abi::{GuestUi, PointerPhase};
use retro_m68k::Cpu;

const SCRATCH: u32 = 96;
const IE_OFF: u32 = 32; // InputEvent (22 bytes) in the scratch block
const TERM_OFF: u32 = 60; // *gpi_Termination
const IECLASS_RAWMOUSE: u8 = 2;
const IECODE_LBUTTON: u16 = 0x68;
const IECODE_UP: u16 = 0x80;
const IECODE_NOBUTTON: u16 = 0xFF;
const IEQUALIFIER_LEFTBUTTON: u16 = 0x4000;
const GMR_VERIFY: u32 = 8;

pub struct Ptr { pub win: u32, pub x: i32, pub y: i32, pub phase: PointerPhase }

/// Queued pointer events, the gadget that has the pointer (window, gadget), and the trap a blocked Wait sits on.
#[derive(Default)]
pub struct Input { pub q: VecDeque<Ptr>, pub active: Option<(u32, u32)>, pub trap_pc: u32, pub wait_trap: u32 }

impl Input { pub fn pending(&self) -> bool { !self.q.is_empty() } }

/// A pointer event for guest window `win` (idcmp::deliver's arm).
pub fn queue<U: GuestUi>(a: &mut Amiga<U>, win: u32, x: i32, y: i32, phase: PointerPhase) -> Result<Delivered, Refusal> {
    let has = crate::boopsi_glist::window_gadgets(a, win)?.iter().any(|g| a.classes.class_of(*g).is_some());
    if !has && a.input.active.is_none() { return Ok(Delivered::IgnoredUnsubscribed); }
    a.input.q.push_back(Ptr { win, x, y, phase });
    Ok(Delivered::Handled)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage { Hit, GoActive, Input, GoInactive { verify: bool, code: u32 } }

#[derive(Clone)]
struct Job { win: u32, x: i32, y: i32, phase: PointerPhase, cands: Vec<u32>, idx: usize, stage: Stage, g: u32, scratch: u32, gi: u32, wait: (Waiting, u32) }

/// Called by Wait/WaitPort before they wait: if pointer events are queued, run them first (true = the call is in
/// progress and the handler must return without waiting itself).
pub fn pump<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, w: Waiting) -> Result<bool, Refusal> {
    if !a.input.pending() { return Ok(false); }
    let next = begin(a, cpu, (w, a.input.wait_trap))?;
    guest_call::start(a, cpu, next)?;
    Ok(true)
}

fn rd16<U: GuestUi>(a: &Amiga<U>, at: u32) -> Result<i32, Refusal> { a.mem.read_u16(at).map(|v| v as i16 as i32).map_err(Refusal::mem("Gadget")) }

/// The gadget's box in window coordinates (boopsigadgets.c: RELRIGHT adds Width-1, RELBOTTOM Height-1, RELWIDTH/RELHEIGHT the size).
fn gadget_box<U: GuestUi>(a: &Amiga<U>, win: u32, g: u32) -> Result<(i32, i32, i32, i32), Refusal> {
    let fl = a.mem.read_u16(g + GAD_FLAGS).map_err(Refusal::mem("Gadget.Flags"))? as u32;
    let (ww, wh) = (rd16(a, win + 8)?, rd16(a, win + 10)?);
    let (mut l, mut t, mut w, mut h) = (rd16(a, g + GAD_LEFT)?, rd16(a, g + GAD_TOP)?, rd16(a, g + GAD_WIDTH)?, rd16(a, g + GAD_HEIGHT)?);
    if fl & GFLG_RELRIGHT != 0 { l += ww - 1; }
    if fl & GFLG_RELBOTTOM != 0 { t += wh - 1; }
    if fl & GFLG_RELWIDTH != 0 { w += ww; }
    if fl & GFLG_RELHEIGHT != 0 { h += wh; }
    Ok((l, t, w, h))
}

fn finish_wait<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, (w, trap): (Waiting, u32)) -> Result<Next<U>, Refusal> {
    match a.try_satisfy(w)? {
        Some(v) => Ok(Next::Return(v)),
        None => { a.waiting = Some(w); a.input.wait_trap = trap; cpu.block(); Ok(Next::Return(0)) }
    }
}

/// Take queued events until one needs a guest call; with none left, do the wait the guest was making.
fn begin<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, wait: (Waiting, u32)) -> Result<Next<U>, Refusal> {
    while let Some(ev) = a.input.q.pop_front() {
        if !a.windows.contains_key(&ev.win) { continue; }
        let base = Job { win: ev.win, x: ev.x, y: ev.y, phase: ev.phase, cands: Vec::new(), idx: 0, stage: Stage::Hit, g: 0, scratch: 0, gi: 0, wait };
        match (ev.phase, a.input.active) {
            (PointerPhase::Down, None) => {
                let mut cands = Vec::new();
                for g in crate::boopsi_glist::window_gadgets(a, ev.win)? {
                    if a.classes.class_of(g).is_none() { continue; }
                    if a.mem.read_u16(g + GAD_FLAGS).map_err(Refusal::mem("Gadget.Flags"))? as u32 & GFLG_DISABLED != 0 { continue; }
                    let (l, t, w, h) = gadget_box(a, ev.win, g)?;
                    if ev.x >= l && ev.x < l + w && ev.y >= t && ev.y < t + h { cands.push(g); }
                }
                if cands.is_empty() { continue; }
                return issue(a, Job { g: cands[0], cands, ..base });
            }
            (_, Some((w, g))) if w == ev.win => return issue(a, Job { stage: Stage::Input, g, ..base }),
            _ => continue,
        }
    }
    finish_wait(a, cpu, wait)
}

/// Build the message for `job.stage` and call the gadget's dispatcher.
fn issue<U: GuestUi>(a: &mut Amiga<U>, mut job: Job) -> Result<Next<U>, Refusal> {
    let g = job.g;
    let cl = a.classes.class_of(g).ok_or_else(|| Refusal(format!("pointer input: {g:#x} is not a BOOPSI object")))?;
    let entry = a.mem.read_u32(cl + CL_H_ENTRY).map_err(Refusal::mem("cl_Dispatcher.h_Entry"))?;
    job.gi = crate::boopsi_gadget::make_ginfo(a, job.win, 0, g)?;
    job.scratch = memory::alloc_zeroed(a, SCRATCH, "gadget input scratch")?;
    let (l, t, _, _) = gadget_box(a, job.win, g)?;
    let (mx, my) = (job.x - l, job.y - t);
    let s = job.scratch;
    let e = Refusal::mem("gadget input message");
    let xy = ((mx as u16 as u32) << 16) | (my as u16 as u32);
    match job.stage {
        Stage::Hit => { a.mem.write_u32(s, GM_HITTEST).map_err(&e)?; a.mem.write_u32(s + 4, job.gi).map_err(&e)?; a.mem.write_u32(s + GPHT_MOUSE, xy).map_err(&e)?; }
        Stage::GoActive | Stage::Input => {
            let id = if job.stage == Stage::GoActive { GM_GOACTIVE } else { GM_HANDLEINPUT };
            a.mem.write_u32(s, id).map_err(&e)?;
            a.mem.write_u32(s + 4, job.gi).map_err(&e)?;
            a.mem.write_u32(s + GPI_IEVENT, s + IE_OFF).map_err(&e)?;
            a.mem.write_u32(s + GPI_TERMINATION, s + TERM_OFF).map_err(&e)?;
            a.mem.write_u32(s + GPI_MOUSE, xy).map_err(&e)?;
            let (code, qual) = match job.phase {
                PointerPhase::Down => (IECODE_LBUTTON, IEQUALIFIER_LEFTBUTTON),
                PointerPhase::Up => (IECODE_LBUTTON | IECODE_UP, 0),
                PointerPhase::Move => (IECODE_NOBUTTON, IEQUALIFIER_LEFTBUTTON),
            };
            a.mem.write_u8(s + IE_OFF + 4, IECLASS_RAWMOUSE).map_err(&e)?;
            a.mem.write_u16(s + IE_OFF + 6, code).map_err(&e)?;
            a.mem.write_u16(s + IE_OFF + 8, qual).map_err(&e)?;
            a.mem.write_u16(s + IE_OFF + 10, job.x as u16).map_err(&e)?;
            a.mem.write_u16(s + IE_OFF + 12, job.y as u16).map_err(&e)?;
        }
        Stage::GoInactive { .. } => { a.mem.write_u32(s, GM_GOINACTIVE).map_err(&e)?; a.mem.write_u32(s + 4, job.gi).map_err(&e)?; a.mem.write_u32(s + GPGI_ABORT, 0).map_err(&e)?; }
    }
    let then = Box::new(move |a: &mut Amiga<U>, cpu: &mut Cpu, d0: u32| advance(a, cpu, job, d0));
    Ok(Next::Call { target: entry, d0: 0, aregs: alloc::vec![(0, cl), (1, s), (2, g)], then })
}

fn advance<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, job: Job, d0: u32) -> Result<Next<U>, Refusal> {
    let term = a.mem.read_u32(job.scratch + TERM_OFF).map_err(Refusal::mem("gpi_Termination"))?;
    a.heap.free(job.scratch, SCRATCH).map_err(|e| Refusal(format!("gadget input scratch: {e}")))?;
    crate::boopsi_gadget::free_ginfo(a, job.gi)?;
    let (win, g) = (job.win, job.g);
    match job.stage {
        Stage::Hit => {
            if d0 == GMR_GADGETHIT { return issue(a, Job { stage: Stage::GoActive, ..job }); }
            let idx = job.idx + 1;
            match job.cands.get(idx) {
                Some(&n) => issue(a, Job { idx, g: n, ..job }),
                None => begin(a, cpu, job.wait),
            }
        }
        Stage::GoActive | Stage::Input => {
            if d0 == 0 { a.input.active = Some((win, g)); return begin(a, cpu, job.wait); }
            issue(a, Job { stage: Stage::GoInactive { verify: d0 & GMR_VERIFY != 0, code: term }, ..job })
        }
        Stage::GoInactive { verify, code } => {
            a.input.active = None;
            if verify { crate::idcmp::post(a, win, IDCMP_GADGETUP, code as u16, g)?; }
            begin(a, cpu, job.wait)
        }
    }
}
