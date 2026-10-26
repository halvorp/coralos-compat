//! Running a PowerPC application: the [`heirloom_cpu_ppc`] interpreter as the CPU, the glue stubs as native hooks.
//!
//! [`MacPpc::start`] instantiates nothing itself (that is [`MacPpc::instantiate`]); it builds the CPU the way the Code
//! Fragment Manager enters a fragment: a fresh stack, r1 16-byte aligned over a zero back chain, r2 = the TOC of the
//! entry TVector, r12 = the TVector's own address (the pointer-glue convention: a callee may rely on it), LR = the
//! `ExitToShell` glue so returning from `main` ends the program. A fragment with an init routine runs it first: LR is
//! then a pseudo-glue (`$InitDone`) that, when reached, switches the CPU to `main`. The init routine gets r3 = 0 where
//! the real CFM passes a `CFragInitBlock *` (not built: nothing here reads it; named, not hidden). Its OSErr result (r3) is
//! honoured: nonzero is a fault and `main` does not run. Term routines are not
//! run (ExitToShell of a classic application tears the whole process down).
//!
//! [`MacPpc::step`] is the same resumable shape as the 68k engines: `Exited | Blocked | BudgetSpent | Fault`.
//! Every glue address is bound with `Cpu::bind_native`; the interpreter calls [`Host::native`] there, which runs the
//! Toolbox handler through [`crate::host::PpcHost::glue_call`] with the live r0..r31 and writes back the result.
//! `WaitNextEvent` with nothing to deliver parks the CPU ON the call (`Blocked`); a step resumes it only once the GuestUi
//! has produced an event the call's mask accepts.
use crate::cfm::{Fragment, StartState};
use crate::error::CfmError;
use crate::host::{GlueOutcome, PpcHost};
use crate::machine::{MacPpc, UNBOUND_DATA};
use crate::symbols::{Binding, Native};
use crate::threads::ThreadFn;
use alloc::format;
use alloc::string::String;
use heirloom_ui_abi::GuestUi;
use heirloom_common::Step;
use heirloom_mem_flat32::GuestMem;
use heirloom_cpu_ppc::{Cpu, Host, RunStats};

/// The pseudo-symbol an init routine returns to.
pub const INIT_DONE: &str = "$InitDone";

/// The CPU and what the run loop tracks around it.
#[derive(Default)]
pub struct Runtime {
    cpu: Option<Cpu>,
    /// Glue cells below this index are already bound as natives in the CPU.
    pub(crate) bound: usize,
    /// Set by a `WaitNextEvent` that parked the CPU: the event mask a queued event must match to resume.
    pub(crate) wait_mask: Option<u16>,
    /// The tick at which that wait times out (`WaitNextEvent`'s sleep; none for "until an event comes", sleep >= 0x50000).
    pub(crate) wait_deadline: Option<u32>,
    /// The parked wait ran out of time: the retried call must return the null event instead of parking again.
    pub(crate) wait_timed_out: bool,
    /// Set by a `ModalDialog` that parked the CPU: it resumes when the front dialog has an item hit.
    pub(crate) wait_dialog: bool,
    /// A `StandardAlert` is parked on the host's answer ([`crate::stdalert_c`]).
    pub(crate) std_alert: bool,
    /// `main`'s entry state, held while the init routine runs.
    after_init: Option<StartState>,
    /// Init routines still to run before `main` (the libraries' and the application's), in order.
    init_queue: alloc::collections::VecDeque<crate::cfm::Tv>,
    /// The scratch File Manager parameter block (`crate::files`), allocated on first use.
    pub(crate) fm_pb: Option<u32>,
    /// `rand`'s state and the last `errno` a native set.
    pub(crate) rand: u32,
    /// `atexit` handlers (TVectors), in registration order.
    pub(crate) atexit: alloc::vec::Vec<u32>,
    /// Open `FILE` streams by the FILE block's guest address ([`crate::stdio`]).
    /// The task table ([`crate::sched`]).
    pub(crate) sched: crate::sched::Sched,
    pub(crate) mp: crate::mp::MpState,
    pub(crate) cf: crate::cf::CfState,
    pub(crate) cfx: crate::cf_more::CfxState,
    pub(crate) carbon: crate::carbon::CarbonState,
    /// The PowerPC execution tier and its counters ([`heirloom_cpu_ppc::Exec`]); outlives CPU context switches.
    pub(crate) exec: heirloom_cpu_ppc::Exec,
    pub(crate) ae: crate::ae_c::AeState,
    pub(crate) streams: alloc::collections::BTreeMap<u32, crate::stdio::Stream>,
    pub(crate) rand_seeded: bool,
    pub(crate) errno: u32,
    main_tvector: u32,
    /// The lowest address of the application thread's stack (for `ThreadCurrentStackSpace`; accurate to the 16-byte alignment of r1).
    pub(crate) main_stack: u32,
    /// A refusal or CPU trap; once set, every step reports it again (the program cannot continue).
    pub(crate) fault: Option<String>,
    /// Callbacks into the program in progress, innermost last (`crate::callback`).
    pub(crate) callbacks: alloc::vec::Vec<crate::callback::Frame>,
}

