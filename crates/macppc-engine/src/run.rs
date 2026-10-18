//! Running a PowerPC application: the [`retro_ppc`] interpreter as the CPU, the glue stubs as native hooks.
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
use alloc::format;
use alloc::string::String;
use guestui_abi::GuestUi;
use retro_common::Step;
use retro_guest_mem::GuestMem;
use retro_ppc::{Cpu, Host, RunStats};

/// The pseudo-symbol an init routine returns to.
pub const INIT_DONE: &str = "$InitDone";

/// The CPU and what the run loop tracks around it.
#[derive(Default)]
pub struct Runtime {
    cpu: Option<Cpu>,
    /// Glue cells below this index are already bound as natives in the CPU.
    bound: usize,
    /// Set by a `WaitNextEvent` that parked the CPU: the event mask a queued event must match to resume.
    pub(crate) wait_mask: Option<u16>,
    /// Set by a `ModalDialog` that parked the CPU: it resumes when the front dialog has an item hit.
    pub(crate) wait_dialog: bool,
    /// `main`'s entry state, held while the init routine runs.
    after_init: Option<StartState>,
    main_tvector: u32,
    /// A refusal or CPU trap; once set, every step reports it again (the program cannot continue).
    fault: Option<String>,
}

fn idle() -> RunStats { RunStats { insns: 0, halted: false, blocked: true, trap: None } }

impl<U: GuestUi> MacPpc<U> {
    /// Prepare to run `frag`: `main` (after `init`, when it has one).
    pub fn start(&mut self, frag: &Fragment) -> Result<(), CfmError> {
        let main = self.prepare_main(frag)?;
        self.bindings.insert(INIT_DONE.into(), Binding::Native(Native::InitDone));
        let mut cpu = Cpu::new(main.pc, main.r1);
        cpu.gpr[2] = main.r2;
        cpu.gpr[12] = frag.main.map_or(0, |t| t.address);
        cpu.lr = main.lr;
        self.rt = Runtime { main_tvector: cpu.gpr[12], ..Runtime::default() };
        if let Some(init) = frag.init {
            let id = self.glue.intern(&mut self.mac, "CFM", INIT_DONE)?;
            cpu.pc = init.code;
            cpu.gpr[2] = init.toc;
            cpu.gpr[12] = init.address;
            cpu.lr = self.glue.entry(id).map_or(0, |e| e.stub);
            self.rt.after_init = Some(main);
        }
        self.sync_natives(&mut cpu);
        self.rt.cpu = Some(cpu);
        Ok(())
    }

    /// The CPU while a program is loaded (for inspection; `None` once it exited).
    pub fn cpu(&self) -> Option<&Cpu> { self.rt.cpu.as_ref() }
    /// The refusal or trap that ended the program, if one did.
    pub fn fault(&self) -> Option<&str> { self.rt.fault.as_deref() }

    /// Bind every glue stub created since the last call (a `FindSymbol` interns cells while the program runs).
    fn sync_natives(&mut self, cpu: &mut Cpu) {
        while let Some(e) = self.glue.entry(self.rt.bound) { cpu.bind_native(e.stub); self.rt.bound += 1; }
    }

    /// Run at most `budget` instructions.
    pub fn step(&mut self, budget: u64) -> (Step, RunStats) {
        let gone = RunStats { halted: true, blocked: false, ..idle() };
        if let Some(f) = &self.rt.fault { return (Step::Fault(f.clone()), gone); }
        let Some(mut cpu) = self.rt.cpu.take() else { return (Step::Exited, gone) };
        if let Some(mask) = self.rt.wait_mask {
            self.mac.pump_ui();
            if !self.mac.event_pending(mask) { self.rt.cpu = Some(cpu); return (Step::Blocked, idle()); }
            self.rt.wait_mask = None;
        }
        if self.rt.wait_dialog {
            if !self.mac.modal_ready() { self.rt.cpu = Some(cpu); return (Step::Blocked, idle()); }
            self.rt.wait_dialog = false;
        }
        self.sync_natives(&mut cpu);
        let stats = retro_ppc::resume(self, &mut cpu, budget);
        mac68k_engine::qd::flush(&mut self.mac);
        if let Some(t) = stats.trap {
            let mut why = format!("PowerPC fault: {t}");
            if let retro_ppc::Trap::BusFault { addr, .. } = t {
                if addr >= UNBOUND_DATA && addr - UNBOUND_DATA < 0x1000 && !self.unbound_data.is_empty() {
                    why = format!("{why} -- the placeholder address of an unbound DATA import; unbound data imports of this fragment: {}", self.unbound_data.join(", "));
                }
            }
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
                cpu.halt();
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
            let Some(m) = self.rt.after_init.take() else { self.rt.fault = Some("the init routine returned twice".into()); cpu.halt(); return true };
            // a CFM initialization function returns an OSErr (16 bits); nonzero fails the fragment's preparation, so main never runs
            let status = cpu.gpr[3] as u16 as i16;
            if status != 0 {
                self.rt.fault = Some(format!("the fragment's init routine failed with OSErr {status}: the fragment is not prepared, main does not run"));
                cpu.halt();
                return true;
            }
            cpu.gpr[1] = m.r1;
            cpu.gpr[2] = m.r2;
            cpu.gpr[12] = self.rt.main_tvector;
            cpu.lr = m.lr;
            cpu.set_pc(m.pc);
            return true;
        }
        if let Some(Binding::Native(n @ (Native::SetJmp | Native::CExit | Native::BreakPoint))) = self.bindings.get(&symbol).cloned() { self.cpu_native(n, cpu); return true; }
        let mut regs = cpu.gpr;
        let outcome = self.glue_call(&symbol, &mut regs);
        // a host call changes the result registers only; anything else it touched would be a bug in the adapter
        cpu.gpr[3] = regs[3];
        cpu.gpr[4] = regs[4];
        match outcome {
            GlueOutcome::Return => {}
            GlueOutcome::Exit => cpu.halt(),
            GlueOutcome::Idle => { cpu.block(); cpu.set_pc(addr); }
            GlueOutcome::Refused(why) => { self.rt.fault = Some(why); cpu.halt(); }
        }
        self.sync_natives(cpu);
        true
    }
}
