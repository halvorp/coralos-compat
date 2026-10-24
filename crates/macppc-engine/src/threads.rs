//! The Thread Manager (Threads.h; Inside Macintosh: Thread Manager) as COOPERATIVE threads over the scheduler ([`crate::sched`]). The calls are
//! exports of `ThreadsLib` (the PowerPC shared library; the Systemless import census shows real programs import them from it), served natively: a call that switches replaces the live registers
//! with another thread's saved ones (the PowerPC code on both sides is interpreted; the library never is).
//!
//! * `NewThread(style, entry, param, stackSize, options, &result, &made)`: `entry` is a UPP (a routine descriptor) or, as a PowerPC-only program
//!   may pass it, a bare TVector; it is called `void* entry(void* param)` on a fresh stack allocated from the guest heap (`stackSize` 0 = the default,
//!   [`DEFAULT_STACK`]) with LR at `$ThreadExit`, so returning ends the thread with that value (stored at `result`). A new thread is READY unless
//!   `options` has `kNewSuspend` (1), in which case it is stopped until `SetThreadState` readies it. `kPreemptiveThread` (2) threads are created and
//!   run COOPERATIVELY (logged): the classic Mac OS gave them no more than that on a single processor without MP-aware code.
//!   `kUsePremadeThread`/`kCreateIfNeeded`/`kFPUNotNeeded`/`kExactMatchThread` are accepted and ignored: there is no thread pool and the FPU is
//!   always saved with the registers.
//! * Switching happens only in `YieldToThread`, `YieldToAnyThread` (round robin by thread id), `DisposeThread`/return of the running thread,
//!   `SetThreadState(self, stopped)`, and the idle-yield rule. `ThreadBeginCritical`/`EndCritical` count and change nothing: nothing else switches.
//!   The custom scheduler/switcher/terminator hooks (`SetThreadScheduler`, ...) are not provided and refuse by name.
//! * Errors: `threadNotFoundErr` -618 for an id that is not (or no longer) a thread, `threadProtocolErr` -619 for yielding to a stopped thread or
//!   disposing the application thread, `paramErr` -50 for a bad style/state, `memFullErr` -108 when the stack cannot be allocated.
use crate::clib::arg;
use crate::machine::MacPpc;
use crate::sched::{Kind, Task, TState, MAIN_TASK};
use crate::symbols::{Binding, Native};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use guestui_abi::GuestUi;
use mac68k_engine::mixed_mode::{parse, ppc_record};
use retro_ppc::Cpu;

pub const NAME: &str = "ThreadsLib";
/// The pseudo-symbol a thread's entry routine returns to.
pub const THREAD_EXIT: &str = "$ThreadExit";
/// Stack for `NewThread(.., stackSize = 0, ..)` and `GetDefaultThreadStackSize`.
pub const DEFAULT_STACK: u32 = 64 * 1024;
pub const THREAD_NOT_FOUND: i32 = -618;
pub const THREAD_PROTOCOL: i32 = -619;
const PARAM_ERR: i32 = -50;
const MEM_FULL: i32 = -108;
const K_COOPERATIVE: u32 = 1;
const K_PREEMPTIVE: u32 = 2;
const K_NEW_SUSPEND: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadFn { NewThread, GetCurrentThread, YieldToThread, YieldToAnyThread, DisposeThread, GetThreadState, SetThreadState, ThreadBeginCritical, ThreadEndCritical, GetDefaultThreadStackSize, Exit }

impl ThreadFn {
    pub fn by_name(name: &str) -> Option<ThreadFn> {
        use ThreadFn::*;
        Some(match name {
            "NewThread" => NewThread, "GetCurrentThread" => GetCurrentThread, "YieldToThread" => YieldToThread, "YieldToAnyThread" => YieldToAnyThread,
            "DisposeThread" => DisposeThread, "GetThreadState" => GetThreadState, "SetThreadState" => SetThreadState,
            "ThreadBeginCritical" => ThreadBeginCritical, "ThreadEndCritical" => ThreadEndCritical, "GetDefaultThreadStackSize" => GetDefaultThreadStackSize,
            _ => return None,
        })
    }
}

fn state_code(s: TState) -> u32 { match s { TState::Ready => 0, TState::Stopped | TState::Waiting => 1, TState::Running => 2 } }