fn idle() -> RunStats { RunStats { insns: 0, halted: false, blocked: true, trap: None } }

impl<U: GuestUi> MacPpc<U> {
    /// The pointer-glue sequence (`lwz r12,d(r2); stw r2,20(r1); lwz r0,0(r12) ...`) that jumped to 0, decoded from the call site in LR: which import's TOC slot held the 0.
    fn null_call_import(&self, cpu: &Cpu) -> Option<String> {
        let call = cpu.lr.wrapping_sub(4);
        let w = self.mac.mem.read_u32(call).ok()?;
        if w >> 26 != 18 || w & 3 != 1 { return None; } // not a `bl`
        let li = ((w & 0x03FF_FFFC) as i32) << 6 >> 6;
        let glue = call.wrapping_add(li as u32);
        let first = self.mac.mem.read_u32(glue).ok()?;
        if first >> 26 != 32 || (first >> 21) & 31 != 12 || (first >> 16) & 31 != 2 { return None; } // not `lwz r12,d(r2)`
        let caller_toc = self.mac.mem.read_u32(cpu.gpr[1].wrapping_add(20)).ok()?;
        let slot = caller_toc.wrapping_add(first as u16 as i16 as i32 as u32);
        let (lib, sym, weak) = self.import_slots.get(&slot)?;
        Some(format!("the call at {call:#x} went through {lib}/{sym}, an import that is {} and was relocated to 0", if *weak { "WEAK and absent" } else { "unresolved" }))
    }

    /// Prepare to run `frag`: `main` (after `init`, when it has one).
    pub fn start(&mut self, frag: &Fragment) -> Result<(), CfmError> {
        let main = self.prepare_main(frag)?;
        self.bindings.insert(INIT_DONE.into(), Binding::Native(Native::InitDone));
        let mut cpu = Cpu::new(main.pc, main.r1);
        cpu.gpr[2] = main.r2;
        cpu.gpr[12] = frag.main.map_or(0, |t| t.address);
        cpu.lr = main.lr;
        self.rt = Runtime { main_tvector: cpu.gpr[12], main_stack: (main.r1 + 64).saturating_sub(crate::cfm::STACK_SIZE), ..Runtime::default() };
        // every library's init routine (dependencies first), then the application's own, run before main; each returns to `$InitDone`, which starts the next
        let mut inits: alloc::collections::VecDeque<crate::cfm::Tv> = self.lib_inits.iter().copied().collect();
        if let Some(init) = frag.init { inits.push_back(init); }
        if let Some(first) = inits.pop_front() {
            let id = self.glue.intern(&mut self.mac, "CFM", INIT_DONE)?;
            cpu.pc = first.code;
            cpu.gpr[2] = first.toc;
            cpu.gpr[12] = first.address;
            cpu.lr = self.glue.entry(id).map_or(0, |e| e.stub);
            self.rt.after_init = Some(main);
            self.rt.init_queue = inits;
        }
        self.sync_natives(&mut cpu);
        self.rt.cpu = Some(cpu);
        Ok(())
    }

