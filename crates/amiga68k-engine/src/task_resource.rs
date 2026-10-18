//! AROS's `task.resource`: per-task storage slots and task hooks. AROS-built m68k programs open it first thing
//! (compiler/startup/startup.c: `OpenResource("task.resource")`, then `InitTaskHooks`/`RunTaskHooks`) and the rellib
//! `stdc.library` they all need allocates a storage slot from it in its init; with no such resource every AROS binary in the
//! corpus stopped at `stdc.library`'s init (corpus census 2026-10-18, 60 of 60). Real AmigaOS has no such resource, so the
//! resource is only handed out under that exact name.
//!
//! Vector numbers are from the call sites in the shipped binaries (gtdemo: `jsr -54(a6)` = InitTaskHooks, `-66` = RunTaskHooks)
//! and the order of AROS rom/task/task.conf: InitTaskHooks -54, AddTaskHook -60, RunTaskHooks -66, AllocTaskStorageSlot -72,
//! FreeTaskStorageSlot -78, SaveTaskStorage -84, RestoreTaskStorage -90, SetTaskStorageSlot -96, GetTaskStorageSlot -102,
//! GetParentTaskStorageSlot -108. Semantics: rom/task/{AllocTaskStorageSlot,SetTaskStorageSlot,GetTaskStorageSlot}.c.
//!
//! What is NOT modelled is refused by name: running hooks that were registered (RunTaskHooks with hooks of that type),
//! SaveTaskStorage/RestoreTaskStorage, the parent slot of a task other than the main program. No hooks registered = nothing to run = TRUE, which is what the real call returns.
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;
use crate::libtable::LibraryBuilder;
use crate::{machine::Amiga, Refusal};
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

pub const NAME: &str = "task.resource";
const LVO_INIT_TASK_HOOKS: i32 = -54;
const LVO_ADD_TASK_HOOK: i32 = -60;
const LVO_RUN_TASK_HOOKS: i32 = -66;
const LVO_ALLOC_SLOT: i32 = -72;
const LVO_FREE_SLOT: i32 = -78;
const LVO_SAVE_STORAGE: i32 = -84;
const LVO_RESTORE_STORAGE: i32 = -90;
const LVO_SET_SLOT: i32 = -96;
const LVO_GET_SLOT: i32 = -102;
const LVO_GET_PARENT_SLOT: i32 = -108;

/// The resource's host-side state. Slot 0 means "none" in the AROS API, so ids start at 1.
#[derive(Default)]
pub struct State {
    /// The base once opened (the resource is created on the first OpenResource of its name).
    pub base: Option<u32>,
    next_slot: i32,
    free: Vec<i32>,
    /// (task, slot) -> value.
    values: BTreeMap<(u32, i32), u32>,
    /// type -> dispatcher registered by InitTaskHooks.
    dispatchers: BTreeMap<u32, u32>,
    /// type -> hooks registered by AddTaskHook.
    hooks: BTreeMap<u32, Vec<u32>>,
}

pub fn library<U: GuestUi>(base: u32) -> LibraryBuilder<U> {
    LibraryBuilder::new(NAME, base)
        .func(LVO_INIT_TASK_HOOKS, "InitTaskHooks", init_task_hooks::<U>)
        .func(LVO_ADD_TASK_HOOK, "AddTaskHook", add_task_hook::<U>)
        .func(LVO_RUN_TASK_HOOKS, "RunTaskHooks", run_task_hooks::<U>)
        .func(LVO_ALLOC_SLOT, "AllocTaskStorageSlot", alloc_slot::<U>)
        .func(LVO_FREE_SLOT, "FreeTaskStorageSlot", free_slot::<U>)
        .func(LVO_SAVE_STORAGE, "SaveTaskStorage", save_storage::<U>)
        .func(LVO_RESTORE_STORAGE, "RestoreTaskStorage", restore_storage::<U>)
        .func(LVO_SET_SLOT, "SetTaskStorageSlot", set_slot::<U>)
        .func(LVO_GET_SLOT, "GetTaskStorageSlot", get_slot::<U>)
        .func(LVO_GET_PARENT_SLOT, "GetParentTaskStorageSlot", get_parent_slot::<U>)
}

