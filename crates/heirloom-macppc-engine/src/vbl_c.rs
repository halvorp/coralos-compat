//! The Vertical Retrace Manager from PowerPC (`VInstall`, `VRemove`; Inside Macintosh II "The Vertical Retrace Manager", the PowerPC form
//! in Inside Macintosh: PowerPC System Software): the VBLTask queue is the Toolbox's own (heirloom-mac68k-engine `vbl`: the record checks,
//! the 60 Hz count-down). A task that came due runs at the next SAFEPOINT (a glue call made with no callback in progress), as the Time
//! Manager's do (tm_c.rs): its `vblAddr` is a UPP; a PowerPC routine is called with the task in r3 (the PowerPC form of the 68k A0 = task)
//! through the one callback machine, and the interrupted call is entered again afterwards. NAMED LIMITS: not at interrupt time, 1/60 s
//! resolution, no nesting; a 68k task procedure (register-based, A0) is refused by name until the cross-ISA calls build that convention.
use crate::callback::{Continuation, Frame, Saved};
use crate::cpu::Cpu;
use crate::machine::MacPpc;
use alloc::format;
use alloc::string::String;
use heirloom_mac68k_engine::mixed_mode::{resolve, Routine};
use heirloom_ui_abi::GuestUi;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VblFn { VInstall, VRemove }

impl VblFn {
    pub fn by_name(name: &str) -> Option<VblFn> { Some(match name { "VInstall" => VblFn::VInstall, "VRemove" => VblFn::VRemove, _ => return None }) }
}

impl<U: GuestUi> MacPpc<U> {
    pub(crate) fn vbl_call(&mut self, f: VblFn, cpu: &mut Cpu) -> Result<(), String> {
        let task = cpu.gpr[3];
        let r = match f {
            VblFn::VInstall => self.mac.vbl_install(task).map_err(|e| format!("VInstall: {}", e.0))?,
            VblFn::VRemove => self.mac.vbl_remove(task),
        };
        let count = self.mac.mem.read_u16(task + 10).unwrap_or(0);
        self.mac.log.push(format!("[vbl] {f:?} {task:#x} -> {r} (vblCount {count})"));
        cpu.gpr[3] = r as i32 as u32;
        Ok(())
    }

    /// At a safepoint (`call_pc` is the glue call about to run): a due VBL task's procedure runs. `Ok(true)` = the CPU now runs it.
    pub(crate) fn vbl_safepoint(&mut self, cpu: &mut Cpu, call_pc: u32) -> Result<bool, String> {
        if !self.rt.callbacks.is_empty() || !self.mac.vbl_due() { return Ok(false); }
        let Some((task, upp)) = self.mac.vbl_take_due() else { return Ok(false) };
        match resolve(&self.mac.mem, upp).map_err(|e| format!("VBL task {task:#x}: {e}"))? {
            Routine::PowerPc(r) => {
                let saved = Saved::of(cpu, call_pc);
                let sp = cpu.gpr[1];
                self.enter_tvector(cpu, r.proc_descriptor, &[task], sp).map_err(|e| format!("VBL task {task:#x}: {e}"))?;
                self.mac.log.push(format!("[vbl] task {task:#x} due: its PowerPC procedure runs at this safepoint"));
                self.rt.callbacks.push(Frame { saved, cont: Continuation::TimerTask });
                Ok(true)
            }
            Routine::M68k { code, .. } => Err(format!("[HEIRLOOM-MACPPC VBL-68K task={task:#x} code={code:#x}] a 68k VBL procedure (register-based: A0 = the task) is not run from PowerPC yet")),
        }
    }
}
