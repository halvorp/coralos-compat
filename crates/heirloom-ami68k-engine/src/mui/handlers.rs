//! Input handlers (`MUIM_Application_AddInputHandler`) and event handlers (`MUIM_Window_AddEventHandler`) that FIRE.
//!
//! Input handler (`struct MUI_InputHandlerNode`: ihn_Node 0, ihn_Object 8, ihn_Millis(UWORD) 12 / ihn_Current 14 or ihn_Signals(ULONG) 12,
//! ihn_Flags 16, ihn_Method 20; Zune application.h): with `MUIIHNF_TIMER` the application sends a `timerequest` for `ihn_Millis` and, when it
//! comes back in `MUIM_Application_NewInput`, sends it again and does `DoMethod(ihn_Object, ihn_Method)` (application.c). Here the timer
//! is a deadline on the engine's VIRTUAL clock: `advance_clock_to` wakes the application, its next `NewInput` re-arms the node (from the
//! clock at that moment, as Zune's SendIO does) and sends the method. Without the flag the node waits on `ihn_Signals`: `NewInput` sends the
//! method when the signals the program was woken by include them.
//!
//! Event handler (`struct MUI_EventHandlerNode`: ehn_Node 0, ehn_Priority(BYTE) 9, ehn_Flags(UWORD) 10, ehn_Object 12, ehn_Class 16,
//! ehn_Events 20; window.h): a native pointer or key event for the window becomes the `struct IntuiMessage` Intuition would send
//! (content_input.rs), and window.c's `HandleInputEvent` / `HandleRawkey` offer it to the nodes in list order (priority, then address,
//! both descending: `EnqueueByPriAndAddress`): a node whose `ehn_Events` has the message's class, whose object is not disabled (and, with
//! `MUI_EHF_GUIMODE`, can draw and is shown), gets `MUIM_HandleEvent(imsg, muikey)` through `CoerceMethod(ehn_Class, ..)` or, with no class,
//! `DoMethod`; a result with `MUI_EventHandlerRC_Eat` ends the walk. A RAWKEY goes to the active object's nodes, then the default object's,
//! then every other node (the order window.c's `HandleRawkey` uses). NOT modelled: the user's MUI key preferences (`muikey` is always
//! `MUIKEY_NONE`), virtual-group clipping of a press, drag and drop.
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::vec::Vec;
use super::drawn::{self, Step};
use super::ids_gen::*;
use super::{app, notify};
use crate::guest_call::Next;
use crate::{content_input, idcmp, machine::Amiga, Refusal};
use heirloom_ui_abi::{GuestUi, PointerButton, PointerPhase};

const IHN_OBJECT: u32 = 8;
const IHN_STUFF: u32 = 12;
const IHN_FLAGS: u32 = 16;
const IHN_METHOD: u32 = 20;
const MUIIHNF_TIMER: u32 = 1;

const EHN_PRIORITY: u32 = 9;
const EHN_FLAGS: u32 = 10;
const EHN_OBJECT: u32 = 12;
const EHN_CLASS: u32 = 16;
const EHN_EVENTS: u32 = 20;
const MUI_EHF_ALWAYSKEYS: u16 = 1 << 0;
const MUI_EHF_GUIMODE: u16 = 1 << 1;
/// `MUI_EventHandlerRC_Eat`: do not invoke more handlers.
const EAT: u32 = 1;
/// `MUIKEY_NONE` (classes/window.h).
const MUIKEY_NONE: u32 = u32::MAX;

/// An IntuiMessage block plus the longword `IAddress` of a RAWKEY points at (window.c hands the handlers a copy whose IAddress is the dead key).
const IMSG_SIZE: u32 = idcmp::INTUIMESSAGE_SIZE + 4;
const IM_CLASS: u32 = 20;
const IM_CODE: u32 = 24;
const IM_QUALIFIER: u32 = 26;
const IM_IADDRESS: u32 = 28;
const IM_MOUSE_X: u32 = 32;
const IM_MOUSE_Y: u32 = 34;
const IM_SECONDS: u32 = 36;
const IM_MICROS: u32 = 40;
const IM_WINDOW: u32 = 44;

/// One registered input handler.
#[derive(Debug, Clone, Copy)]
pub struct Ihn { pub app: u32, /// The virtual-clock microsecond the timer fires at; `None` = not a timer, or fired and waiting for `NewInput` to re-arm it.
    pub due: Option<u64>, pub timer: bool }

/// One registered event handler: the window it listens for and its place in the window's list.
#[derive(Debug, Clone, Copy)]
pub struct Evh { pub win: u32, pub pri: i8 }

