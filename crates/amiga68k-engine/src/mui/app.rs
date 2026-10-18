//! The Application class: the return-id queue the program's main loop drains, and the signal that wakes it.
use crate::boopsi::Ext;
use crate::{exec_wait, machine::Amiga, Refusal};
use guestui_abi::GuestUi;

fn bit_of<U: GuestUi>(a: &Amiga<U>, app: u32) -> Result<u32, Refusal> {
    match a.objects.map.get(&app).map(|o| &o.ext) { Some(Ext::App { bit, .. }) => Ok(*bit), _ => Err(Refusal("not a MUI Application".into())) }
}

/// MUIM_Application_ReturnID: queue `id` and wake the task (the signal NewInput reported).
pub fn return_id<U: GuestUi>(a: &mut Amiga<U>, app: u32, id: u32) -> Result<(), Refusal> {
    let bit = bit_of(a, app)?;
    if id != 0 { if let Some(Ext::App { pending, .. }) = a.objects.map.get_mut(&app).map(|o| &mut o.ext) { pending.push_back(id); } }
    let task = a.sched.app_task.get(&app).copied().unwrap_or(a.sched.cur);
    exec_wait::signal_task(a, task, 1 << bit)
}

/// MUIM_Application_NewInput(&sigs): the next pending return id (0 = none). `*sigs` is the signal mask to Wait on, or 0
/// while more ids are queued (MUI's "do not wait yet").
pub fn new_input<U: GuestUi>(a: &mut Amiga<U>, app: u32, sigs_ptr: u32) -> Result<u32, Refusal> {
    let bit = bit_of(a, app)?;
    let (id, more) = match a.objects.map.get_mut(&app).map(|o| &mut o.ext) {
        Some(Ext::App { pending, .. }) => (pending.pop_front().unwrap_or(0), !pending.is_empty()),
        _ => (0, false),
    };
    if sigs_ptr != 0 { a.mem.write_u32(sigs_ptr, if more { 0 } else { 1 << bit }).map_err(Refusal::mem("NewInput signal"))?; }
    Ok(id)
}
