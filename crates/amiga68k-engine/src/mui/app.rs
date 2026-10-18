//! The Application class: the return-id queue the program's main loop drains, the inbox of native events that loop processes
//! (MUI handles input INSIDE `MUIM_Application_NewInput`, in the task of the program), and the signal that wakes it.
//!
//! A native event ([`Inbound`]) is validated when it arrives (a stale or meaningless one is a named refusal at once) and then
//! waits in the inbox; the program's next `NewInput` applies it exactly as MUI's own input handling would: the attributes the
//! translation table says are set (and notified), `MUIM_CallHook` follow-ups run as guest calls ([`super::hooks`]), and the
//! return ids they produce are handed back.
use alloc::vec::Vec;
use alloc::format;
use super::classes::Kind;
use super::ids_gen::*;
use super::{input, object};
use crate::boopsi::Ext;
use crate::{exec_wait, machine::Amiga, Refusal};
use guestui_abi::{GuestUi, Intent};

/// A native event waiting for the program's next `NewInput`.
#[derive(Debug, Clone)]
pub enum Inbound {
    /// The window's close gadget (the MUI window object).
    Close(u32),
    /// A menu item (the MUI menuitem object) picked in window `win`.
    Menu { item: u32, win: u32 },
    /// A native control's intent (the MUI object it is the projection of).
    Intent { obj: u32, intent: Intent },
    /// `MUIM_Application_PushMethod`: a method to run in the program's own loop.
    Push { dest: u32, id: u32, args: Vec<u32> },
}

fn bit_of<U: GuestUi>(a: &Amiga<U>, app: u32) -> Result<u32, Refusal> {
    match a.objects.map.get(&app).map(|o| &o.ext) { Some(Ext::App { bit, .. }) => Ok(*bit), _ => Err(Refusal("not a MUI Application".into())) }
}

fn wake<U: GuestUi>(a: &mut Amiga<U>, app: u32) -> Result<(), Refusal> {
    let bit = bit_of(a, app)?;
    let task = a.sched.app_task.get(&app).copied().unwrap_or(a.sched.cur);
    exec_wait::signal_task(a, task, 1 << bit)
}

/// MUIM_Application_ReturnID: queue `id` and wake the task (the signal NewInput reported).
pub fn return_id<U: GuestUi>(a: &mut Amiga<U>, app: u32, id: u32) -> Result<(), Refusal> {
    bit_of(a, app)?;
    if id != 0 { if let Some(Ext::App { pending, .. }) = a.objects.map.get_mut(&app).map(|o| &mut o.ext) { pending.push_back(id); } }
    wake(a, app)
}

/// A native event for the application owning `obj`'s tree: queued, and the program woken to process it.
pub fn post<U: GuestUi>(a: &mut Amiga<U>, obj: u32, ev: Inbound) -> Result<(), Refusal> {
    let app = object::ancestor(&a.objects, obj, Kind::Application)
        .ok_or_else(|| Refusal(format!("native event for {obj:#x}: the object belongs to no Application, so no program loop would process it")))?;
    if let Some(Ext::App { inbox, .. }) = a.objects.map.get_mut(&app).map(|o| &mut o.ext) { inbox.push_back(ev); }
    wake(a, app)
}

/// Apply every queued native event, in arrival order (what MUI's input handling does at the top of `NewInput`).
pub fn pump<U: GuestUi>(a: &mut Amiga<U>, app: u32) -> Result<(), Refusal> {
    loop {
        let ev = match a.objects.map.get_mut(&app).map(|o| &mut o.ext) { Some(Ext::App { inbox, .. }) => inbox.pop_front(), _ => None };
        let Some(ev) = ev else { return Ok(()) };
        match ev {
            Inbound::Close(w) => if a.objects.map.contains_key(&w) {
                object::set_attr(a, w, MUIA_Window_CloseRequest, 1)?;
                object::set_attr_quiet(a, w, MUIA_Window_CloseRequest, 0)?; // MUI resets it after notifying
            },
            Inbound::Menu { item, win } => if let Some(user) = super::menu::item_action(a, item) {
                // Zune window.c, IDCMP_MENUPICK: a checkable item toggles, then Trigger, the application's and the window's MenuAction
                // are set to the item's UserData, and the UserData is returned as a ReturnID.
                if a.objects.flag(item, MUIA_Menuitem_Checkit) { let c = !a.objects.flag(item, MUIA_Menuitem_Checked) as u32; object::set_attr(a, item, MUIA_Menuitem_Checked, c)?; }
                object::set_attr(a, item, MUIA_Menuitem_Trigger, item)?;
                object::set_attr(a, app, MUIA_Application_MenuAction, user)?;
                if a.objects.map.contains_key(&win) { object::set_attr(a, win, MUIA_Window_MenuAction, user)?; }
                if user != 0 { return_id(a, app, user)?; }
            },
            Inbound::Push { dest, id, args } => if a.objects.map.contains_key(&dest) { super::method(a, dest, id, &args)?; },
            Inbound::Intent { obj, intent } => if a.objects.map.contains_key(&obj) { input::apply(a, obj, &intent)?; },
        }
    }
}

