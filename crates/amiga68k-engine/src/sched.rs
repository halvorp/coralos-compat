//! The scheduler: several guest tasks in one engine, run one at a time by the host's `Amiga::step` (run.rs).
//!
//! MODEL (faithful to exec's rules, in VIRTUAL time = instructions executed):
//! - every task has a real `struct Task`/`Process` in guest memory and its own `CpuContext`, stack and host-side
//!   bookkeeping ([`Task`]). `ExecBase.ThisTask` names the running one; `ExecBase.TaskReady` / `TaskWait` are real
//!   lists the guest can walk. The HOST table is authoritative: the lists are rebuilt from it at every task switch
//!   (and when a task is added or removed), so a guest that scribbles on them cannot change who runs.
//! - the highest-priority READY task runs (`tc_Node.ln_Pri`, read from guest memory so SetTaskPri and direct writes
//!   agree). Equal priorities round-robin on a quantum of [`QUANTUM_INSNS`] instructions; a task that calls
//!   Wait/WaitPort/WaitIO/Delay/ObtainSemaphore with nothing to wait for moves to TaskWait; a Signal/PutMsg that
//!   satisfies it moves it back, and if it outranks the running task the slice ends at the end of that library call
//!   (see `Amiga::preempt_check`). Forbid()/Disable() stop both the quantum switch and preemption, per task
//!   (`tc_TDNestCnt`); a task that Waits inside Forbid is switched away anyway and gets its nesting back, as exec does.
//!   Disable()/Enable() are treated like Forbid()/Permit(): interrupts are not modelled.
//! - `step` runs tasks until every task is blocked (`Step::Blocked`), the budget is spent, or the MAIN program task
//!   exits (`Step::Exited`). AmigaOS would leave the other tasks running; here they are reported and dropped, named
//!   in the log and in [`Sched::dropped`].
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use crate::exec_wait::Waiting;
use crate::guest_call::Frame;
use crate::machine::{Amiga, EXEC_BASE};
use crate::{execlists, Refusal, Step};
use guestui_abi::GuestUi;
use retro_m68k::{CpuContext, RunStats, REG_D0};

/// Instructions a task may run before an equal-priority peer gets the CPU.
pub const QUANTUM_INSNS: u64 = 20_000;

/// Offsets in `struct ExecBase` (m68k gcc -mno-align-int over AROS exec/execbase.h, 2026-10-18): ThisTask 276,
/// IDNestCnt 294, TDNestCnt 295, TaskReady 406, TaskWait 420. And in `struct Task` (exec/tasks.h): ln_Pri 9,
/// tc_State 15, tc_IDNestCnt 16, tc_TDNestCnt 17.
pub const EXECBASE_THIS_TASK: u32 = 276;
const EXECBASE_ID_NEST: u32 = 294;
const EXECBASE_TD_NEST: u32 = 295;
const EXECBASE_TASK_READY: u32 = 406;
const EXECBASE_TASK_WAIT: u32 = 420;
pub const LN_PRI: u32 = 9;
const TC_STATE: u32 = 15;
const TS_RUN: u8 = 2;
const TS_READY: u8 = 3;
const TS_WAIT: u8 = 4;
const NT_TASK_LIST: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState { Run, Ready, Wait }

/// How a task that never ran begins: registers are created at its first dispatch (Musashi's reset cannot run inside a
/// library call).
pub struct Start { pub pc: u32, pub sp: u32, pub regs: Vec<(u32, u32)> }

pub struct Task<U: GuestUi> {
    pub name: String,
    pub state: RunState,
    /// What the task waits for; also set on a READY task that was woken and not yet resumed (the wait is completed,
    /// and D0 set, at its dispatch).
    pub waiting: Option<Waiting>,
    pub ctx: Option<CpuContext>,
    pub start: Option<Start>,
    pub forbid: u32,
    pub disable: u32,
    pub seq: u64,
    /// The task ended or was removed while it was running; reaped when its slice returns.
    pub dead: bool,
    /// Host-started guest calls in flight (guest_call.rs) and local variables (vars.rs): swapped in while it runs.
    pub calls: Vec<Frame<U>>,
    pub vars: crate::vars::Vars,
    /// Bookkeeping of a process dos created (dos_newproc.rs); None for the main program and for AddTask tasks.
    pub own: Option<crate::dos_newproc::ProcOwn>,
    pub main: bool,
    /// The task that was running when this one was added (exec's `GET_THIS_TASK` in newaddtask.c:189 -> `et_Parent`, exec_util.c:169); None
    /// for the main program, which the host-made launcher starts.
    pub parent: Option<u32>,
}