    /// The CPU while a program is loaded (for inspection; `None` once it exited).
    pub fn cpu(&self) -> Option<&Cpu> { self.rt.cpu.as_ref() }
    /// Record the program counters of the last `n` instructions (0 = off), read back with `cpu().recent_pcs()` after a stop.
    pub fn trace_pcs(&mut self, n: usize) { if let Some(c) = self.rt.cpu.as_mut() { c.trace_pcs(n); } }
    /// The PowerPC execution tier, its profile counters and census ([`heirloom_cpu_ppc::Exec`]).
    pub fn exec(&mut self) -> &mut heirloom_cpu_ppc::Exec { &mut self.rt.exec }
    /// The refusal or trap that ended the program, if one did.
    pub fn fault(&self) -> Option<&str> { self.rt.fault.as_deref() }

    /// Bind every glue stub created since the last call (a `FindSymbol` interns cells while the program runs).
    pub(crate) fn sync_natives(&mut self, cpu: &mut Cpu) {
        while let Some(e) = self.glue.entry(self.rt.bound) { cpu.bind_native(e.stub); self.rt.bound += 1; }
    }

    /// Run at most `budget` instructions.
    pub fn step(&mut self, budget: u64) -> (Step, RunStats) {
        let gone = RunStats { halted: true, blocked: false, ..idle() };
        if let Some(f) = &self.rt.fault { return (Step::Fault(f.clone()), gone); }
        let Some(mut cpu) = self.rt.cpu.take() else { return (Step::Exited, gone) };
        if let Some(mask) = self.rt.wait_mask {
            self.mac.pump_ui();
            let timed_out = self.rt.wait_deadline.is_some_and(|d| self.mac.ev.ticks.wrapping_sub(d) < 0x8000_0000);
            // a Time Manager task that is due wakes the parked call too: the retried call is the safepoint that runs it
            let timer_due = self.tm_next_wake().is_some_and(|w| w <= self.mac.ev.ticks_total);
            let wake = self.mac.event_pending(mask) || timed_out || timer_due;
            if self.rt.sched.sweep.is_some() {
                if wake {
                    self.rt.sched.sweep = None;
                    self.rt.wait_timed_out = timed_out;
                    self.rt.wait_mask = None;
                    self.rt.wait_deadline = None;
                }
                // else the sweep is still going round: another task holds the CPU, so this is not a park
            } else {
                if !wake { self.rt.cpu = Some(cpu); return (Step::Blocked, idle()); }
                self.rt.wait_timed_out = timed_out;
                self.rt.wait_mask = None;
                self.rt.wait_deadline = None;
            }
        }
        if self.rt.wait_dialog && self.rt.sched.sweep.is_none() {
            if !self.mac.modal_ready() { self.rt.cpu = Some(cpu); return (Step::Blocked, idle()); }
            self.rt.wait_dialog = false;
        }
        self.sync_natives(&mut cpu);
        let mut exec = core::mem::take(&mut self.rt.exec);
        let stats = exec.resume(self, &mut cpu, budget);
        self.rt.exec = exec;
        heirloom_mac68k_engine::qd::flush(&mut self.mac);
        if let Some(t) = stats.trap {
            let mut why = format!("PowerPC fault: {t}");
            if let heirloom_cpu_ppc::Trap::BusFault { addr, .. } = t {
                if addr >= UNBOUND_DATA && addr - UNBOUND_DATA < 0x1000 && !self.unbound_data.is_empty() {
                    why = format!("{why} -- the placeholder address of an unbound DATA import; unbound data imports of this fragment: {}", self.unbound_data.join(", "));
                }
            }
            // a branch to address 0 is, almost always, a call through a weak import that is absent: name it
            if matches!(t, heirloom_cpu_ppc::Trap::Unimplemented { pc: 0, .. } | heirloom_cpu_ppc::Trap::BusFault { pc: 0, .. }) { if let Some(n) = self.null_call_import(&cpu) { why = format!("{why} -- {n}"); } }
            self.rt.fault = Some(why);
        }
        let step = if let Some(f) = &self.rt.fault { Step::Fault(f.clone()) }
            else if stats.halted { Step::Exited } else if stats.blocked { Step::Blocked } else { Step::BudgetSpent };
        if step != Step::Exited { self.rt.cpu = Some(cpu); }
        (step, stats)
    }

    /// Step until the program exits, blocks for an event, faults, or `max_insns` are spent. Returns the last step.
    pub fn run_loop(&mut self, max_insns: u64) -> (Step, u64) {
        let mut total = 0u64;
        loop {
            let (step, s) = self.step(max_insns.saturating_sub(total));
            total += s.insns;
            if step != Step::BudgetSpent || total >= max_insns { return (step, total); }
        }
    }
}

