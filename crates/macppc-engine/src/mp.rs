//! Multiprocessing Services (`MPLibrary`; Multiprocessing.h) as COOPERATIVE tasks over the scheduler ([`crate::sched`]): tasks, queues, semaphores,
//! critical regions, `MPYield`, `MPCurrentTaskID`, `MPProcessors`. Tasks are interpreted PowerPC; the library is native.
//!
//! WHAT IS HONEST: there is one processor (`MPProcessors()` is 1) and a task runs until it yields, waits, or ends -- there is NO preemption. A
//! program that spins on a flag another task sets (legal on real MP hardware) never lets the setter run and the run ends only by the instruction
//! budget. Preemption needs the scheduler to cut a task off in `resume` (`retro_ppc::resume(host, cpu, slice)`) and switch at the slice boundary,
//! with the interrupted task's registers saved exactly as a yield saves them; the primitive for that (`switch_to`) exists, the slice policy and a
//! fairness rule do not. Gestalt `'mpsv'`/`'mpsc'` therefore stay unanswered (gestalt_ppc).
//! WAITING: a task that waits (queue, semaphore, critical region) is Waiting until signalled or until its `Duration` runs out. Time is the machine's
//! tick clock; when NO task is ready and some are waiting with deadlines, the clock is advanced to the earliest deadline (a sleeping machine would
//! have done exactly that), and when nothing can ever wake, the run stops with a named fault.
//! DURATION (Multiprocessing.h / Timer.h): 0 = immediate (`kDurationImmediate`), 0x7FFFFFFF = forever, positive = milliseconds, negative = microseconds;
//! rounded up to whole ticks (60/s).
//! TERMINATION NOTICE (assumption, from the header's documentation of `MPCreateTask`): when a task ends, `notifyQueue` receives
//! (terminationParameter1, terminationParameter2, the task's status).
//! Error numbers are `MacErrors.h`'s `kMP...Err` values (names checked against Nitrogen's MacErrors.hh; the numbers are as I recall them from the header).
use crate::clib::arg;
use crate::machine::MacPpc;
use crate::sched::{Kind, Task, TState, Wait, WaitOn};
use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use guestui_abi::GuestUi;
use retro_ppc::Cpu;

pub const NAME: &str = "MPLibrary";
pub const MP_DELETED_ERR: i32 = -29295;
pub const MP_TIMEOUT_ERR: i32 = -29296;
pub const MP_INSUFFICIENT_RESOURCES_ERR: i32 = -29298;
pub const MP_INVALID_ID_ERR: i32 = -29299;
const DURATION_FOREVER: u32 = 0x7FFF_FFFF;
const DEFAULT_STACK: u32 = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MpFn {
    MPLibraryIsLoaded, MPProcessors, MPCurrentTaskID, MPYield, MPCreateTask, MPTerminateTask,
    MPCreateQueue, MPDeleteQueue, MPNotifyQueue, MPWaitOnQueue,
    MPCreateSemaphore, MPDeleteSemaphore, MPSignalSemaphore, MPWaitOnSemaphore,
    MPCreateCriticalRegion, MPDeleteCriticalRegion, MPEnterCriticalRegion, MPExitCriticalRegion,
}
impl MpFn {
    pub fn by_name(name: &str) -> Option<MpFn> {
        use MpFn::*;
        Some(match name {
            "MPLibraryIsLoaded" => MPLibraryIsLoaded, "MPProcessors" => MPProcessors, "MPCurrentTaskID" => MPCurrentTaskID, "MPYield" => MPYield,
            "MPCreateTask" => MPCreateTask, "MPTerminateTask" => MPTerminateTask, "MPCreateQueue" => MPCreateQueue, "MPDeleteQueue" => MPDeleteQueue,
            "MPNotifyQueue" => MPNotifyQueue, "MPWaitOnQueue" => MPWaitOnQueue, "MPCreateSemaphore" => MPCreateSemaphore, "MPDeleteSemaphore" => MPDeleteSemaphore,
            "MPSignalSemaphore" => MPSignalSemaphore, "MPWaitOnSemaphore" => MPWaitOnSemaphore, "MPCreateCriticalRegion" => MPCreateCriticalRegion,
            "MPDeleteCriticalRegion" => MPDeleteCriticalRegion, "MPEnterCriticalRegion" => MPEnterCriticalRegion, "MPExitCriticalRegion" => MPExitCriticalRegion,
            _ => return None,
        })
    }
}