/// MUIM_Application_NewInput(&sigs): process pending input, then the next pending return id (0 = none). `*sigs` is the
/// signal mask to Wait on, or 0 while more ids are queued (MUI's "do not wait yet").
pub fn new_input<U: GuestUi>(a: &mut Amiga<U>, app: u32, sigs_ptr: u32) -> Result<u32, Refusal> {
    let bit = bit_of(a, app)?;
    pump(a, app)?;
    let (id, more) = match a.objects.map.get_mut(&app).map(|o| &mut o.ext) {
        Some(Ext::App { pending, .. }) => (pending.pop_front().unwrap_or(0), !pending.is_empty()),
        _ => (0, false),
    };
    if sigs_ptr != 0 { a.mem.write_u32(sigs_ptr, if more { 0 } else { 1 << bit }).map_err(Refusal::mem("NewInput signal"))?; }
    Ok(id)
}

/// Every Application object (for tests and the census).
pub fn applications<U: GuestUi>(a: &Amiga<U>) -> Vec<u32> { a.objects.map.iter().filter(|(_, o)| o.kind == Kind::Application).map(|(&k, _)| k).collect() }

/// Where the program-loop stub of `MUIM_Application_Execute` lives (free low memory above the guest-call slots at 0x500-0x53F).
pub const EXECUTE_AT: u32 = 0x540;

/// `MUIM_Application_Execute` (Zune): the application's own main loop, `while (DoMethod(app, MUIM_Application_NewInput, &sigs) != Quit)
/// { if (sigs) Wait(sigs); }`, run as guest code so that every `NewInput` is a real trapped call (and the hooks a notification
/// queues run as guest calls inside it). The 64 bytes are 68020 code written for this repo (A2 = the application):
/// `link a5,#-12; clr.l -4(a5); loop: move.l #MUIM_Application_NewInput,-12(a5); lea -4(a5),a0; move.l a0,-8(a5); movea.l -4(a2),a0;
/// lea -12(a5),a1; movea.l 8(a0),a3; jsr (a3); moveq #-1,d1; cmp.l d1,d0; beq done; move.l -4(a5),d0; beq loop; movea.l 4.w,a6;
/// jsr -318(a6); bra loop; done: unlk a5; rts`  (-318 is exec's Wait, `-4(a2)` the object's class, `8(a0)` its h_Entry).
const EXECUTE_STUB: [u8; 64] = [
    0x4E, 0x55, 0xFF, 0xF4, 0x42, 0xAD, 0xFF, 0xFC, 0x2B, 0x7C, 0x80, 0x42, 0x3B, 0xA6, 0xFF, 0xF4, 0x41, 0xED, 0xFF, 0xFC, 0x2B, 0x48, 0xFF, 0xF8,
    0x20, 0x6A, 0xFF, 0xFC, 0x43, 0xED, 0xFF, 0xF4, 0x26, 0x68, 0x00, 0x08, 0x4E, 0x93, 0x72, 0xFF, 0xB0, 0x81, 0x67, 0x10, 0x20, 0x2D, 0xFF, 0xFC,
    0x67, 0xD6, 0x2C, 0x78, 0x00, 0x04, 0x4E, 0xAE, 0xFE, 0xC2, 0x60, 0xCC, 0x4E, 0x5D, 0x4E, 0x75,
];

/// Start the loop: the guest call that returns 0 when the loop saw `MUIV_Application_ReturnID_Quit`.
pub fn execute<U: GuestUi>(a: &mut Amiga<U>, app: u32) -> Result<crate::guest_call::Next<U>, Refusal> {
    bit_of(a, app)?;
    a.mem.write_bytes(EXECUTE_AT, &EXECUTE_STUB).map_err(Refusal::mem("MUIM_Application_Execute stub"))?;
    Ok(crate::guest_call::Next::Call { target: EXECUTE_AT, d0: 0, aregs: alloc::vec![(2, app)], then: alloc::boxed::Box::new(|_, _, _| Ok(crate::guest_call::Next::Return(0))) })
}