impl<U: GuestUi> MacPpc<U> {
    /// StdCLib functions that read or write the CPU's own registers (see [`crate::stdclib`]).
    fn cpu_native(&mut self, n: Native, cpu: &mut Cpu) {
        match n {
            Native::SetJmp => {
                let buf = cpu.gpr[3];
                // a glue stub is entered with r2 = 0 (its TVector's toc), so the CALLER's RTOC is the one it saved at 20(r1)
                let rtoc = self.mac.mem.read_u32(cpu.gpr[1].wrapping_add(20)).unwrap_or(cpu.gpr[2]);
                let saved = [cpu.gpr[1], rtoc, cpu.lr, cpu.cr].into_iter().chain(cpu.gpr[13..32].iter().copied());
                let ok = saved.enumerate().all(|(i, w)| self.mac.mem.write_u32(buf.wrapping_add(4 * i as u32), w).is_ok());
                if !ok { self.rt.fault = Some(format!("__setjmp: the jmp_buf at {buf:#x} leaves guest RAM")); cpu.halt(); }
                cpu.gpr[3] = 0;
            }
            Native::CExit => {
                let status = cpu.gpr[3];
                if let Some(&p) = self.cdata.get("_exit_status") { let _ = self.mac.mem.write_u32(p, status); }
                self.mac.log.push(format!("[stdclib] exit({})", status as i32));
                self.begin_exit(cpu);
            }
            _ => self.mac.log.push("[stdclib] _BreakPoint (no debugger: ignored)".into()),
        }
    }
}

impl<U: GuestUi> Host for MacPpc<U> {
    fn mem(&mut self) -> &mut GuestMem { &mut self.mac.mem }

