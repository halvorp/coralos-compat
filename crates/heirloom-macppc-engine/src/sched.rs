//! The scheduler: any number of saved PowerPC register files (`heirloom_cpu_ppc::Cpu` is a plain value), of which exactly one is the live CPU the run loop
//! drives. Switching is a native's act: the live CPU is cloned into the current task's slot, the target's saved CPU is assigned into the live one,
//! and the callbacks in progress (`Runtime::callbacks`, host work in flight for the interrupted call) travel with their task. Nothing else changes;
//! the native "returns" into the other task. Both the Thread Manager ([`crate::threads`], cooperative: switches only when a task asks) and
//! Multiprocessing Services ([`crate::mp`], tasks that wait on queues and semaphores) are built on this one primitive.
//!
//! IDLE-YIELD: a task that would park the whole machine (`WaitNextEvent` with a sleep and no event: `GlueOutcome::Idle`) while another task is ready
//! yields to it instead, and retries the parked call when it is scheduled again; the machine reports `Step::Blocked` only when no other task can run.
//! No time slicing yet: a task that never yields or waits keeps the CPU (see docs/consults/2026-10-24-macppc-thread-manager-design.md for what
//! preemption would need).
use crate::callback::Frame;
use crate::machine::MacPpc;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_ui_abi::GuestUi;
use heirloom_cpu_ppc::Cpu;

/// The task the application itself runs as (`kApplicationThreadID` = 1 in Threads.h).
pub const MAIN_TASK: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TState { Ready, Stopped, Running, Waiting }

/// What a task is, for the API that created it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind { Main, Thread, Mp }

/// What a Waiting task waits on (Multiprocessing Services) and where its results go when it wakes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitOn { Queue(u32), Semaphore(u32), Region(u32) }
#[derive(Debug, Clone, Copy)]
pub struct Wait { pub on: WaitOn, /// the tick at which the wait times out (None = forever)
    pub deadline: Option<u32>, /// MPWaitOnQueue's three result pointers
    pub out: [u32; 3] }

pub struct Task {
    /// The saved registers while the task is not the live one (`None` for the live task and before its first switch-out).
    pub cpu: Option<Cpu>,
    pub(crate) callbacks: Vec<Frame>,
    pub state: TState,
    pub kind: Kind,
    /// The guest stack block to give back when the task ends.
    pub stack: Option<u32>,
    /// Where a thread's exit value is stored when it ends (`NewThread`'s `threadResult`), or 0.
    pub result_ptr: u32,
    pub wait: Option<Wait>,
    /// A wake-up status delivered to the LIVE task (it has no saved registers to put it in): `block` reads it back.
    pub(crate) woken: Option<i32>,
    /// `SetThreadTerminator`: (TVector, parameter) called with (thread id, parameter) when the thread ends.
    pub(crate) terminator: Option<(u32, u32)>,
    /// `SetThreadSwitcher`: (TVector, parameter) called with (thread id, parameter) as the thread is switched IN / OUT.
    pub(crate) switch_in: Option<(u32, u32)>,
    pub(crate) switch_out: Option<(u32, u32)>,
    /// MPCreateTask's termination notice: (queue, parameter1, parameter2).
    pub notify: Option<(u32, u32, u32)>,
    /// How many glue entries the SAVED `cpu` already has bound (the live CPU always has `rt.bound`): a switch back binds only what was interned since.
    pub(crate) bound: usize,
}

#[derive(Default)]
pub struct Sched {
    pub tasks: BTreeMap<u32, Task>,
    pub current: u32,
    pub next_id: u32,
    /// `ThreadBeginCritical` nesting (cooperative scheduling makes it a count, nothing more).
    pub critical: u32,
    /// `bind_native` calls made by task switches (a test counts them: a switch must not rebind what the task already has).
    pub rebinds: u64,
}

impl Sched {
    /// The next Ready task after `after` in id order, wrapping, other than `after` itself.
    pub fn next_ready(&self, after: u32) -> Option<u32> {
        self.tasks.range(after + 1..).chain(self.tasks.range(..after)).find(|(_, t)| t.state == TState::Ready).map(|(&id, _)| id)
    }
}

impl<U: GuestUi> MacPpc<U> {
    /// The `bind_native` calls task switches have made so far.
    pub fn switch_rebinds(&self) -> u64 { self.rt.sched.rebinds }

    /// The task table exists once anything schedules: task 1 is the application, running.
    pub(crate) fn ensure_sched(&mut self) {
        if self.rt.sched.tasks.is_empty() {
            self.rt.sched.tasks.insert(MAIN_TASK, Task { cpu: None, callbacks: Vec::new(), state: TState::Running, kind: Kind::Main, stack: None, result_ptr: 0, wait: None, woken: None, terminator: None, switch_in: None, switch_out: None, notify: None, bound: 0 });
            self.rt.sched.current = MAIN_TASK;
            self.rt.sched.next_id = MAIN_TASK + 1;
        }
    }

    pub fn current_task(&self) -> u32 { self.rt.sched.current }