pub struct Sched<U: GuestUi> {
    pub tasks: BTreeMap<u32, Task<U>>,
    /// The running task (`ExecBase.ThisTask`); the main program's Process while idle.
    pub cur: u32,
    next_seq: u64,
    /// SignalSemaphore -> the task holding it (semaphores.rs).
    pub sem_owner: BTreeMap<u32, u32>,
    /// SignalSemaphore -> the tasks holding it SHARED and how many times each.
    pub sem_shared: BTreeMap<u32, BTreeMap<u32, u16>>,
    /// MUI Application object -> the task that created it (its ReturnID signal goes there).
    pub app_task: BTreeMap<u32, u32>,
    /// Low-memory handlers (exec_misc.rs), in the order added.
    pub mem_handlers: Vec<u32>,
    /// Tasks dropped when the main program exited.
    pub dropped: Vec<String>,
}

impl<U: GuestUi> Default for Sched<U> {
    fn default() -> Self { Sched { tasks: BTreeMap::new(), cur: crate::process::PROCESS, next_seq: 1, sem_owner: BTreeMap::new(), sem_shared: BTreeMap::new(), app_task: BTreeMap::new(), mem_handlers: Vec::new(), dropped: Vec::new() } }
}

impl<U: GuestUi> Sched<U> {
    pub fn next_seq(&mut self) -> u64 { self.next_seq += 1; self.next_seq }
    pub fn task(&self, t: u32) -> Option<&Task<U>> { self.tasks.get(&t) }
}

/// What a slice ended in, for `step`.
enum Outcome { Continue, Exited, Fault(String) }

impl<U: GuestUi> Amiga<U> {
    /// The running task (ThisTask).
    pub fn this_task(&self) -> u32 { self.sched.cur }

    /// Register a new task that is ready to run at `start`.
    pub fn add_sched_task(&mut self, addr: u32, name: String, main: bool, start: Start, own: Option<crate::dos_newproc::ProcOwn>) {
        let seq = self.sched.next_seq();
        let parent = if main { None } else { Some(self.sched.cur) };
        self.sched.tasks.insert(addr, Task { name, state: RunState::Ready, waiting: None, ctx: None, start: Some(start), forbid: 0, disable: 0, seq, dead: false, calls: Vec::new(), vars: Default::default(), own, main, parent });
    }

    /// Is `t` a task? Before any program started (a bare machine in a unit test) the table is empty and every address
    /// passes, so host-side tests can signal the Process they set up by hand.
    pub fn known_task(&self, t: u32) -> bool { self.sched.tasks.is_empty() || self.sched.tasks.contains_key(&t) }

    pub fn task_pri(&self, t: u32) -> i8 { self.mem.read_u8(t + LN_PRI).unwrap_or(0) as i8 }

    /// Park the running task on `w` (the caller then ends the slice with `cpu.block()`).
    pub fn park(&mut self, w: Waiting) {
        let cur = self.sched.cur;
        if let Some(t) = self.sched.tasks.get_mut(&cur) { t.waiting = Some(w); t.state = RunState::Wait; }
    }

    /// A Wait-ing task whose condition now holds becomes ready (exec's Signal moving it from TaskWait to TaskReady).
    pub fn wake(&mut self, task: u32) -> Result<(), Refusal> {
        let Some(w) = self.sched.tasks.get(&task).and_then(|t| if t.state == RunState::Wait { t.waiting } else { None }) else { return Ok(()) };
        if self.satisfy(task, w, false)?.is_some() || crate::boopsi_input::wakes(self, task) {
            let seq = self.sched.next_seq();
            let t = self.sched.tasks.get_mut(&task).expect("task");
            t.state = RunState::Ready;
            t.seq = seq;
        }
        Ok(())
    }

    /// Wake every task whose wait holds (after the host advanced the clock, delivered an event, ...).
    pub fn wake_all(&mut self) -> Result<(), Refusal> {
        let ids: Vec<u32> = self.sched.tasks.iter().filter(|(_, t)| t.state == RunState::Wait).map(|(&a, _)| a).collect();
        for t in ids { self.wake(t)?; }
        Ok(())
    }

