//! The ARM callback machine (doc rule RETRO-MIXED-MODE-ONE-CALLBACK-MACHINE; the pattern is the Mac PPC engine's
//! `callback.rs` Frame / Saved / Continuation).
//!
//! Native code never runs the CPU itself. To call a guest function, [`Process::call_guest`] pushes a [`Frame`] (a copy of the
//! interrupted CPU and a [`Continuation`] naming what to do with the result), loads r0-r3 and SP, points LR at the dedicated
//! return stub and the PC at the target (a Thumb bit selects Thumb), and returns to the run loop. The run loop executes the
//! guest; when it returns into the stub, `SVC #CALLBACK_SVC` hands control to [`Process::callback_return`], which pops the
//! frame, puts the interrupted CPU back (so the interrupted native call resumes as if it had just returned), and acts on the
//! continuation with the guest's r0. A continuation that wants another guest call simply calls `call_guest` again, so a
//! multi-step sequence (see `appstart`) is a state machine of continuations, never a nest of host calls. Nesting (a callback
//! whose guest code reaches a native that calls guest code) pushes more frames, up to [`MAX_CALLBACK_DEPTH`].
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_cpu_arm::Cpu;
use crate::layout::{PAGE, STUB_BASE};
use crate::loader::Process;
use crate::run::Outcome;

/// The SVC number of the return stub. It lies above every import stub's (`STUB_SVC_BASE + index`) and below `EXIT_SVC`;
/// the run loop tests it before the import range.
pub const CALLBACK_SVC: u32 = 0x00FE_0000;
/// The return stub's address: its own page, directly below the entry's exit stub page. The word there is `SVC #CALLBACK_SVC`.
pub const CALLBACK_STUB: u32 = STUB_BASE - 2 * PAGE;
/// A callback chain deeper than this is refused by name (a runaway guest recursion through natives, not a real program).
pub const MAX_CALLBACK_DEPTH: usize = 64;

/// What to do with the guest's r0 when the callback returns.
#[derive(Debug, Clone)]
pub enum Continuation {
    /// Give r0 to the interrupted native call as its own result: the guest function was the native's whole job.
    Resume,
    /// One step of the application start state machine (`appstart`).
    App(crate::appstart::Step),
    /// `CEikApplication::CreateDocumentL(CApaProcess*)` called the application's private `CreateDocumentL()`; r0 is the new
    /// document, which gets its `iApplication` / `iApaProcess` (see `appstart::finish_create_document`).
    EikCreateDocument { app: u32, process: u32 },
    /// `CAknViewAppUi::AddViewL` called the view's `Id()`; r0 is its UID (see `viewappui::view_added`).
    ViewAdded { appui: u32, view: u32 },
    /// `CleanupStack::PopAndDestroy(n)` ran a destructor / cleanup function; `left` more items to destroy.
    CleanupDestroy { left: u32 },
}

/// One interrupted call: the CPU as it was (resuming at the instruction after the SVC) and what to do on return.
#[derive(Debug, Clone)]
pub struct Frame {
    pub saved: Cpu,
    pub cont: Continuation,
}

/// The machine code of the return stub page: `SVC #CALLBACK_SVC; BX LR` (the BX is never reached).
pub fn return_stub_code() -> [u8; 8] {
    let mut b = [0u8; 8];
    b[..4].copy_from_slice(&(0xEF00_0000 | CALLBACK_SVC).to_le_bytes());
    b[4..].copy_from_slice(&0xE12F_FF1Eu32.to_le_bytes());
    b
}

impl Process {
    /// Start a guest call of `target` (bit 0 = Thumb) with up to four word `args`, to be continued by `cont` when it returns.
    /// Nothing runs here: the CPU is only pointed at the target. Refusals leave the CPU untouched.
    pub fn call_guest(&mut self, target: u32, args: &[u32], cont: Continuation) -> Result<(), String> {
        if args.len() > 4 {
            return Err(format!("callback with {} word arguments: only r0-r3 are passed", args.len()));
        }
        if target & !1 == 0 {
            return Err(String::from("callback to a null function pointer"));
        }
        if self.frames.len() >= MAX_CALLBACK_DEPTH {
            return Err(format!("callback nesting deeper than {MAX_CALLBACK_DEPTH}"));
        }
        self.frames.push(Frame { saved: self.cpu.clone(), cont });
        for (i, a) in args.iter().enumerate() {
            self.cpu.regs[i] = *a;
        }
        // The interrupted code's SP is valid and everything below it is free; AAPCS wants 8-byte alignment at a call.
        self.cpu.regs[13] &= !7;
        self.cpu.regs[14] = CALLBACK_STUB;
        self.cpu.set_pc_interworking(target);
        Ok(())
    }

    /// The guest returned into the return stub: pop the frame, restore the interrupted CPU, act on the continuation.
    /// `Err` ends the run with that outcome.
    pub(crate) fn callback_return(&mut self) -> Result<(), Outcome> {
        let Some(frame) = self.frames.pop() else {
            return Err(Outcome::Fault(String::from("callback return stub reached with no callback frame")));
        };
        let r0 = self.cpu.regs[0];
        self.cpu = frame.saved;
        match frame.cont {
            Continuation::Resume => {
                self.cpu.regs[0] = r0;
                Ok(())
            }
            Continuation::App(step) => crate::appstart::step(self, step, r0),
            Continuation::EikCreateDocument { app, process } => crate::appstart::finish_create_document(self, app, process, r0),
            Continuation::CleanupDestroy { left } => crate::cleanup::destroy_next(self, left),
            Continuation::ViewAdded { appui, view } => crate::viewappui::view_added(self, appui, view, r0),
        }
    }

    /// The number of guest calls currently in flight (frames on the callback stack).
    pub fn callback_depth(&self) -> usize {
        self.frames.len()
    }
}

/// A stack of frames, for `Process`.
pub type Frames = Vec<Frame>;