#[derive(Default)]
pub(crate) struct Queue { items: VecDeque<[u32; 3]>, waiters: VecDeque<u32> }
pub(crate) struct Sem { count: u32, max: u32, waiters: VecDeque<u32> }
#[derive(Default)]
pub(crate) struct Region { owner: Option<u32>, depth: u32, waiters: VecDeque<u32> }
#[derive(Default)]
pub(crate) struct MpState { queues: BTreeMap<u32, Queue>, sems: BTreeMap<u32, Sem>, regions: BTreeMap<u32, Region>, next_id: u32 }

fn ticks_of(duration: u32) -> Option<u32> {
    match duration {
        0 => Some(0),
        DURATION_FOREVER => None,
        d if (d as i32) > 0 => Some((u64::from(d) * 60).div_ceil(1000).max(1) as u32),
        d => Some((u64::from((d as i32).unsigned_abs()) * 60).div_ceil(1_000_000).max(1) as u32),
    }
}

impl<U: GuestUi> MacPpc<U> {
    fn mp_id(&mut self) -> u32 { self.rt.mp.next_id += 1; 0x1000 + self.rt.mp.next_id }

    /// Wake task `t` with `status` in r3 (and drop it from whatever it waited on).
    fn wake(&mut self, t: u32, status: i32) {
        if let Some(task) = self.rt.sched.tasks.get_mut(&t) {
            match task.cpu.as_mut() { Some(c) => c.gpr[3] = status as u32, None => task.woken = Some(status) }
            task.state = TState::Ready;
            task.wait = None;
        }
    }

    fn unlist(&mut self, t: u32, on: WaitOn) {
        let mp = &mut self.rt.mp;
        match on {
            WaitOn::Queue(q) => { if let Some(x) = mp.queues.get_mut(&q) { x.waiters.retain(|&w| w != t); } }
            WaitOn::Semaphore(s) => { if let Some(x) = mp.sems.get_mut(&s) { x.waiters.retain(|&w| w != t); } }
            WaitOn::Region(r) => { if let Some(x) = mp.regions.get_mut(&r) { x.waiters.retain(|&w| w != t); } }
        }
    }

    /// Time out every waiter whose deadline has passed.
    pub(crate) fn expire_waits(&mut self) {
        let now = self.mac.ev.ticks;
        let due: Vec<(u32, WaitOn)> = self.rt.sched.tasks.iter().filter_map(|(&id, t)| t.wait.filter(|w| w.deadline.is_some_and(|d| now.wrapping_sub(d) < 0x8000_0000)).map(|w| (id, w.on))).collect();
        for (id, on) in due { self.unlist(id, on); self.wake(id, MP_TIMEOUT_ERR); }
    }

    /// Nothing is ready: advance the clock to the earliest deadline and time that waiter out. False when nobody waits with a deadline.
    pub(crate) fn wake_earliest_timeout(&mut self) -> bool {
        let now = self.mac.ev.ticks;
        let Some((id, w)) = self.rt.sched.tasks.iter().filter_map(|(&id, t)| t.wait.filter(|w| w.deadline.is_some()).map(|w| (id, w)))
            .min_by_key(|(_, w)| w.deadline.expect("filtered").wrapping_sub(now)) else { return false };
        let ahead = w.deadline.expect("filtered").wrapping_sub(now);
        if ahead < 0x8000_0000 && ahead > 0 { self.mac.advance_ticks(ahead); }
        self.unlist(id, w.on);
        self.wake(id, MP_TIMEOUT_ERR);
        true
    }