impl<U: GuestUi> MacPpc<U> {
    /// The `$ThreadExit` stub address (interned, bound as a native on first use).
    pub(crate) fn thread_exit_stub(&mut self) -> Result<u32, String> {
        self.bindings.insert(THREAD_EXIT.into(), Binding::Native(Native::Thread(ThreadFn::Exit)));
        let id = self.glue.intern(&mut self.mac, "CFM", THREAD_EXIT).map_err(|e| format!("{e:?}"))?;
        self.glue.entry(id).map(|e| e.stub).ok_or_else(|| String::from("the $ThreadExit glue cell is missing"))
    }

    /// A procedure pointer that is either a PowerPC routine descriptor or a bare TVector -> the TVector.
    pub(crate) fn proc_tvector(&self, p: u32) -> Result<u32, String> {
        match parse(&self.mac.mem, p).and_then(|d| ppc_record(&d)) {
            Ok(r) => Ok(r.proc_descriptor),
            Err(_) => Ok(p), // not a descriptor: a CFM function pointer is the TVector itself
        }
    }

    fn put32(&mut self, at: u32, v: u32, who: &str) -> Result<(), String> {
        if at == 0 { return Ok(()); }
        self.mac.mem.write_u32(at, v).map_err(|_| format!("{who}: result pointer {at:#x} leaves guest RAM"))
    }

    /// The next thread to run when the current one cannot: its suggestion if ready, else round robin; none is a named fault.
    fn pick(&mut self, suggested: u32, who: &str) -> Result<u32, String> {
        let cur = self.rt.sched.current;
        if suggested != 0 && suggested != cur && self.rt.sched.tasks.get(&suggested).is_some_and(|t| t.state == TState::Ready) { return Ok(suggested); }
        self.runnable_after(cur, true).ok_or_else(|| format!("{who}: no task is ready to run (the last runnable task stopped, ended or waits with no deadline)"))
    }

