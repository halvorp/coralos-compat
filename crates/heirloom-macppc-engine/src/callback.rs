//! Mixed Mode slice 3: the Toolbox calling PowerPC code back (docs/MACPPC-PLUMBING.md, "Mixed Mode"; the ruling in
//! docs/consults/2026-10-20-mixed-mode.md). No handler re-enters the interpreter: the adapter saves the interrupted call's
//! registers in a [`Frame`], points the CPU at the callback with LR = the `$CallbackReturn` pseudo-glue, and lets the run loop
//! execute it. When the callback returns to that glue, the frame's typed [`Continuation`] decides what happens next: call the
//! next procedure, or finish and resume the interrupted call. Frames form a stack, so a callback that itself makes a Toolbox
//! call that calls back nests; a return with no frame is a fault, never a jump to nowhere.
//!
//! The one continuation today is a dialog's user items, drawn before `ModalDialog` waits: each item's procedure is a UPP
//! (`NewRoutineDescriptor` of a TVector), resolved through the one descriptor parser and called as a PowerPC routine
//! (r3 = the dialog, r4 = the item number). After the last one, `EndUpdate`, `thePort` back, and the ModalDialog call is
//! entered AGAIN with its saved registers: its update region is empty then, so it goes straight to its wait.
use crate::machine::MacPpc;
use crate::symbols::{Binding, Native};
use alloc::format;
use alloc::string::String;
use heirloom_ui_abi::GuestUi;
use heirloom_mac68k_engine::mixed_mode::{parse, ppc_record};
use heirloom_mac68k_engine::translate::useritems::UserItemDraw;
use heirloom_cpu_ppc::Cpu;

/// The pseudo-symbol a callback returns to.
pub const CALLBACK_RETURN: &str = "$CallbackReturn";

/// Bytes below the interrupted call's stack pointer the callback's frame starts at: past the ABI's 224-byte red zone, so
/// nothing the interrupted code may keep there is overwritten, and 16-byte aligned.
const CALLBACK_STACK_GAP: u32 = 512;

/// The interrupted call's registers, put back exactly when the continuation resumes it.
#[derive(Debug, Clone)]
pub(crate) struct Saved { pub(crate) gpr: [u32; 32], fpr: [u64; 32], cr: u32, lr: u32, ctr: u32, xer: u32, fpscr: u32, pc: u32 }

impl Saved {
    pub(crate) fn of(cpu: &Cpu, pc: u32) -> Self { Saved { gpr: cpu.gpr, fpr: cpu.fpr, cr: cpu.cr, lr: cpu.lr, ctr: cpu.ctr, xer: cpu.xer, fpscr: cpu.fpscr, pc } }
    pub(crate) fn restore(&self, cpu: &mut Cpu) {
        cpu.gpr = self.gpr; cpu.fpr = self.fpr; cpu.cr = self.cr; cpu.lr = self.lr; cpu.ctr = self.ctr; cpu.xer = self.xer; cpu.fpscr = self.fpscr;
        cpu.set_pc(self.pc);
    }
}

/// What to do when a callback returns. A variant is added only with a caller that runs it.
#[derive(Debug, Clone)]
pub(crate) enum Continuation {
    /// A dialog's user items before `ModalDialog` waits: the draw (window, procedures left, the port to restore).
    UserItems(UserItemDraw),
    /// `qsort` in progress ([`crate::callback_c`]): the comparison procedure is the program's.
    Qsort(crate::callback_c::Sort),
    /// `bsearch` in progress.
    Bsearch(crate::callback_c::Search),
    /// `exit` running the `atexit` handlers, last registered first; the run ends after the last.
    AtExit,
    /// A Carbon event being offered to the program's handlers ([`crate::carbon`]).
    Carbon(crate::carbon::Dispatch),
    /// A thread's terminator ran; now switch to this thread and drop the ended one.
    ThreadEnd(u32),
    /// The leaving thread's out-switcher ran: now the switch (to this task, the leaving one saved as `state`, resuming at `pc`).
    SwitchOut { to: u32, state: Option<crate::sched::TState>, pc: u32 },
    /// The new thread's in-switcher ran: its registers come back and it continues.
    SwitchIn,
    /// An Apple Event handler ran for `AEProcessAppleEvent`: its OSErr is the call's result.
    AeHandler,
    /// A Time Manager task's procedure ran at a safepoint: the interrupted glue call is entered again ([`crate::tm_c`]).
    TimerTask,
    /// A File Manager completion routine ran at a safepoint: the interrupted glue call is entered again ([`crate::pb`]).
    IoCompletion,
    /// A program's UPP called through an `Invoke<Name>UPP` ([`crate::invoke_c`]): the result goes back to the caller.
    Invoke(crate::invoke_c::Result_),
}

#[derive(Debug, Clone)]
pub(crate) struct Frame { pub(crate) saved: Saved, pub(crate) cont: Continuation }