    /// The task to run next: highest priority, then the one that has waited longest.
    fn pick(&self) -> Option<u32> {
        self.sched.tasks.iter().filter(|(_, t)| t.state == RunState::Ready).max_by_key(|(&a, t)| (self.task_pri(a), core::cmp::Reverse(t.seq))).map(|(&a, _)| a)
    }

    /// After a library call: end the running task's slice if a READY task outranks it and switching is allowed.
    pub(crate) fn preempt_check(&mut self, cpu: &mut retro_m68k::Cpu) {
        if self.sched.tasks.len() < 2 { return; }
        let cur = self.sched.cur;
        let Some(c) = self.sched.tasks.get(&cur) else { return };
        if c.state != RunState::Run || c.forbid > 0 || c.disable > 0 { return; }
        let mine = self.task_pri(cur);
        if self.sched.tasks.iter().any(|(&a, t)| a != cur && t.state == RunState::Ready && self.task_pri(a) > mine) { cpu.block(); }
    }

    /// Rebuild ExecBase.TaskReady / TaskWait from the host table (the running task is on neither, as in exec).
    pub fn sync_task_lists(&mut self) -> Result<(), Refusal> {
        let (ready, wait) = (EXEC_BASE + EXECBASE_TASK_READY, EXEC_BASE + EXECBASE_TASK_WAIT);
        execlists::new_list(&mut self.mem, ready, NT_TASK_LIST)?;
        execlists::new_list(&mut self.mem, wait, NT_TASK_LIST)?;
        let mut order: Vec<(u32, RunState, i8, u64)> = self.sched.tasks.iter().filter(|(&a, _)| a != self.sched.cur)
            .map(|(&a, t)| (a, t.state, self.task_pri(a), t.seq)).collect();
        order.sort_by_key(|&(_, _, pri, seq)| (core::cmp::Reverse(pri), seq));
        for (a, st, _, _) in order {
            let (list, ts) = if st == RunState::Ready { (ready, TS_READY) } else { (wait, TS_WAIT) };
            execlists::add_tail(&mut self.mem, list, a)?;
            self.mem.write_u8(a + TC_STATE, ts).map_err(Refusal::mem("tc_State"))?;
        }
        Ok(())
    }

    /// Remove `t` from the table and release what it owned. Ok(name) for the log.
    pub fn reap(&mut self, t: u32, why: &str) -> Result<String, Refusal> {
        let Some(task) = self.sched.tasks.remove(&t) else { return Err(Refusal(format!("{t:#x} is not a task"))) };
        let line = format!("[sched] task {t:#x} \"{}\" ended ({why})", task.name);
        match task.own {
            Some(own) => crate::dos_newproc::teardown(self, t, own, task.vars)?,
            None if !task.main => crate::exec_task::free_mem_entries(self, t)?,
            None => {}
        }
        self.sync_task_lists()?;
        self.log.push(line.clone());
        Ok(line)
    }

