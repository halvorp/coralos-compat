//! exec.library task management: AddTask / RemTask / FindTask / SetTaskPri, Forbid / Permit / Disable / Enable, and
//! the trap a task's entry function returns into.
//!
//! Semantics follow AROS rom/exec (addtask.c + newaddtask.c, remtask.c, settaskpri.c, findtask.c; facts only):
//! AddTask fills in what the caller left zero (node type NT_TASK, tc_SigAlloc = the system signals, an empty
//! tc_MemEntry), starts the task at `initialPC` with every register 0 and `finalPC` (or the default finalizer, which
//! is RemTask(0)) as its return address, and makes it ready; RemTask(0) removes the caller, RemTask(t) another task
//! and frees the memory in its tc_MemEntry list; FindTask(name) looks at the running task first, then the others.
//! Struct offsets: m68k-linux-gnu-gcc -m68020 -mno-align-int over the AROS exec/tasks.h field lists (2026-10-18):
//! Task is 92 bytes; ln_Name 10, tc_SigAlloc 18, tc_SPReg 54, tc_SPLower 58, tc_SPUpper 62, tc_MemEntry 74 (a List).
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use crate::libtable::LibraryBuilder;
use crate::machine::Amiga;
use crate::sched::LN_PRI;
use crate::{execlists, Refusal};
use heirloom_ui_abi::GuestUi;
use heirloom_cpu_68k::Cpu;

/// Where a task's entry function returns to when it has no finalPC: RemTask(0).
pub const TASK_END: u32 = 0x530;
const LN_NAME: u32 = 10;
const LN_TYPE: u32 = 8;
const NT_TASK: u8 = 1;
const TC_SIG_ALLOC: u32 = 18;
const TC_SP_REG: u32 = 54;
const TC_SP_LOWER: u32 = 58;
const TC_SP_UPPER: u32 = 62;
const TC_MEM_ENTRY: u32 = 74;
const TC_ID_NEST: u32 = 16;
const TC_TD_NEST: u32 = 17;
/// exec/memory.h: struct MemList { Node ml_Node (14); UWORD ml_NumEntries (14); struct MemEntry ml_ME[] (16, 8 each) }.
const ML_NUM_ENTRIES: u32 = 14;
const ML_ME: u32 = 16;
/// Signals 0..15 belong to exec (tasks' default tc_SigAlloc).
pub const SYSTEM_SIGNALS: u32 = 0x0000_FFFF;

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.named("AddTask", add_task::<U>).named("RemTask", rem_task::<U>).named("SetTaskPri", set_task_pri::<U>)
        .named("Forbid", forbid::<U>).named("Permit", permit::<U>).named("Disable", disable::<U>).named("Enable", enable::<U>)
}

fn m(what: &'static str) -> impl Fn(heirloom_mem_flat32::MemError) -> Refusal { Refusal::mem(what) }

/// The task's name as the guest sees it (ln_Name), or "" when it has none.
fn name_of<U: GuestUi>(a: &Amiga<U>, t: u32) -> String {
    match a.mem.read_u32(t + LN_NAME) { Ok(p) if p != 0 => a.mem.read_cstr(p).unwrap_or_default(), _ => String::new() }
}

/// FindTask(a1 = name): NULL = the running task. A name is looked for in the running task first, then the others (exec's
/// order: ThisTask, TaskReady, TaskWait).
pub fn find_task<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let name = cpu.areg(1);
    if name == 0 {
        cpu.set_d0(a.sched.cur);
        return Ok(format!("[exec] FindTask NULL -> {:#x} (this task)", a.sched.cur));
    }
    let n = a.mem.read_cstr(name).map_err(m("FindTask.name"))?;
    let mut order: Vec<u32> = alloc::vec![a.sched.cur];
    order.extend(a.sched.tasks.keys().copied().filter(|&t| t != a.sched.cur));
    let found = order.into_iter().filter(|t| a.sched.tasks.contains_key(t)).find(|&t| name_of(a, t) == n).unwrap_or(0);
    cpu.set_d0(found);
    Ok(format!("[exec] FindTask \"{n}\" -> {found:#x}"))
}