impl<U: GuestUi> MacPpc<U> {
    /// The `$CallbackReturn` stub address, interned (and bound as a native) on first use.
    fn callback_return_stub(&mut self, cpu: &mut Cpu) -> Result<u32, String> {
        self.bindings.insert(CALLBACK_RETURN.into(), Binding::Native(Native::CallbackReturn));
        let id = self.glue.intern(&mut self.mac, "CFM", CALLBACK_RETURN).map_err(|e| format!("{e:?}"))?;
        self.sync_natives(cpu);
        self.glue.entry(id).map(|e| e.stub).ok_or_else(|| String::from("the $CallbackReturn glue cell is missing"))
    }

    /// Call the PowerPC routine behind `upp` with `args` in r3.., returning to `$CallbackReturn`. The UPP is resolved (one
    /// parser) BEFORE any register changes: a descriptor this engine cannot run is a named refusal, the CPU untouched.
    pub(crate) fn enter_upp(&mut self, cpu: &mut Cpu, upp: u32, args: &[u32], sp: u32) -> Result<(), String> {
        let record = parse(&self.mac.mem, upp).and_then(|d| ppc_record(&d)).map_err(|e| format!("{e}"))?;
        self.enter_tvector(cpu, record.proc_descriptor, args, sp).map_err(|e| format!("UPP {upp:#x}: {e}"))
    }

    /// Call the PowerPC routine whose TVector is at `tv` (a CFM function pointer) with `args` in r3.., returning to `$CallbackReturn`.
    pub(crate) fn enter_tvector(&mut self, cpu: &mut Cpu, tv: u32, args: &[u32], sp: u32) -> Result<(), String> {
        let code = self.mac.mem.read_u32(tv).map_err(|e| format!("the TVector at {tv:#x}: {e}"))?;
        let toc = self.mac.mem.read_u32(tv + 4).map_err(|e| format!("the TVector at {tv:#x}: {e}"))?;
        let ret = self.callback_return_stub(cpu)?;
        let r1 = (sp.wrapping_sub(CALLBACK_STACK_GAP)) & !0xF;
        self.mac.mem.write_u32(r1, sp).map_err(|e| format!("the callback frame at {r1:#x}: {e}"))?; // back chain
        for (i, a) in args.iter().enumerate() { cpu.gpr[3 + i] = *a; }
        cpu.gpr[1] = r1;
        cpu.gpr[2] = toc;
        cpu.gpr[12] = tv;
        cpu.lr = ret;
        cpu.set_pc(code);
        Ok(())
    }

    /// `ModalDialog` reached with user items to draw first: save the call, run the first procedure. False = nothing to draw.
    pub(crate) fn begin_user_items(&mut self, cpu: &mut Cpu, call_pc: u32) -> Result<bool, String> {
        let Some(draw) = self.mac.begin_modal_user_items().map_err(|e| e.0)? else { return Ok(false) };
        let saved = Saved::of(cpu, call_pc);
        self.next_user_item(cpu, Frame { saved, cont: Continuation::UserItems(draw) })?;
        Ok(true)
    }

    /// Call the frame's next user item, or finish: `EndUpdate`, the port back, the interrupted call entered again.
    fn next_user_item(&mut self, cpu: &mut Cpu, mut frame: Frame) -> Result<(), String> {
        let Continuation::UserItems(draw) = &mut frame.cont else { return Err(String::from("a user-item step on another continuation")) };
        match draw.todo.pop_front() {
            Some((upp, item)) => {
                let win = draw.win;
                self.mac.log.push(format!("[mixedmode] user item {item} of {win:#x}: the program's procedure (UPP {upp:#x}) runs"));
                let sp = frame.saved.gpr[1];
                self.enter_upp(cpu, upp, &[win, item as i32 as u32], sp)
                    .map_err(|e| { self.mac.end_user_items(draw); format!("ModalDialog: user item {item}: {e}") })?;
                self.rt.callbacks.push(frame);
                Ok(())
            }
            None => {
                self.mac.end_user_items(draw);
                frame.saved.restore(cpu);
                Ok(())
            }
        }
    }

    /// A callback returned to `$CallbackReturn`.
    pub(crate) fn callback_returned(&mut self, cpu: &mut Cpu) -> Result<(), String> {
        let Some(frame) = self.rt.callbacks.pop() else { return Err(String::from("$CallbackReturn reached with no callback in progress")) };
        match frame.cont {
            Continuation::UserItems(_) => self.next_user_item(cpu, frame),
            Continuation::Qsort(_) => self.sort_returned(cpu, frame),
            Continuation::Bsearch(_) => self.search_returned(cpu, frame),
            Continuation::AtExit => self.exit_handler_returned(cpu, frame),
            Continuation::Carbon(_) => self.dispatch_returned(cpu, frame),
            Continuation::ThreadEnd(to) => self.switch_to(cpu, to, None, 0),
            Continuation::SwitchOut { to, state, pc } => { frame.saved.restore(cpu); self.finish_switch(cpu, to, state, pc) }
            Continuation::AeHandler => self.ae_handler_returned(cpu, frame),
            Continuation::SwitchIn => { frame.saved.restore(cpu); Ok(()) }
            Continuation::TimerTask | Continuation::IoCompletion => { frame.saved.restore(cpu); Ok(()) }
            Continuation::Invoke(res) => { self.invoke_returned(cpu, frame, res); Ok(()) }
        }
    }