    /// Run `t` for at most `slice` instructions.
    fn run_slice(&mut self, t: u32, slice: u64, tot: &mut RunStats) -> Outcome {
        // Complete a wait that was satisfied before this dispatch (or find it no longer is: back to TaskWait).
        let mut d0 = None;
        crate::boopsi_input::redirect(self, t); // pointer events for a gadget of this task's window run before its wait resumes
        if let Some(w) = self.sched.tasks[&t].waiting {
            match self.satisfy(t, w, true) {
                Ok(None) => { self.sched.tasks.get_mut(&t).expect("task").state = RunState::Wait; return Outcome::Continue; }
                Ok(Some(v)) => { d0 = Some(v); self.sched.tasks.get_mut(&t).expect("task").waiting = None; }
                Err(Refusal(e)) => return Outcome::Fault(e),
            }
        }
        let start = self.sched.tasks.get_mut(&t).expect("task").start.take();
        let mut ctx = match start {
            Some(s) => retro_m68k::start_with(self, s.pc, s.sp, &s.regs),
            None => self.sched.tasks.get_mut(&t).expect("task").ctx.take().expect("a task that is not running has a context"),
        };
        if let Some(v) = d0 { ctx.set_reg(REG_D0, v); }
        // Switch in: ThisTask, nest counts, the per-task host state.
        self.sched.cur = t;
        let k = self.sched.tasks.get_mut(&t).expect("task");
        k.state = RunState::Run;
        core::mem::swap(&mut self.calls, &mut k.calls);
        core::mem::swap(&mut self.fs.vars, &mut k.vars);
        let (fb, dis) = (k.forbid, k.disable);
        let sw = (|| -> Result<(), Refusal> {
            self.mem.write_u32(EXEC_BASE + EXECBASE_THIS_TASK, t).map_err(Refusal::mem("ExecBase.ThisTask"))?;
            self.mem.write_u8(EXEC_BASE + EXECBASE_TD_NEST, (fb as i32 - 1) as u8).map_err(Refusal::mem("ExecBase.TDNestCnt"))?;
            self.mem.write_u8(EXEC_BASE + EXECBASE_ID_NEST, (dis as i32 - 1) as u8).map_err(Refusal::mem("ExecBase.IDNestCnt"))?;
            self.mem.write_u8(t + TC_STATE, TS_RUN).map_err(Refusal::mem("tc_State"))?;
            self.sync_task_lists()
        })();
        if let Err(Refusal(e)) = sw { return Outcome::Fault(e); }
        let stats = retro_m68k::resume(self, &mut ctx, slice);
        crate::graphics::content::flush(self);
        tot.insns += stats.insns;
        tot.bus_faults += stats.bus_faults;
        // Switch out.
        let k = self.sched.tasks.get_mut(&t).expect("the running task stays in the table");
        core::mem::swap(&mut self.calls, &mut k.calls);
        core::mem::swap(&mut self.fs.vars, &mut k.vars);
        k.ctx = Some(ctx);
        if let Some(pc) = stats.stalled_at {
            return Outcome::Fault(format!("task \"{}\": the 68k stopped making progress at pc={pc:#x} (STOP, or halted after a double fault)", k.name));
        }
        let (dead, main, waiting) = (k.dead, k.main, k.waiting.is_some());
        if stats.halted {
            if main || self.stop_on_unimplemented {
                if !main { let n = self.sched.tasks[&t].name.clone(); self.log.push(format!("[sched] task {t:#x} \"{n}\" halted the run (census: an unimplemented call)")); }
                tot.halted = true;
                return Outcome::Exited;
            }
            return match self.reap(t, "halted by a refusal") { Ok(_) => Outcome::Continue, Err(Refusal(e)) => Outcome::Fault(e) };
        }
        if dead {
            return match self.reap(t, "returned or removed itself") { Ok(_) => Outcome::Continue, Err(Refusal(e)) => Outcome::Fault(e) };
        }
        let k = self.sched.tasks.get_mut(&t).expect("task");
        if stats.blocked && waiting {
            k.state = RunState::Wait;
        } else if stats.blocked {
            k.state = RunState::Ready; // preempted by a higher-priority task: keeps its place in the queue
        } else {
            // Quantum spent: behind its equal-priority peers, unless switching is forbidden.
            k.state = RunState::Ready;
            if k.forbid == 0 && k.disable == 0 { let s = self.sched.next_seq(); self.sched.tasks.get_mut(&t).expect("task").seq = s; }
        }
        Outcome::Continue
    }

    /// Run at most `budget` instructions in total over all tasks; see the module docs for when it returns.
    pub fn step(&mut self, budget: u64) -> (Step, RunStats) {
        let mut tot = RunStats { insns: 0, bus_faults: 0, halted: false, blocked: false, stalled_at: None };
        if self.sched.tasks.is_empty() { tot.halted = true; return (Step::Exited, tot); }
        loop {
            if let Err(Refusal(e)) = self.wake_all() { return (Step::Fault(e), tot); }
            let Some(t) = self.pick() else { tot.blocked = true; return (Step::Blocked, tot) };
            if tot.insns >= budget { return (Step::BudgetSpent, tot); }
            let slice = QUANTUM_INSNS.min(budget - tot.insns);
            match self.run_slice(t, slice, &mut tot) {
                Outcome::Continue => {}
                Outcome::Fault(e) => return (Step::Fault(e), tot),
                Outcome::Exited => {
                    self.drop_all_tasks();
                    return (Step::Exited, tot);
                }
            }
        }
    }

    /// The main program exited (or the machine halted): the other tasks are reported and dropped.
    fn drop_all_tasks(&mut self) {
        let others: Vec<String> = self.sched.tasks.iter().filter(|(_, t)| !t.main).map(|(a, t)| format!("{a:#x} \"{}\"", t.name)).collect();
        if !others.is_empty() {
            let line = format!("[sched] the main program exited; dropping {} task(s): {}", others.len(), others.join(", "));
            self.log.push(line);
        }
        self.sched.dropped = others;
        self.sched.tasks.clear();
    }
}