#[derive(Debug, Default)]
pub struct Handlers { pub ihn: BTreeMap<u32, Ihn>, pub evh: BTreeMap<u32, Evh> }

/// A native event waiting for the program's next `NewInput`.
#[derive(Debug, Clone)]
pub enum Event {
    Pointer { win: u32, x: i32, y: i32, button: PointerButton, phase: PointerPhase },
    Key { win: u32, code: u32, down: bool },
}

fn rd32<U: GuestUi>(a: &Amiga<U>, at: u32, what: &'static str) -> Result<u32, Refusal> { a.mem.read_u32(at).map_err(Refusal::mem(what)) }

/// `MUIM_Application_AddInputHandler`.
pub fn add_input<U: GuestUi>(a: &mut Amiga<U>, app: u32, node: u32) -> Result<(), Refusal> {
    if a.objects.handlers.ihn.contains_key(&node) { return Err(Refusal(format!("MUIM_Application_AddInputHandler: node {node:#x} is already added"))); }
    let flags = rd32(a, node + IHN_FLAGS, "ihn_Flags")?;
    let timer = flags & MUIIHNF_TIMER != 0;
    let due = if timer {
        let ms = a.mem.read_u16(node + IHN_STUFF).map_err(Refusal::mem("ihn_Millis"))?;
        if ms == 0 { return Err(Refusal(format!("MUIM_Application_AddInputHandler: node {node:#x} wants a timer of 0 ms (it would fire without end)"))); }
        Some(a.clock_us + ms as u64 * 1000)
    } else { None };
    a.objects.handlers.ihn.insert(node, Ihn { app, due, timer });
    Ok(())
}

/// `MUIM_Application_RemInputHandler`.
pub fn rem_input<U: GuestUi>(a: &mut Amiga<U>, node: u32) -> Result<(), Refusal> {
    a.objects.handlers.ihn.remove(&node).map(|_| ()).ok_or_else(|| Refusal(format!("MUIM_Application_RemInputHandler: {node:#x} was never added")))
}

/// `MUIM_Window_AddEventHandler`.
pub fn add_event<U: GuestUi>(a: &mut Amiga<U>, win: u32, node: u32) -> Result<(), Refusal> {
    if a.objects.handlers.evh.contains_key(&node) { return Err(Refusal(format!("MUIM_Window_AddEventHandler: node {node:#x} is already added"))); }
    let pri = a.mem.read_u8(node + EHN_PRIORITY).map_err(Refusal::mem("ehn_Priority"))? as i8;
    a.objects.handlers.evh.insert(node, Evh { win, pri });
    Ok(())
}

/// `MUIM_Window_RemEventHandler`.
pub fn rem_event<U: GuestUi>(a: &mut Amiga<U>, node: u32) -> Result<(), Refusal> {
    a.objects.handlers.evh.remove(&node).map(|_| ()).ok_or_else(|| Refusal(format!("MUIM_Window_RemEventHandler: {node:#x} was never added")))
}

/// An application or window is gone: its handler nodes are forgotten (their memory is the program's).
pub fn forget<U: GuestUi>(a: &mut Amiga<U>, obj: u32) {
    a.objects.handlers.ihn.retain(|_, h| h.app != obj);
    a.objects.handlers.evh.retain(|_, h| h.win != obj);
}

/// The earliest virtual-clock deadline of an armed input-handler timer.
pub fn next_deadline<U: GuestUi>(a: &Amiga<U>) -> Option<u64> { a.objects.handlers.ihn.values().filter_map(|h| h.due).min() }

/// The clock moved: every armed timer that is due is queued on its application (which is woken), in deadline order.
pub fn fire_due<U: GuestUi>(a: &mut Amiga<U>) -> Result<(), Refusal> {
    let now = a.clock_us;
    let mut due: Vec<(u64, u32, u32)> = a.objects.handlers.ihn.iter().filter_map(|(&n, h)| h.due.filter(|&d| d <= now).map(|d| (d, n, h.app))).collect();
    due.sort();
    for (_, node, application) in due {
        if let Some(h) = a.objects.handlers.ihn.get_mut(&node) { h.due = None; }
        app::post_to(a, application, app::Inbound::Timer(node))?;
    }
    Ok(())
}

/// A timer node came back in `NewInput`: re-armed from the clock now, then its method is sent (application.c).
pub fn timer_fired<U: GuestUi>(a: &mut Amiga<U>, node: u32) -> Result<(), Refusal> {
    if !a.objects.handlers.ihn.get(&node).is_some_and(|h| h.timer) { return Ok(()); } // removed while the message was in flight
    let ms = a.mem.read_u16(node + IHN_STUFF).map_err(Refusal::mem("ihn_Millis"))?;
    if ms == 0 { return Err(Refusal(format!("input handler {node:#x}: ihn_Millis was set to 0"))); }
    let at = a.clock_us + ms as u64 * 1000;
    if let Some(h) = a.objects.handlers.ihn.get_mut(&node) { h.due = Some(at); }
    send_ihn(a, node)
}

fn send_ihn<U: GuestUi>(a: &mut Amiga<U>, node: u32) -> Result<(), Refusal> {
    let (obj, method) = (rd32(a, node + IHN_OBJECT, "ihn_Object")?, rd32(a, node + IHN_METHOD, "ihn_Method")?);
    notify::send(a, obj, method, &[]).map(|_| ())
}

/// The signals every signal handler of `app` waits for.
pub fn signal_mask<U: GuestUi>(a: &Amiga<U>, app: u32) -> u32 {
    a.objects.handlers.ihn.iter().filter(|(_, h)| h.app == app && !h.timer).fold(0, |m, (&n, _)| m | a.mem.read_u32(n + IHN_STUFF).unwrap_or(0))
}

/// `NewInput` was woken by `signal`: every signal handler that waits for one of them gets its method, in the order they were added.
pub fn signalled<U: GuestUi>(a: &mut Amiga<U>, app: u32, signal: u32) -> Result<(), Refusal> {
    let nodes: Vec<u32> = a.objects.handlers.ihn.iter().filter(|(_, h)| h.app == app && !h.timer).map(|(&n, _)| n).collect();
    for n in nodes {
        if signal & rd32(a, n + IHN_STUFF, "ihn_Signals")? != 0 { send_ihn(a, n)?; }
    }
    Ok(())
}

/// Does window `win` have an event handler (so that a native pointer or key event is worth queueing)?
pub fn listening<U: GuestUi>(a: &Amiga<U>, win: u32) -> bool { a.objects.handlers.evh.values().any(|h| h.win == win) }

/// A native event for MUI window `win`: queued on the application, handled in its next `NewInput`.
pub fn deliver<U: GuestUi>(a: &mut Amiga<U>, win: u32, ev: Event) -> Result<(), Refusal> {
    if let Event::Key { code, .. } = &ev { if *code > 0x7F { return Err(Refusal(format!("deliver: key code {code:#x} has no Amiga raw key code (0..=0x7F)"))); } }
    app::post(a, win, app::Inbound::Event(ev))
}

/// The nodes of window `win` in list order.
fn ordered<U: GuestUi>(a: &Amiga<U>, win: u32) -> Vec<u32> {
    let mut v: Vec<(i8, u32)> = a.objects.handlers.evh.iter().filter(|(_, h)| h.win == win).map(|(&n, h)| (h.pri, n)).collect();
    v.sort_by(|x, y| y.cmp(x));
    v.into_iter().map(|(_, n)| n).collect()
}

fn object_of<U: GuestUi>(a: &Amiga<U>, node: u32) -> u32 { a.mem.read_u32(node + EHN_OBJECT).unwrap_or(0) }

/// The nodes a RAWKEY visits, in window.c's order: the active object's, the default object's, every other.
fn key_order<U: GuestUi>(a: &Amiga<U>, win: u32, nodes: Vec<u32>) -> Vec<u32> {
    let active = a.objects.num_or(win, MUIA_Window_ActiveObject, 0);
    let default = a.objects.num_or(win, MUIA_Window_DefaultObject, 0);
    let wants = |a: &Amiga<U>, n: u32| {
        a.mem.read_u32(n + EHN_EVENTS).map_or(false, |e| e & content_input::IDCMP_RAWKEY != 0) || a.mem.read_u16(n + EHN_FLAGS).map_or(false, |f| f & MUI_EHF_ALWAYSKEYS != 0)
    };
    let mut out: Vec<u32> = Vec::new();
    for want in [active, default] {
        if want == 0 { continue; }
        let more: Vec<u32> = nodes.iter().copied().filter(|&n| object_of(a, n) == want && wants(a, n) && !out.contains(&n)).collect();
        out.extend(more);
    }
    out.extend(nodes.iter().copied().filter(|&n| object_of(a, n) != active && object_of(a, n) != default && a.mem.read_u32(n + EHN_EVENTS).map_or(false, |e| e & content_input::IDCMP_RAWKEY != 0)));
    out
}

/// Process one native event in the program's task: build the IntuiMessage and queue one `MUIM_HandleEvent` step per node that listens.
pub fn dispatch<U: GuestUi>(a: &mut Amiga<U>, ev: Event) -> Result<(), Refusal> {
    let (win, x, y, class, code, stamp) = match ev {
        Event::Pointer { win, x, y, button, phase } => {
            let stamp = content_input::hold_buttons(a, win, button, phase);
            let traps_menu = a.objects.flag(win, MUIA_Window_NoMenus);
            let Some((class, code)) = content_input::pointer_message(button, phase, true, traps_menu)? else { return Ok(()) };
            (win, x, y, class, code, stamp)
        }
        Event::Key { win, code, down } => {
            let stamp = content_input::current_stamp(a, win);
            (win, 0, 0, content_input::IDCMP_RAWKEY, code as u16 | if down { 0 } else { 0x80 }, stamp)
        }
    };
    let all = ordered(a, win);
    let nodes: Vec<u32> = if class == content_input::IDCMP_RAWKEY { key_order(a, win, all) } else {
        all.into_iter().filter(|&n| a.mem.read_u32(n + EHN_EVENTS).map_or(false, |e| e & class != 0)).collect()
    };
    if nodes.is_empty() { return Ok(()); }
    let imsg = crate::memory::alloc_zeroed(a, IMSG_SIZE, "MUI event: IntuiMessage")?;
    let e = Refusal::mem("IntuiMessage");
    let surface_win = a.objects.surfaces.get(&win).map_or(0, |s| s.win);
    let (secs, micros) = ((a.clock_us / 1_000_000) as u32, (a.clock_us % 1_000_000) as u32);
    a.mem.write_u32(imsg + IM_CLASS, class).map_err(&e)?;
    a.mem.write_u16(imsg + IM_CODE, code).map_err(&e)?;
    a.mem.write_u16(imsg + IM_QUALIFIER, stamp.qualifier).map_err(&e)?;
    a.mem.write_u32(imsg + IM_IADDRESS, if class == content_input::IDCMP_RAWKEY { imsg + idcmp::INTUIMESSAGE_SIZE } else { 0 }).map_err(&e)?;
    a.mem.write_u16(imsg + IM_MOUSE_X, x as u16).map_err(&e)?;
    a.mem.write_u16(imsg + IM_MOUSE_Y, y as u16).map_err(&e)?;
    a.mem.write_u32(imsg + IM_SECONDS, secs).map_err(&e)?;
    a.mem.write_u32(imsg + IM_MICROS, micros).map_err(&e)?;
    a.mem.write_u32(imsg + IM_WINDOW, surface_win).map_err(&e)?;
    let last = nodes.len() - 1;
    a.objects.draw_steps.extend(nodes.into_iter().enumerate().map(|(i, ehn)| Step::Handle { imsg, ehn, last: i == last }));
    Ok(())
}

/// Run one `Step::Handle`: a guest call of `MUIM_HandleEvent(imsg, MUIKEY_NONE)` on the node's object, or nothing when the node is
/// gone, its object disabled or (GUIMODE) not drawable. `Ok(None)` = no call was made; the queue goes on.
pub fn handle_step<U: GuestUi>(a: &mut Amiga<U>, imsg: u32, ehn: u32, last: bool, d0: u32) -> Result<Option<Next<U>>, Refusal> {
    let skip = |a: &mut Amiga<U>| -> Result<Option<Next<U>>, Refusal> { if last { free(a, imsg)?; } Ok(None) };
    if !a.objects.handlers.evh.contains_key(&ehn) { return skip(a); }
    let (obj, flags, class) = (object_of(a, ehn), a.mem.read_u16(ehn + EHN_FLAGS).map_err(Refusal::mem("ehn_Flags"))?, rd32(a, ehn + EHN_CLASS, "ehn_Class")?);
    if !a.objects.map.contains_key(&obj) || a.objects.flag(obj, MUIA_Disabled) { return skip(a); }
    if flags & MUI_EHF_GUIMODE != 0 && !drawn::can_draw(a, obj) { return skip(a); }
    let cl = if class != 0 { class } else { a.classes.class_of(obj).ok_or_else(|| Refusal(format!("MUIM_HandleEvent: {obj:#x} has no class")))? };
    drawn::call(a, cl, obj, &[MUIM_HandleEvent, imsg, MUIKEY_NONE], d0, alloc::boxed::Box::new(move |a: &mut Amiga<U>, r: u32| {
        if r & EAT != 0 {
            // The event is eaten: the walk ends, the nodes after this one never see it.
            a.objects.draw_steps.retain(|s| !matches!(s, Step::Handle { imsg: m, .. } if *m == imsg));
            free(a, imsg)
        } else if last { free(a, imsg) } else { Ok(()) }
    }))
}

fn free<U: GuestUi>(a: &mut Amiga<U>, imsg: u32) -> Result<(), Refusal> { a.heap.free(imsg, IMSG_SIZE).map_err(|e| Refusal(format!("MUI event IntuiMessage: {e}"))) }
