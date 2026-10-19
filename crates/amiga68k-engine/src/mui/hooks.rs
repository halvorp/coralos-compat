//! `MUIM_CallHook`: a notification (or a DoMethod) that calls a guest `struct Hook`. MUI calls it in the program's own task
//! with `A0 = hook, A2 = the object, A1 = the parameters after the hook`; here the call is queued by the method and run as a
//! guest call ([`crate::guest_call`]) when the handler that caused it finishes, so the program sees it before the trapped
//! call returns, exactly where it would on AmigaOS. Facts: Zune's notify.c (MUIM_CallHook passes `&msg->param1`) and exec's
//! `struct Hook` (h_Entry at offset 8).
use alloc::boxed::Box;
use alloc::vec::Vec;
use alloc::format;
use crate::guest_call::{self, Next};
use crate::{machine::Amiga, Refusal};
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

const H_ENTRY: u32 = 8;

/// One queued hook invocation.
#[derive(Debug, Clone)]
pub struct HookCall { pub hook: u32, pub obj: u32, pub msg: Msg, pub after: After }

/// What the engine does with the hook's result (`D0`) once the guest call returns.
#[derive(Debug, Clone)]
pub enum After {
    Nothing,
    /// A commodity message handed to the Application's broker hook: Zune replies it once the hook has returned (mui/broker.rs).
    ReplyCx(u32),
    /// A List's construct hook returned the new entry for row `id` (NULL = the entry is not added).
    Constructed { list: u32, id: u32 },
    /// A List's display hook filled the `cols` cell pointers at `block + 4` for row `id` (`None` = the column titles).
    Displayed { list: u32, id: Option<u32>, block: u32, size: u32, cols: usize },
}

/// What `A1` points at: parameters the engine writes into a block of its own (a notification's), or the guest's own
/// message area (a direct DoMethod: the parameters follow the hook in the message).
#[derive(Debug, Clone)]
pub enum Msg { Params(Vec<u32>), Addr(u32), /// `A1` is this value itself (a List's entry).
    Word(u32),
    /// An `OM_SET` message (`{ OM_SET, AttrList, GInfo }`) over a tag list the engine writes behind it, TAG_DONE-terminated.
    Set(Vec<(u32, u32)>) }

/// Run every queued hook, then finish the trapped call with `d0`.
pub fn run<U: GuestUi>(a: &mut Amiga<U>, d0: u32) -> Result<Next<U>, Refusal> {
    let Some(h) = a.objects.hooks.pop_front() else {
        // Notifications first; then the methods a window with self-drawing objects is owed (drawn.rs).
        return match super::drawn::run_next(a, d0)? { Some(next) => Ok(next), None => Ok(Next::Return(d0)) };
    };
    let entry = a.mem.read_u32(h.hook + H_ENTRY).map_err(Refusal::mem("Hook.h_Entry"))?;
    if entry == 0 { return Err(Refusal(format!("MUIM_CallHook: the hook {:#x} has no h_Entry", h.hook))); }
    let (msg, owned) = match &h.msg {
        Msg::Addr(p) => (*p, None),
        Msg::Word(w) => (*w, None),
        Msg::Set(tags) => {
            let size = 4 * (3 + 2 * tags.len() as u32 + 2);
            let p = a.heap.alloc(size).ok_or_else(|| Refusal("OM_SET: no memory for the message (NOMEM)".into()))?;
            let mut words = alloc::vec![crate::boopsi_ids::OM_SET, p + 12, 0];
            for (t, v) in tags { words.push(*t); words.push(*v); }
            words.extend([0, 0]);
            for (i, x) in words.iter().enumerate() { a.mem.write_u32(p + 4 * i as u32, *x).map_err(Refusal::mem("OM_SET message"))?; }
            (p, Some(size))
        }
        Msg::Params(v) => {
            let size = 4 * (v.len() as u32 + 1);
            let p = a.heap.alloc(size).ok_or_else(|| Refusal("MUIM_CallHook: no memory for the parameters (NOMEM)".into()))?;
            for (i, x) in v.iter().enumerate() { a.mem.write_u32(p + 4 * i as u32, *x).map_err(Refusal::mem("MUIM_CallHook parameters"))?; }
            (p, Some(size))
        }
    };
    let after = h.after.clone();
    Ok(Next::Call { target: entry, d0: 0, aregs: alloc::vec![(0, h.hook), (1, msg), (2, h.obj)], then: Box::new(move |a: &mut Amiga<U>, _cpu: &mut Cpu, r: u32| {
        if let Some(size) = owned { a.heap.free(msg, size).map_err(|e| Refusal(format!("MUIM_CallHook: {e}")))?; }
        if let After::ReplyCx(m) = after { crate::exec_wait::reply_to_port(a, m)?; }
        super::list::hook_returned(a, &after, r)?;
        run(a, d0)
    }) })
}

/// Called by a trapped MUI entry point after it has done its work and chosen its result `d0`: if notifications queued hooks,
/// they run now; the entry point's own result is returned when the last one is done.
pub fn after_call<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, d0: u32) -> Result<(), Refusal> {
    if a.objects.hooks.is_empty() && (a.objects.draw_steps.is_empty() || a.objects.draw_busy) { cpu.set_d0(d0); return Ok(()); }
    let next = run(a, d0)?;
    guest_call::start(a, cpu, next)
}