    /// Make `cpu` the registers of task `to`. The current task is saved as `cur_state` and resumes at `resume_pc` (with whatever `cpu` holds now), or,
    /// with `None`, it is over: removed, its stack freed. `to` must be a Ready task.
    pub(crate) fn switch_to(&mut self, cpu: &mut Cpu, to: u32, cur_state: Option<TState>, resume_pc: u32) -> Result<(), String> {
        let cur = self.rt.sched.current;
        // the thread's out-switcher runs first, on the leaving thread; the switch itself happens when it returns (callback.rs: SwitchOut)
        if let Some((tv, param)) = self.rt.sched.tasks.get(&cur).and_then(|t| t.switch_out) {
            let frame = Frame { saved: crate::callback::Saved::of(cpu, resume_pc), cont: crate::callback::Continuation::SwitchOut { to, state: cur_state, pc: resume_pc } };
            let sp = cpu.gpr[1];
            self.enter_tvector(cpu, tv, &[cur, param], sp).map_err(|e| format!("thread switcher (out): {e}"))?;
            self.rt.callbacks.push(frame);
            return Ok(());
        }
        self.finish_switch(cpu, to, cur_state, resume_pc)
    }

    /// The switch itself, then the new thread's in-switcher (run on the new thread before it continues).
    pub(crate) fn finish_switch(&mut self, cpu: &mut Cpu, to: u32, cur_state: Option<TState>, resume_pc: u32) -> Result<(), String> {
        self.raw_switch(cpu, to, cur_state, resume_pc)?;
        if let Some((tv, param)) = self.rt.sched.tasks.get(&to).and_then(|t| t.switch_in) {
            let frame = Frame { saved: crate::callback::Saved::of(cpu, cpu.pc), cont: crate::callback::Continuation::SwitchIn };
            let sp = cpu.gpr[1];
            self.enter_tvector(cpu, tv, &[to, param], sp).map_err(|e| format!("thread switcher (in): {e}"))?;
            self.rt.callbacks.push(frame);
        }
        Ok(())
    }

    fn raw_switch(&mut self, cpu: &mut Cpu, to: u32, cur_state: Option<TState>, resume_pc: u32) -> Result<(), String> {
        let cur = self.rt.sched.current;
        if !self.rt.sched.tasks.get(&to).is_some_and(|t| t.state == TState::Ready && t.cpu.is_some()) { return Err(format!("switch to task {to}: it is not a ready task with saved registers")); }
        let callbacks = core::mem::take(&mut self.rt.callbacks);
        let ending = cur_state.is_none();
        let mut dead_stack = None;
        // The incoming task's registers come out first so the outgoing CPU can be MOVED into its slot: no clone (a clone copied the whole native-address set on every switch).
        let t = self.rt.sched.tasks.get_mut(&to).expect("checked");
        let next = t.cpu.take().expect("checked");
        let (next_callbacks, next_bound) = (core::mem::take(&mut t.callbacks), t.bound);
        t.state = TState::Running;
        let mut old = core::mem::replace(cpu, next);
        if let Some(state) = cur_state {
            old.pc = resume_pc;
            let t = self.rt.sched.tasks.get_mut(&cur).ok_or_else(|| format!("the current task {cur} is not in the table"))?;
            t.cpu = Some(old);
            t.callbacks = callbacks;
            t.state = state;
            t.bound = self.rt.bound; // the live CPU had exactly these stubs bound
        } else if let Some(t) = self.rt.sched.tasks.remove(&cur) { dead_stack = t.stack; }
        self.rt.callbacks = next_callbacks;
        self.rt.sched.current = to;
        let pc = cpu.pc;
        cpu.set_pc(pc); // the native "returned" into the other task: the run loop must not overwrite pc with this task's LR
        // only the stubs interned since this task last ran are missing from its bound set
        for i in next_bound..self.glue.len() { if let Some(e) = self.glue.entry(i) { cpu.bind_native(e.stub); self.rt.sched.rebinds += 1; } }
        self.rt.bound = self.glue.len().max(self.rt.bound);
        if ending { if let Some(s) = dead_stack { let _ = self.mac.mm.dispose_ptr(&mut self.mac.heap, s); } }
        Ok(())
    }

    /// `GlueOutcome::Idle` with another task ready: run that task now and retry the parked call (`addr`) when this one is scheduled again.
    pub(crate) fn idle_yield(&mut self, cpu: &mut Cpu, addr: u32) -> bool {
        if self.rt.sched.tasks.len() < 2 { return false; }
        let cur = self.rt.sched.current;
        let Some(to) = self.runnable_after(cur, false) else { return false };
        // the call is not parked: it will simply run again, so the machine-wide wait markers must not survive
        self.rt.wait_mask = None;
        self.rt.wait_deadline = None;
        self.rt.wait_dialog = false;
        self.mac.log.push(format!("[sched] task {cur} would idle: task {to} runs instead"));
        match self.switch_to(cpu, to, Some(TState::Ready), addr) {
            Ok(()) => true,
            Err(why) => { self.rt.fault = Some(why); cpu.halt(); true }
        }
    }
}
