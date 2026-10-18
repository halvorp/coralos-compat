//! exec.library SignalSemaphores (exec/semaphores.h): Init / Obtain / Release / Attempt (exclusive and shared) and the
//! public list (AddSemaphore / RemSemaphore / FindSemaphore).
//!
//! A semaphore is owned by one task (host-side `Sched::sem_owner`, mirrored into ss_Owner): Obtain by the owner nests, by
//! another task parks the caller until it is released (sched.rs), and the failures are a Release by a task that
//! does not hold it - a named refusal. Ownership and nesting are kept HOST-side and mirrored into the guest struct
//! (ss_NestCount 14, ss_Owner 40; layout: ss_Link Node 0..14, ss_NestCount WORD 14, ss_WaitQueue MinList 16..28,
//! ss_MultipleLink SemaphoreRequest 28..40, ss_Owner 40, ss_QueueCount WORD 44 = 46 bytes; AROS exec/semaphores.h,
//! m68k ABI).
use alloc::string::String;
use alloc::format;
use crate::{machine::Amiga, Refusal};
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

pub const LVO_INIT_SEMAPHORE: i32 = -558;
pub const LVO_OBTAIN_SEMAPHORE: i32 = -564;
pub const LVO_RELEASE_SEMAPHORE: i32 = -570;
pub const LVO_ATTEMPT_SEMAPHORE: i32 = -576;
pub const LVO_ADD_SEMAPHORE: i32 = -600;
pub const LVO_REM_SEMAPHORE: i32 = -606;
pub const LVO_FIND_SEMAPHORE: i32 = -594;
pub const LVO_OBTAIN_SEMAPHORE_SHARED: i32 = -678;
pub const LVO_ATTEMPT_SEMAPHORE_SHARED: i32 = -720;

const SS_NEST: u32 = 14;
const SS_WAITQUEUE: u32 = 16;
const SS_OWNER: u32 = 40;
const SS_QUEUECOUNT: u32 = 44;
const SS_SIZE: u32 = 46;
const LN_TYPE: u32 = 8;
const LN_NAME: u32 = 10;
const NT_SIGNALSEM: u8 = 15;

fn m(what: &'static str) -> impl Fn(retro_guest_mem::MemError) -> Refusal { Refusal::mem(what) }

fn mirror<U: GuestUi>(a: &mut Amiga<U>, ss: u32, nest: u16, owner: u32) -> Result<(), Refusal> {
    a.mem.write_u16(ss + SS_NEST, nest).map_err(m("ss_NestCount"))?;
    a.mem.write_u32(ss + SS_OWNER, if nest == 0 { 0 } else { owner }).map_err(m("ss_Owner"))
}

/// Initialise the semaphore at `ss` as exec's InitSemaphore does (the engine's own structs, e.g. the DosInfo locks, use it too).
pub(crate) fn init_struct<U: GuestUi>(a: &mut Amiga<U>, ss: u32) -> Result<(), Refusal> {
    let name = a.mem.read_u32(ss + LN_NAME).map_err(m("ln_Name"))?; // InitSemaphore keeps the caller's name
    for i in 0..SS_SIZE { a.mem.write_u8(ss + i, 0).map_err(m("SignalSemaphore"))?; }
    a.mem.write_u8(ss + LN_TYPE, NT_SIGNALSEM).map_err(m("ln_Type"))?;
    a.mem.write_u32(ss + LN_NAME, name).map_err(m("ln_Name"))?;
    a.mem.write_u32(ss + SS_WAITQUEUE, ss + SS_WAITQUEUE + 4).map_err(m("ss_WaitQueue"))?;
    a.mem.write_u32(ss + SS_WAITQUEUE + 8, ss + SS_WAITQUEUE).map_err(m("ss_WaitQueue"))?;
    a.mem.write_u16(ss + SS_QUEUECOUNT, 0xFFFF).map_err(m("ss_QueueCount"))?; // -1: free, as exec initialises it
    a.semaphores.insert(ss, 0);
    Ok(())
}

/// InitSemaphore(a0 = semaphore): zero, NT_SIGNALSEM, an empty wait queue (a MinList: mlh_Head = &mlh_Tail).
pub fn init<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let ss = cpu.areg(0);
    init_struct(a, ss)?;
    Ok(format!("[exec] InitSemaphore {ss:#x}"))
}

/// `task` takes (one more level of) `ss` if it is free or already its own. Ok(false) = another task holds it.
fn take<U: GuestUi>(a: &mut Amiga<U>, task: u32, ss: u32, what: &str) -> Result<bool, Refusal> {
    let n = a.semaphores.get(&ss).copied().ok_or_else(|| Refusal(format!("{what}: {ss:#x} was never InitSemaphore'd")))?;
    if n != 0 && a.sched.sem_owner.get(&ss) != Some(&task) { return Ok(false); }
    let n = n.checked_add(1).ok_or_else(|| Refusal(format!("{what}: {ss:#x} nested past 65535")))?;
    a.semaphores.insert(ss, n);
    a.sched.sem_owner.insert(ss, task);
    mirror(a, ss, n, task)?;
    Ok(true)
}