    /// Callbacks in progress (for tests and the census).
    pub fn callback_depth(&self) -> usize { self.rt.callbacks.len() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::PpcHost;
    use heirloom_ui_abi::RecordingGuestUi;
    use heirloom_mac68k_engine::mixed_mode::{new_routine_descriptor, ISA_M68K, ISA_PPC};

    type M = MacPpc<RecordingGuestUi>;
    const TV: u32 = 0x0003_0000;
    const CODE: u32 = 0x0003_1000;
    const TOC: u32 = 0x0003_8000;
    const SP: u32 = 0x0004_0000;

    fn setup() -> (M, Cpu) {
        let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
        m.mac.mem.write_u32(TV, CODE).unwrap();
        m.mac.mem.write_u32(TV + 4, TOC).unwrap();
        let mut cpu = Cpu::new(0x0005_0000, SP);
        cpu.gpr[2] = 0x0006_0000; // the interrupted code's own TOC
        cpu.lr = 0x0005_1234;
        (m, cpu)
    }

    #[test]
    fn a_powerpc_callback_is_entered_with_its_toc_its_tvector_its_arguments_and_the_return_glue() {
        let (mut m, mut cpu) = setup();
        let upp = new_routine_descriptor(&mut m.mac, TV, 0x2C0, ISA_PPC).unwrap();
        m.enter_upp(&mut cpu, upp, &[0x0024_5210, 2], SP).unwrap();
        assert_eq!((cpu.pc, cpu.gpr[2], cpu.gpr[12], cpu.gpr[3], cpu.gpr[4]), (CODE, TOC, TV, 0x0024_5210, 2));
        assert_eq!(m.glue_symbol_at(cpu.lr).as_deref(), Some(CALLBACK_RETURN), "LR is the callback-return glue");
        let r1 = cpu.gpr[1];
        assert!(r1 % 16 == 0 && r1 <= SP - CALLBACK_STACK_GAP, "a fresh, aligned frame past the red zone: {r1:#x}");
        assert_eq!(m.mac.mem.read_u32(r1).unwrap(), SP, "its back chain is the interrupted stack pointer");
    }

    #[test]
    fn a_descriptor_this_engine_cannot_run_is_refused_before_any_register_changes() {
        let (mut m, mut cpu) = setup();
        let before = (cpu.pc, cpu.gpr, cpu.lr);
        let only68k = new_routine_descriptor(&mut m.mac, 0x0001_2000, 0x2C0, ISA_M68K).unwrap();
        let e = m.enter_upp(&mut cpu, only68k, &[1, 2], SP).unwrap_err();
        assert!(e.contains("68k code only"), "{e}");
        // NEGATIVE CONTROLS: a raw pointer (not a descriptor) is refused the same way, never run as a TVector
        let e = m.enter_upp(&mut cpu, TV, &[1, 2], SP).unwrap_err();
        assert!(e.contains("not a routine descriptor"), "{e}");
        assert_eq!((cpu.pc, cpu.gpr, cpu.lr), before, "the CPU is untouched by a refusal");
    }

    #[test]
    fn a_return_with_no_callback_in_progress_is_refused() {
        let (mut m, mut cpu) = setup();
        assert_eq!(m.callback_returned(&mut cpu).unwrap_err(), "$CallbackReturn reached with no callback in progress");
    }

    #[test]
    fn finishing_a_draw_puts_back_every_register_of_the_interrupted_call_and_enters_it_again() {
        let (mut m, mut cpu) = setup();
        let call_pc = 0x0007_0000; // the ModalDialog glue stub
        cpu.gpr[31] = 0xDEAD_BEEF;
        cpu.fpr[14] = 42;
        cpu.cr = 0x2000_0000;
        let saved = Saved::of(&cpu, call_pc);
        let frame = Frame { saved, cont: Continuation::UserItems(UserItemDraw { win: 0, todo: Default::default(), port: 0 }) };
        // the callback clobbered everything it may
        cpu.gpr = [7; 32];
        cpu.fpr = [7; 32];
        cpu.cr = 0;
        cpu.lr = 0;
        m.rt.callbacks.push(frame);
        m.callback_returned(&mut cpu).unwrap();
        assert_eq!((cpu.pc, cpu.gpr[1], cpu.gpr[2], cpu.gpr[31], cpu.fpr[14], cpu.cr, cpu.lr), (call_pc, SP, 0x0006_0000, 0xDEAD_BEEF, 42, 0x2000_0000, 0x0005_1234));
        assert_eq!(m.callback_depth(), 0);
    }
}