/// InitTaskHooks(a0 = dispatcher, d0 = type, d1 = flags) -> TRUE.
fn init_task_hooks<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (disp, ty, flags) = (cpu.areg(0), cpu.dreg(0), cpu.dreg(1));
    a.task_res.dispatchers.insert(ty, disp);
    cpu.set_d0(1);
    Ok(format!("[task.resource] InitTaskHooks type={ty} dispatcher={disp:#x} flags={flags:#x}"))
}

/// AddTaskHook(a0 = hook, d0 = type) -> TRUE.
fn add_task_hook<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (hook, ty) = (cpu.areg(0), cpu.dreg(0));
    a.task_res.hooks.entry(ty).or_default().push(hook);
    cpu.set_d0(1);
    Ok(format!("[task.resource] AddTaskHook type={ty} hook={hook:#x}"))
}

/// RunTaskHooks(a0 = dispatcher, d0 = type) -> TRUE; refuses (by name) when hooks of that type are registered.
fn run_task_hooks<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let ty = cpu.dreg(0);
    let n = a.task_res.hooks.get(&ty).map_or(0, Vec::len);
    if n > 0 { return Err(Refusal(format!("RunTaskHooks type {ty}: {n} registered hook(s) cannot be run yet (hook dispatch is not modelled)"))); }
    cpu.set_d0(1);
    Ok(format!("[task.resource] RunTaskHooks type={ty}: no hooks registered"))
}

/// AllocTaskStorageSlot() -> a slot id >= 1.
fn alloc_slot<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let s = &mut a.task_res;
    let id = s.free.pop().unwrap_or_else(|| { s.next_slot += 1; s.next_slot });
    cpu.set_d0(id as u32);
    Ok(format!("[task.resource] AllocTaskStorageSlot -> {id}"))
}

/// FreeTaskStorageSlot(d0 = slot): forget every task's value and make the id reusable.
fn free_slot<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let id = cpu.dreg(0) as i32;
    let s = &mut a.task_res;
    if id < 1 || id > s.next_slot || s.free.contains(&id) { return Err(Refusal(format!("FreeTaskStorageSlot {id}: not an allocated slot"))); }
    s.values.retain(|&(_, slot), _| slot != id);
    s.free.push(id);
    Ok(format!("[task.resource] FreeTaskStorageSlot {id}"))
}

/// SetTaskStorageSlot(d0 = slot, d1 = value) -> TRUE.
fn set_slot<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (id, v) = (cpu.dreg(0) as i32, cpu.dreg(1));
    let task = a.sched.cur;
    let s = &mut a.task_res;
    if id < 1 || id > s.next_slot || s.free.contains(&id) { return Err(Refusal(format!("SetTaskStorageSlot {id}: not an allocated slot"))); }
    s.values.insert((task, id), v);
    cpu.set_d0(1);
    Ok(format!("[task.resource] SetTaskStorageSlot {id} = {v:#x}"))
}

/// GetTaskStorageSlot(d0 = slot) -> the calling task's value, 0 if never set.
fn get_slot<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let id = cpu.dreg(0) as i32;
    let v = a.task_res.values.get(&(a.sched.cur, id)).copied().unwrap_or(0);
    cpu.set_d0(v);
    Ok(format!("[task.resource] GetTaskStorageSlot {id} -> {v:#x}"))
}

/// GetParentTaskStorageSlot(d0 = slot) -> the PARENT task's value of the slot, 0 if it never set one. The engine starts
/// the program from a host-made "CLI" that owns no task storage, so for the main task the parent's value is 0 (the real
/// call's answer for a parent with no storage); a task created by CreateNewProc has no recorded parent yet, which is
/// refused by name rather than answered 0.
fn get_parent_slot<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let id = cpu.dreg(0) as i32;
    if a.sched.cur != crate::process::PROCESS { return Err(Refusal(format!("GetParentTaskStorageSlot {id}: the parent of task {:#x} is not recorded", a.sched.cur))); }
    cpu.set_d0(0);
    Ok(format!("[task.resource] GetParentTaskStorageSlot {id} -> 0 (the launcher owns no task storage)"))
}

fn save_storage<U: GuestUi>(_: &mut Amiga<U>, _: &mut Cpu) -> Result<String, Refusal> { Err(Refusal("SaveTaskStorage is not modelled".into())) }
fn restore_storage<U: GuestUi>(_: &mut Amiga<U>, _: &mut Cpu) -> Result<String, Refusal> { Err(Refusal("RestoreTaskStorage is not modelled".into())) }