/// The scheduler's check for a task parked in ObtainSemaphore (`Waiting::Sem`): take it when it is free.
pub(crate) fn try_take<U: GuestUi>(a: &mut Amiga<U>, task: u32, ss: u32, commit: bool) -> Result<Option<u32>, Refusal> {
    let n = a.semaphores.get(&ss).copied().unwrap_or(0);
    if n != 0 && a.sched.sem_owner.get(&ss) != Some(&task) { return Ok(None); }
    if commit { take(a, task, ss, "ObtainSemaphore")?; }
    Ok(Some(0))
}

/// ObtainSemaphore / ObtainSemaphoreShared. A semaphore another task holds parks the caller until it is released.
/// SHARED obtains are treated as exclusive (a second shared reader waits for the first): less concurrent than exec,
/// never wrong.
fn obtain_one<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, what: &str) -> Result<String, Refusal> {
    let (ss, cur) = (cpu.areg(0), a.sched.cur);
    if take(a, cur, ss, what)? {
        let n = a.semaphores[&ss];
        return Ok(format!("[exec] {what} {ss:#x} (nest {n})"));
    }
    a.park(crate::exec_wait::Waiting::Sem(ss));
    cpu.block();
    Ok(format!("[exec] {what} {ss:#x} (held by task {:#x}; blocked)", a.sched.sem_owner.get(&ss).copied().unwrap_or(0)))
}

pub fn obtain<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> { obtain_one(a, cpu, "ObtainSemaphore") }
pub fn obtain_shared<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> { obtain_one(a, cpu, "ObtainSemaphoreShared") }

/// AttemptSemaphore(a0) -> TRUE if taken, FALSE if another task holds it.
fn attempt_one<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, what: &str) -> Result<String, Refusal> {
    let (ss, cur) = (cpu.areg(0), a.sched.cur);
    let got = take(a, cur, ss, what)?;
    cpu.set_d0(got as u32);
    Ok(format!("[exec] {what} {ss:#x} -> {got}"))
}
pub fn attempt<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> { attempt_one(a, cpu, "AttemptSemaphore") }
pub fn attempt_shared<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> { attempt_one(a, cpu, "AttemptSemaphoreShared") }

/// ReleaseSemaphore(a0): one level of the holder's nesting; releasing one that is not held, or that another task holds,
/// is the guest's bug, named. A task parked on it wakes (and preempts, if it outranks the releaser).
pub fn release<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (ss, cur) = (cpu.areg(0), a.sched.cur);
    let n = a.semaphores.get(&ss).copied().ok_or_else(|| Refusal(format!("ReleaseSemaphore: {ss:#x} was never InitSemaphore'd")))?;
    let n = n.checked_sub(1).ok_or_else(|| Refusal(format!("ReleaseSemaphore: {ss:#x} is not held")))?;
    if let Some(&o) = a.sched.sem_owner.get(&ss) { if o != cur { return Err(Refusal(format!("ReleaseSemaphore: {ss:#x} is held by task {o:#x}, not by {cur:#x}"))); } }
    a.semaphores.insert(ss, n);
    if n == 0 { a.sched.sem_owner.remove(&ss); }
    mirror(a, ss, n, cur)?;
    if n == 0 { a.wake_all()?; }
    Ok(format!("[exec] ReleaseSemaphore {ss:#x} (nest {n})"))
}

/// AddSemaphore(a1): InitSemaphore + publish it under its ln_Name.
pub fn add<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let ss = cpu.areg(1);
    cpu.set_areg(0, ss);
    init(a, cpu)?;
    let np = a.mem.read_u32(ss + LN_NAME).map_err(m("ln_Name"))?;
    let name = if np == 0 { String::new() } else { a.mem.read_cstr(np).map_err(m("ln_Name"))? };
    a.public_semaphores.insert(name.clone(), ss);
    Ok(format!("[exec] AddSemaphore {ss:#x} \"{name}\""))
}

pub fn rem<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let ss = cpu.areg(1);
    let before = a.public_semaphores.len();
    a.public_semaphores.retain(|_, &mut v| v != ss);
    if a.public_semaphores.len() == before { return Err(Refusal(format!("RemSemaphore: {ss:#x} is not a public semaphore"))); }
    Ok(format!("[exec] RemSemaphore {ss:#x}"))
}

/// FindSemaphore(a1 = name) -> the public semaphore of that name, or NULL (e.g. amimon looking for a running amiagent).
pub fn find<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let name = a.mem.read_cstr(cpu.areg(1)).map_err(m("FindSemaphore.name"))?;
    let ss = a.public_semaphores.get(&name).copied().unwrap_or(0);
    cpu.set_d0(ss);
    Ok(format!("[exec] FindSemaphore \"{name}\" -> {ss:#x}"))
}