/// AddTask(a1 = task, a2 = initialPC, a3 = finalPC) -> the task, or NULL (refused by name).
fn add_task<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (t, pc, fin) = (cpu.areg(1), cpu.areg(2), cpu.areg(3));
    if t == 0 || pc == 0 { return Err(Refusal(format!("AddTask: task {t:#x} / initialPC {pc:#x} must not be NULL"))); }
    if a.sched.tasks.contains_key(&t) { return Err(Refusal(format!("AddTask: {t:#x} is already a task"))); }
    let (lower, upper) = (a.mem.read_u32(t + TC_SP_LOWER).map_err(m("tc_SPLower"))?, a.mem.read_u32(t + TC_SP_UPPER).map_err(m("tc_SPUpper"))?);
    if lower == 0 || upper <= lower { return Err(Refusal(format!("AddTask: tc_SPLower {lower:#x} / tc_SPUpper {upper:#x} do not describe a stack"))); }
    if a.mem.read_u8(t + LN_TYPE).map_err(m("ln_Type"))? == 0 { a.mem.write_u8(t + LN_TYPE, NT_TASK).map_err(m("ln_Type"))?; }
    if a.mem.read_u32(t + TC_SIG_ALLOC).map_err(m("tc_SigAlloc"))? == 0 { a.mem.write_u32(t + TC_SIG_ALLOC, SYSTEM_SIGNALS).map_err(m("tc_SigAlloc"))?; }
    if a.mem.read_u32(t + TC_MEM_ENTRY).map_err(m("tc_MemEntry"))? == 0 { execlists::new_list(&mut a.mem, t + TC_MEM_ENTRY, 0)?; }
    let sp_reg = a.mem.read_u32(t + TC_SP_REG).map_err(m("tc_SPReg"))?;
    let top = if sp_reg > lower && sp_reg <= upper { sp_reg } else { upper } & !3;
    let sp = top.checked_sub(4).filter(|&s| s >= lower).ok_or_else(|| Refusal("AddTask: the stack is too small for a return address".into()))?;
    a.mem.write_u32(sp, if fin == 0 { TASK_END } else { fin }).map_err(m("task stack"))?;
    a.mem.write_u32(t + TC_SP_REG, sp).map_err(m("tc_SPReg"))?;
    a.mem.write_u8(t + TC_ID_NEST, 0xFF).map_err(m("tc_IDNestCnt"))?;
    a.mem.write_u8(t + TC_TD_NEST, 0xFF).map_err(m("tc_TDNestCnt"))?;
    let name = match name_of(a, t) { n if n.is_empty() => "bad task".into(), n => n };
    a.add_sched_task(t, name.clone(), false, crate::sched::Start { pc, sp, regs: Vec::new() }, None);
    a.sync_task_lists()?;
    cpu.set_d0(t);
    Ok(format!("[exec] AddTask {t:#x} \"{name}\" pc={pc:#x} sp={sp:#x} pri={}", a.task_pri(t)))
}

/// RemTask(a1 = task or NULL): remove it (NULL = the caller) and free its tc_MemEntry memory. An address that is not a
/// task (never added, or already gone) is a named refusal.
fn rem_task<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let cur = a.sched.cur;
    let t = if cpu.areg(1) == 0 { cur } else { cpu.areg(1) };
    let Some(task) = a.sched.tasks.get(&t) else { return Err(Refusal(format!("RemTask: {t:#x} is not a task (never added, or already removed)"))) };
    if task.main {
        if t != cur { return Err(Refusal("RemTask: the main program's task can only remove itself".into())); }
        cpu.halt();
        return Ok("[exec] RemTask(0) in the main program: the program ends".into());
    }
    if t == cur {
        a.sched.tasks.get_mut(&t).expect("task").dead = true;
        cpu.block();
        return Ok(format!("[exec] RemTask(0): task {t:#x} removes itself"));
    }
    let line = a.reap(t, "RemTask")?;
    Ok(format!("[exec] RemTask {t:#x}: {line}"))
}

/// Free the memory in `t`'s tc_MemEntry list (exec's RemTask does; the task struct is usually one of the entries).
pub fn free_mem_entries<U: GuestUi>(a: &mut Amiga<U>, t: u32) -> Result<(), Refusal> {
    let list = t + TC_MEM_ENTRY;
    let mut blocks = Vec::new();
    let mut node = a.mem.read_u32(list).map_err(m("tc_MemEntry"))?;
    let mut guard = 0;
    while node != 0 && node != list + 4 && guard < 1024 {
        let succ = a.mem.read_u32(node).map_err(m("MemList.ln_Succ"))?;
        let n = a.mem.read_u16(node + ML_NUM_ENTRIES).map_err(m("ml_NumEntries"))? as u32;
        for i in 0..n {
            let e = node + ML_ME + 8 * i;
            blocks.push((a.mem.read_u32(e).map_err(m("me_Addr"))?, a.mem.read_u32(e + 4).map_err(m("me_Length"))?));
        }
        node = succ;
        guard += 1;
    }
    for (addr, len) in blocks {
        if let Err(e) = crate::fs::free_block(a, addr, len) { a.log.push(format!("[sched] task {t:#x}: tc_MemEntry block {addr:#x}+{len} not freed: {e}")); }
    }
    Ok(())
}