    /// The next task to run after `cur`: a Ready one, after timeouts that have passed; with `may_sleep`, after advancing the clock to the next deadline.
    pub(crate) fn runnable_after(&mut self, cur: u32, may_sleep: bool) -> Option<u32> {
        self.expire_waits();
        if let Some(t) = self.rt.sched.next_ready(cur) { return Some(t); }
        if may_sleep && self.wake_earliest_timeout() { return self.rt.sched.next_ready(cur); }
        None
    }

    /// Block the current task on `wait` (it resumes at `ret` with r3 set by whoever wakes it).
    fn block(&mut self, cpu: &mut Cpu, wait: Wait, ret: u32, who: &str) -> Result<(), String> {
        let cur = self.rt.sched.current;
        if let Some(t) = self.rt.sched.tasks.get_mut(&cur) { t.wait = Some(wait); }
        let found = self.runnable_after(cur, true);
        // the clock may have run out THIS task's own wait: it simply continues with the timeout status
        if let Some(st) = self.rt.sched.tasks.get_mut(&cur).and_then(|t| t.woken.take()) {
            if let Some(t) = self.rt.sched.tasks.get_mut(&cur) { t.state = TState::Running; t.wait = None; }
            cpu.gpr[3] = st as u32;
            return Ok(());
        }
        let to = match found {
            Some(t) => t,
            None => {
                if let Some(t) = self.rt.sched.tasks.get_mut(&cur) { t.wait = None; }
                return Err(format!("{who}: every task is waiting and none has a deadline (a deadlock)"));
            }
        };
        self.mac.log.push(format!("[mp] task {cur} waits ({who}); task {to} runs"));
        self.switch_to(cpu, to, Some(TState::Waiting), ret)
    }

    /// The MP task ended with `status`: its notice goes to the notify queue.
    pub(crate) fn mp_task_ended(&mut self, notify: Option<(u32, u32, u32)>, status: u32) {
        if let Some((q, p1, p2)) = notify { let _ = self.queue_notify(q, [p1, p2, status]); }
    }

    fn queue_notify(&mut self, q: u32, msg: [u32; 3]) -> i32 {
        let Some(queue) = self.rt.mp.queues.get_mut(&q) else { return MP_INVALID_ID_ERR };
        if let Some(w) = queue.waiters.pop_front() {
            let out = self.rt.sched.tasks.get(&w).and_then(|t| t.wait).map_or([0; 3], |x| x.out);
            for (p, v) in out.iter().zip(msg) { if *p != 0 { let _ = self.mac.mem.write_u32(*p, v); } }
            self.wake(w, 0);
        } else { queue.items.push_back(msg); }
        0
    }