    fn native(&mut self, cpu: &mut Cpu, addr: u32) -> bool {
        let Some(symbol) = self.glue_symbol_at(addr) else { return false };
        if symbol == INIT_DONE {
            let Some(m) = self.rt.after_init.clone() else { self.rt.fault = Some("the init routine returned twice".into()); cpu.halt(); return true };
            // a CFM initialization function returns an OSErr (16 bits); nonzero fails the fragment's preparation, so main never runs
            let status = cpu.gpr[3] as u16 as i16;
            if status != 0 {
                self.rt.fault = Some(format!("the fragment's init routine failed with OSErr {status}: the fragment is not prepared, main does not run"));
                cpu.halt();
                return true;
            }
            if let Some(next) = self.rt.init_queue.pop_front() {
                // the next init routine; r3 = 0 where the CFM passes a CFragInitBlock
                cpu.gpr[2] = next.toc;
                cpu.gpr[12] = next.address;
                cpu.gpr[3] = 0;
                cpu.lr = addr;
                cpu.set_pc(next.code);
                return true;
            }
            self.rt.after_init = None;
            cpu.gpr[1] = m.r1;
            cpu.gpr[2] = m.r2;
            cpu.gpr[12] = self.rt.main_tvector;
            cpu.lr = m.lr;
            cpu.set_pc(m.pc);
            return true;
        }
        // idle sweep: every call is progress except a yield (unless its turn computed) and a call that parks (undone below)
        let progressed_before = self.rt.sched.sweep.as_ref().map(|s| s.progressed);
        // Not work by itself: a yield; a callback's return (a thread switcher, a dialog filter, a Time Manager task: what the
        // callback called is noted call by call, and what it computed counts in its task's turn); a Thread Manager query that
        // changes nothing (Warcraft III's waiting thread polls GetCurrentThread + YieldToAnyThread, measured on the host).
        let yielding = symbol == crate::callback::CALLBACK_RETURN
            || matches!(self.bindings.get(&symbol), Some(Binding::Native(Native::Thread(
                ThreadFn::YieldToAnyThread | ThreadFn::YieldToThread | ThreadFn::GetCurrentThread | ThreadFn::GetThreadState
                | ThreadFn::GetDefaultThreadStackSize | ThreadFn::ThreadCurrentStackSpace))));
        self.note_call(cpu, yielding);
        // a Time Manager task that has come due runs now: this glue call is a safepoint (named limit: not interrupt time)
        if symbol != crate::callback::CALLBACK_RETURN {
            match self.tm_safepoint(cpu, addr) {
                Ok(true) => { self.sync_natives(cpu); return true; }
                Ok(false) => {}
                Err(why) => { self.rt.fault = Some(why); cpu.halt(); return true; }
            }
        }
        if let Some(Binding::Native(Native::Invoke(f))) = self.bindings.get(&symbol).cloned() {
            if let Err(why) = self.begin_invoke(cpu, f) { self.rt.fault = Some(why); cpu.halt(); }
            self.sync_natives(cpu);
            return true;
        }
        if let Some(Binding::Native(n @ (Native::SetJmp | Native::CExit | Native::BreakPoint))) = self.bindings.get(&symbol).cloned() { self.cpu_native(n, cpu); return true; }
        match self.bindings.get(&symbol).cloned() {
            Some(Binding::Native(Native::C(f))) => {
                if let Err(why) = self.c_call(f, cpu) { self.rt.fault = Some(why); cpu.halt(); }
                self.sync_natives(cpu);
                return true;
            }
            Some(Binding::Native(Native::Cf(f))) => {
                if let Err(why) = self.cf_call(f, cpu) { self.rt.fault = Some(why); cpu.halt(); }
                self.sync_natives(cpu);
                return true;
            }
            Some(Binding::Native(Native::Carbon(f))) => {
                if let Err(why) = self.carbon_call(f, cpu, addr) { self.rt.fault = Some(why); cpu.halt(); }
                self.sync_natives(cpu);
                return true;
            }
            Some(Binding::Native(Native::Ae(f))) => {
                if let Err(why) = self.ae_call(f, cpu) { self.rt.fault = Some(why); cpu.halt(); }
                self.sync_natives(cpu);
                return true;
            }
            Some(Binding::Native(Native::Mp(f))) => {
                if let Err(why) = self.mp_call(f, cpu) { self.rt.fault = Some(why); cpu.halt(); }
                self.sync_natives(cpu);
                return true;
            }
            Some(Binding::Native(Native::Thread(f))) => {
                if let Err(why) = self.thread_call(f, cpu) { self.rt.fault = Some(why); cpu.halt(); }
                self.sync_natives(cpu);
                return true;
            }
            Some(Binding::Native(Native::Stdio(f))) => {
                if let Err(why) = self.stdio_call(f, cpu) { self.rt.fault = Some(why); cpu.halt(); }
                self.sync_natives(cpu);
                return true;
            }
            Some(Binding::Native(Native::Math(f))) => {
                if let Err(why) = self.math_call(f, cpu) { self.rt.fault = Some(why); cpu.halt(); }
                self.sync_natives(cpu);
                return true;
            }
            _ => {}
        }
        if symbol == crate::callback::CALLBACK_RETURN {
            if let Err(why) = self.callback_returned(cpu) { self.rt.fault = Some(why); cpu.halt(); }
            self.sync_natives(cpu);
            return true;
        }
        // ModalDialog with user items to draw first: the program's procedures run as callbacks, then the call is entered again
        if matches!(self.bindings.get(&symbol), Some(Binding::Native(Native::ModalDialog))) {
            match self.begin_user_items(cpu, addr) {
                Ok(true) => { self.sync_natives(cpu); return true; }
                Ok(false) => {}
                Err(why) => { self.rt.fault = Some(why); cpu.halt(); return true; }
            }
        }
        let mut regs = cpu.gpr;
        let outcome = self.glue_call(&symbol, &mut regs);
        // a host call changes the result registers only; anything else it touched would be a bug in the adapter
        cpu.gpr[3] = regs[3];
        cpu.gpr[4] = regs[4];
        match outcome {
            GlueOutcome::Return => {}
            GlueOutcome::Exit => cpu.halt(),
            GlueOutcome::Idle => {
                // the call parked: it was not progress after all (note_call counted it before it ran)
                if let (Some(s), Some(p)) = (&mut self.rt.sched.sweep, progressed_before) { s.progressed = p; }
                if !self.idle_yield(cpu, addr) { cpu.block(); cpu.set_pc(addr); }
            }
            GlueOutcome::Refused(why) => { self.rt.fault = Some(why); cpu.halt(); }
        }
        self.sync_natives(cpu);
        true
    }
}