    /// Serve `f` for the call the CPU is making.
    pub(crate) fn thread_call(&mut self, f: ThreadFn, cpu: &mut Cpu) -> Result<(), String> {
        use ThreadFn::*;
        self.ensure_sched();
        let a = |m: &Self, i: usize| arg(m, cpu, i);
        let (a0, a1, a2) = (a(self, 0)?, a(self, 1).unwrap_or(0), a(self, 2).unwrap_or(0));
        let ret = cpu.lr & !3;
        let cur = self.rt.sched.current;
        let r: i32 = match f {
            NewThread => {
                let (style, entry, param, stack, options, result_p, made_p) = (a0, a1, a2, a(self, 3)?, a(self, 4)?, a(self, 5)?, a(self, 6)?);
                if style != K_COOPERATIVE && style != K_PREEMPTIVE { cpu.gpr[3] = PARAM_ERR as u32; return Ok(()); }
                let tv = self.proc_tvector(entry)?;
                let (code, toc) = (self.mac.mem.read_u32(tv), self.mac.mem.read_u32(tv + 4));
                let (Ok(code), Ok(toc)) = (code, toc) else { return Err(format!("NewThread: the entry {entry:#x} is not a TVector in guest RAM")) };
                let size = if stack == 0 { DEFAULT_STACK } else { stack.max(256) };
                let Ok(block) = self.mac.mm.new_ptr(&mut self.mac.mem, &mut self.mac.heap, size, false) else { cpu.gpr[3] = MEM_FULL as u32; return Ok(()); };
                let r1 = (block + size - 64) & !0xF;
                self.mac.mem.write_u32(r1, 0).map_err(|e| format!("NewThread: the new stack: {e}"))?;
                let exit = self.thread_exit_stub()?;
                let mut c = cpu.clone();
                c.gpr = [0; 32]; c.fpr = [0; 32]; c.cr = 0; c.ctr = 0; c.xer = 0; c.fpscr = 0;
                c.gpr[1] = r1; c.gpr[2] = toc; c.gpr[3] = param; c.gpr[12] = tv; c.lr = exit; c.pc = code;
                let id = self.rt.sched.next_id;
                self.rt.sched.next_id += 1;
                let state = if options & K_NEW_SUSPEND != 0 { TState::Stopped } else { TState::Ready };
                self.rt.sched.tasks.insert(id, Task { cpu: Some(c), callbacks: Vec::new(), state, kind: Kind::Thread, stack: Some(block), result_ptr: result_p, wait: None, woken: None, notify: None });
                self.put32(made_p, id, "NewThread")?;
                self.mac.log.push(format!("[threads] NewThread -> {id} ({}{}, entry {tv:#x}, stack {size})", if style == K_PREEMPTIVE { "preemptive requested, run cooperatively, " } else { "cooperative, " }, if state == TState::Stopped { "stopped" } else { "ready" }));
                0
            }
            GetCurrentThread => { self.put32(a0, cur, "GetCurrentThread")?; 0 }
            YieldToAnyThread => match self.runnable_after(cur, false) {
                Some(to) => { cpu.gpr[3] = 0; return self.switch_to(cpu, to, Some(TState::Ready), ret); }
                None => 0,
            },
            YieldToThread => {
                if a0 == cur { 0 } else {
                    match self.rt.sched.tasks.get(&a0).map(|t| t.state) {
                        None => THREAD_NOT_FOUND,
                        Some(TState::Ready) => { cpu.gpr[3] = 0; return self.switch_to(cpu, a0, Some(TState::Ready), ret); }
                        Some(_) => THREAD_PROTOCOL,
                    }
                }
            }
            DisposeThread => {
                let (id, value) = (a0, a1);
                if id == MAIN_TASK { THREAD_PROTOCOL }
                else if id == cur { return self.end_current(cpu, value); }
                else {
                    match self.rt.sched.tasks.remove(&id) {
                        Some(t) if t.kind == Kind::Thread => {
                            if t.result_ptr != 0 { self.put32(t.result_ptr, value, "DisposeThread")?; }
                            if let Some(s) = t.stack { let _ = self.mac.mm.dispose_ptr(&mut self.mac.heap, s); }
                            0
                        }
                        Some(t) => { self.rt.sched.tasks.insert(id, t); THREAD_PROTOCOL }
                        None => THREAD_NOT_FOUND,
                    }
                }
            }
            Exit => return self.end_current(cpu, cpu.gpr[3]),
            GetThreadState => match self.rt.sched.tasks.get(&a0) {
                Some(t) => { self.put32(a1, state_code(t.state), "GetThreadState")?; 0 }
                None => THREAD_NOT_FOUND,
            },
            SetThreadState => {
                let (id, state, suggested) = (a0, a1, a2);
                if !self.rt.sched.tasks.contains_key(&id) { THREAD_NOT_FOUND }
                else if state > 2 { PARAM_ERR }
                else if id == cur {
                    match state {
                        1 => { let to = self.pick(suggested, "SetThreadState")?; cpu.gpr[3] = 0; return self.switch_to(cpu, to, Some(TState::Stopped), ret); }
                        _ => 0,
                    }
                } else {
                    match state { 0 => self.rt.sched.tasks.get_mut(&id).expect("checked").state = TState::Ready, 1 => self.rt.sched.tasks.get_mut(&id).expect("checked").state = TState::Stopped, _ => {} }
                    0
                }
            }
            ThreadBeginCritical => { self.rt.sched.critical += 1; 0 }
            ThreadEndCritical => { self.rt.sched.critical = self.rt.sched.critical.saturating_sub(1); 0 }
            GetDefaultThreadStackSize => { self.put32(a1, DEFAULT_STACK, "GetDefaultThreadStackSize")?; 0 }
        };
        cpu.gpr[3] = r as u32;
        Ok(())
    }

    /// The running thread ends with exit value `value`: store it, run the next ready thread. The application thread cannot end this way.
    pub(crate) fn end_current(&mut self, cpu: &mut Cpu, value: u32) -> Result<(), String> {
        let cur = self.rt.sched.current;
        if cur == MAIN_TASK { cpu.gpr[3] = THREAD_PROTOCOL as u32; return Ok(()); }
        let (rp, notify, kind) = self.rt.sched.tasks.get(&cur).map_or((0, None, Kind::Thread), |t| (t.result_ptr, t.notify, t.kind));
        self.put32(rp, value, "thread exit")?;
        if kind == Kind::Mp { self.mp_task_ended(notify, value); }
        let to = self.pick(0, "thread exit")?;
        self.mac.log.push(format!("[threads] thread {cur} ended (value {value:#x}); thread {to} runs"));
        self.switch_to(cpu, to, None, 0)
    }
}