    /// Serve `f`.
    pub(crate) fn mp_call(&mut self, f: MpFn, cpu: &mut Cpu) -> Result<(), String> {
        use MpFn::*;
        self.ensure_sched();
        let a = |m: &Self, i: usize| arg(m, cpu, i);
        let (a0, a1, a2, a3) = (a(self, 0)?, a(self, 1).unwrap_or(0), a(self, 2).unwrap_or(0), a(self, 3).unwrap_or(0));
        let (ret, cur) = (cpu.lr & !3, self.rt.sched.current);
        let put = |m: &mut Self, at: u32, v: u32, who: &str| if at == 0 { Ok(()) } else { m.mac.mem.write_u32(at, v).map_err(|_| format!("{who}: result pointer {at:#x} leaves guest RAM")) };
        let r: i32 = match f {
            MPLibraryIsLoaded => 1,
            MPProcessors => 1,
            MPCurrentTaskID => cur as i32,
            MPYield => match self.runnable_after(cur, false) {
                Some(to) => { cpu.gpr[3] = 0; return self.switch_to(cpu, to, Some(TState::Ready), ret); }
                None => 0,
            },
            MPCreateTask => {
                let (entry, param, stack, notify_q) = (a0, a1, a2, a3);
                let (p1, p2, _options, task_p) = (a(self, 4)?, a(self, 5)?, a(self, 6)?, a(self, 7)?);
                if notify_q != 0 && !self.rt.mp.queues.contains_key(&notify_q) { cpu.gpr[3] = MP_INVALID_ID_ERR as u32; return Ok(()); }
                let tv = self.proc_tvector(entry)?;
                let (Ok(code), Ok(toc)) = (self.mac.mem.read_u32(tv), self.mac.mem.read_u32(tv + 4)) else { return Err(format!("MPCreateTask: the entry {entry:#x} is not a TVector in guest RAM")) };
                let size = if stack == 0 { DEFAULT_STACK } else { stack.max(256) };
                let Ok(block) = self.mac.mm.new_ptr(&mut self.mac.mem, &mut self.mac.heap, size, false) else { cpu.gpr[3] = MP_INSUFFICIENT_RESOURCES_ERR as u32; return Ok(()); };
                let r1 = (block + size - 64) & !0xF;
                self.mac.mem.write_u32(r1, 0).map_err(|e| format!("MPCreateTask: the new stack: {e}"))?;
                let exit = self.thread_exit_stub()?;
                let mut c = cpu.clone();
                c.gpr = [0; 32]; c.fpr = [0; 32]; c.cr = 0; c.ctr = 0; c.xer = 0; c.fpscr = 0;
                c.gpr[1] = r1; c.gpr[2] = toc; c.gpr[3] = param; c.gpr[12] = tv; c.lr = exit; c.pc = code;
                let id = self.rt.sched.next_id;
                self.rt.sched.next_id += 1;
                self.rt.sched.tasks.insert(id, Task { cpu: Some(c), callbacks: Vec::new(), state: TState::Ready, kind: Kind::Mp, stack: Some(block), result_ptr: 0, wait: None, woken: None, terminator: None, switch_in: None, switch_out: None, notify: (notify_q != 0).then_some((notify_q, p1, p2)), bound: 0 });
                put(self, task_p, id, "MPCreateTask")?;
                self.mac.log.push(format!("[mp] MPCreateTask -> task {id} (entry {tv:#x}, stack {size}, cooperative)"));
                0
            }
            MPTerminateTask => {
                let (id, status) = (a0, a1);
                if id == cur { return self.end_current(cpu, status); }
                match self.rt.sched.tasks.get(&id).map(|t| t.kind) {
                    Some(Kind::Mp) => {
                        let t = self.rt.sched.tasks.remove(&id).expect("checked");
                        if let Some(w) = t.wait { self.unlist(id, w.on); }
                        if let Some(s) = t.stack { let _ = self.mac.mm.dispose_ptr(&mut self.mac.heap, s); }
                        self.mp_task_ended(t.notify, status);
                        0
                    }
                    _ => MP_INVALID_ID_ERR,
                }
            }
            MPCreateQueue => { let id = self.mp_id(); self.rt.mp.queues.insert(id, Queue::default()); put(self, a0, id, "MPCreateQueue")?; 0 }
            MPDeleteQueue => match self.rt.mp.queues.remove(&a0) {
                Some(q) => { for w in q.waiters { self.wake(w, MP_DELETED_ERR); } 0 }
                None => MP_INVALID_ID_ERR,
            },
            MPNotifyQueue => self.queue_notify(a0, [a1, a2, a3]),
            MPWaitOnQueue => {
                let (q, out, timeout) = (a0, [a1, a2, a3], a(self, 4)?);
                match self.rt.mp.queues.get_mut(&q) {
                    None => MP_INVALID_ID_ERR,
                    Some(x) => {
                        if let Some(msg) = x.items.pop_front() {
                            for (p, v) in out.iter().zip(msg) { put(self, *p, v, "MPWaitOnQueue")?; }
                            0
                        } else if timeout == 0 { MP_TIMEOUT_ERR }
                        else {
                            x.waiters.push_back(cur);
                            let deadline = ticks_of(timeout).map(|t| self.mac.ev.ticks.wrapping_add(t));
                            return self.block(cpu, Wait { on: WaitOn::Queue(q), deadline, out }, ret, "MPWaitOnQueue");
                        }
                    }
                }
            }
            MPCreateSemaphore => { let id = self.mp_id(); self.rt.mp.sems.insert(id, Sem { count: a1.min(a0), max: a0, waiters: VecDeque::new() }); put(self, a2, id, "MPCreateSemaphore")?; 0 }
            MPDeleteSemaphore => match self.rt.mp.sems.remove(&a0) {
                Some(s) => { for w in s.waiters { self.wake(w, MP_DELETED_ERR); } 0 }
                None => MP_INVALID_ID_ERR,
            },
            MPSignalSemaphore => match self.rt.mp.sems.get_mut(&a0) {
                None => MP_INVALID_ID_ERR,
                Some(s) => {
                    if let Some(w) = s.waiters.pop_front() { self.wake(w, 0); 0 }
                    else if s.count < s.max { s.count += 1; 0 }
                    else { MP_INSUFFICIENT_RESOURCES_ERR }
                }
            },
            MPWaitOnSemaphore => match self.rt.mp.sems.get_mut(&a0) {
                None => MP_INVALID_ID_ERR,
                Some(s) => {
                    if s.count > 0 { s.count -= 1; 0 }
                    else if a1 == 0 { MP_TIMEOUT_ERR }
                    else {
                        s.waiters.push_back(cur);
                        let deadline = ticks_of(a1).map(|t| self.mac.ev.ticks.wrapping_add(t));
                        return self.block(cpu, Wait { on: WaitOn::Semaphore(a0), deadline, out: [0; 3] }, ret, "MPWaitOnSemaphore");
                    }
                }
            },
            MPCreateCriticalRegion => { let id = self.mp_id(); self.rt.mp.regions.insert(id, Region::default()); put(self, a0, id, "MPCreateCriticalRegion")?; 0 }
            MPDeleteCriticalRegion => match self.rt.mp.regions.remove(&a0) {
                Some(r) => { for w in r.waiters { self.wake(w, MP_DELETED_ERR); } 0 }
                None => MP_INVALID_ID_ERR,
            },
            MPEnterCriticalRegion => match self.rt.mp.regions.get_mut(&a0) {
                None => MP_INVALID_ID_ERR,
                Some(r) => {
                    if r.owner.is_none() || r.owner == Some(cur) { r.owner = Some(cur); r.depth += 1; 0 }
                    else if a1 == 0 { MP_TIMEOUT_ERR }
                    else {
                        r.waiters.push_back(cur);
                        let deadline = ticks_of(a1).map(|t| self.mac.ev.ticks.wrapping_add(t));
                        return self.block(cpu, Wait { on: WaitOn::Region(a0), deadline, out: [0; 3] }, ret, "MPEnterCriticalRegion");
                    }
                }
            },
            MPExitCriticalRegion => match self.rt.mp.regions.get_mut(&a0) {
                None => MP_INVALID_ID_ERR,
                Some(r) if r.owner != Some(cur) => MP_INVALID_ID_ERR,
                Some(r) => {
                    r.depth -= 1;
                    if r.depth == 0 {
                        r.owner = None;
                        if let Some(w) = r.waiters.pop_front() { r.owner = Some(w); r.depth = 1; self.wake(w, 0); }
                    }
                    0
                }
            },
        };
        cpu.gpr[3] = r as u32;
        Ok(())
    }
}