/// SetTaskPri(a1 = task, d0 = priority) -> the old priority (sign-extended).
fn set_task_pri<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (t, pri) = (cpu.areg(1), cpu.dreg(0) as i8);
    if !a.sched.tasks.contains_key(&t) { return Err(Refusal(format!("SetTaskPri: {t:#x} is not a task"))); }
    let old = a.task_pri(t);
    a.mem.write_u8(t + LN_PRI, pri as u8).map_err(m("ln_Pri"))?;
    a.sync_task_lists()?;
    cpu.set_d0(old as i32 as u32);
    Ok(format!("[exec] SetTaskPri {t:#x} {old} -> {pri}"))
}

fn nest<U: GuestUi>(a: &mut Amiga<U>) -> &mut crate::sched::Task<U> {
    let cur = a.sched.cur;
    a.sched.tasks.get_mut(&cur).expect("the running task is in the table")
}

fn mirror_nest<U: GuestUi>(a: &mut Amiga<U>) -> Result<(), Refusal> {
    let (f, d) = { let t = nest(a); (t.forbid, t.disable) };
    a.mem.write_u8(crate::machine::EXEC_BASE + 295, (f as i32 - 1) as u8).map_err(m("ExecBase.TDNestCnt"))?;
    a.mem.write_u8(crate::machine::EXEC_BASE + 294, (d as i32 - 1) as u8).map_err(m("ExecBase.IDNestCnt"))
}

/// Forbid(): no task switch until the matching Permit (counted per task). A task that Waits inside it is switched away
/// anyway, as in exec.
fn forbid<U: GuestUi>(a: &mut Amiga<U>, _: &mut Cpu) -> Result<String, Refusal> {
    nest(a).forbid += 1;
    mirror_nest(a)?;
    Ok(format!("[exec] Forbid (depth {})", nest(a).forbid))
}

fn permit<U: GuestUi>(a: &mut Amiga<U>, _: &mut Cpu) -> Result<String, Refusal> {
    let t = nest(a);
    t.forbid = t.forbid.checked_sub(1).ok_or_else(|| Refusal("Permit without Forbid".into()))?;
    mirror_nest(a)?;
    Ok(format!("[exec] Permit (depth {})", nest(a).forbid))
}

/// Disable()/Enable(): interrupts are not modelled, so they stop task switching exactly as Forbid()/Permit() do (a
/// separate count, as exec keeps IDNestCnt apart from TDNestCnt).
fn disable<U: GuestUi>(a: &mut Amiga<U>, _: &mut Cpu) -> Result<String, Refusal> {
    nest(a).disable += 1;
    mirror_nest(a)?;
    Ok(format!("[exec] Disable (depth {}; interrupts are not modelled, acts as Forbid)", nest(a).disable))
}

fn enable<U: GuestUi>(a: &mut Amiga<U>, _: &mut Cpu) -> Result<String, Refusal> {
    let t = nest(a);
    t.disable = t.disable.checked_sub(1).ok_or_else(|| Refusal("Enable without Disable".into()))?;
    mirror_nest(a)?;
    Ok(format!("[exec] Enable (depth {})", nest(a).disable))
}

/// The trap a task's entry function returns into (default finalizer): the task ends. A dos Process with an exit
/// code function runs it first (dos_newproc.rs).
pub fn task_end<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let cur = a.sched.cur;
    let rc = cpu.dreg(0);
    if a.sched.tasks.get(&cur).map_or(false, |t| t.main) { cpu.halt(); return Ok("[exec] the main task reached the task finalizer".into()); }
    if let Some(line) = crate::dos_newproc::process_end(a, cpu, rc)? { return Ok(line); }
    nest(a).dead = true;
    cpu.block();
    Ok(format!("[exec] task {cur:#x} \"{}\" returned {rc}; removed", nest(a).name))
}

/// Install the finalizer trap.
pub fn install<U: GuestUi>(a: &mut Amiga<U>) -> Result<(), Refusal> { a.install_slot(TASK_END, "exec.library", "TaskEnd", task_end::<U>) }

