//! The Vertical Retrace Manager: `VInstall` / `VRemove` and the 60 Hz tick that drives the installed tasks (Inside Macintosh
//! II, "The Vertical Retrace Manager").
//!
//! `VBLTask` is `qLink` 0 (long), `qType` 4 (word, `vType` = 1), `vblAddr` 6 (long, the task's code), `vblCount` 10 (word),
//! `vblPhase` 12 (word). Each retrace (each tick of the host's virtual clock, [`Mac::advance_ticks`]) the Manager decrements the
//! `vblCount` of every installed task whose count is not zero; a task whose count reaches zero is CALLED (`A0` = the task, a
//! plain `RTS` ends it). It runs again only if it stores a new `vblCount` itself.
//!
//! **Where the task runs.** A real VBL interrupts the program between two instructions. Here a task that came due runs at the
//! next Toolbox/OS trap the program executes: the program's registers and SR are saved, the task is entered with the trap's
//! return as its own, and when it returns everything is put back and the trap runs as if nothing had happened (the same
//! machinery the resource decompressor uses). A program that never traps never sees its tasks run; one that does gets them at
//! a granularity of its own trap rate. `vblPhase` is not applied (the task's first period is its `vblCount`); a task is not
//! run from inside another task.
use crate::guestcall::{Frame, TRAMPOLINE};
use crate::machine::Mac;
use crate::osconv::Regs;
use crate::Refusal;
use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

const V_TYPE: u16 = 1;
const VTYP_ERR: i16 = -2;
const Q_TYPE: u32 = 4;
const VBL_ADDR: u32 = 6;
const VBL_COUNT: u32 = 10;

#[derive(Default)]
pub struct Vbl {
    /// The installed tasks (guest addresses of `VBLTask` records), in install order.
    pub queue: Vec<u32>,
    /// Tasks whose count reached zero and have not been run yet.
    due: VecDeque<u32>,
    running: bool,
}

/// What the trampoline restores when a task returns.
#[derive(Debug, Clone)]
pub(crate) struct VblDone { pub(crate) regs: [u32; 15], pub(crate) sr: u32 }

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    m.register_os(0xA033, v_install::<U>)?;
    m.register_os(0xA034, v_remove::<U>)
}

fn v_install<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> Result<String, Refusal> {
    let task = r.a0;
    let e = Refusal::mem("VBLTask");
    let ty = m.mem.read_u16(task + Q_TYPE).map_err(&e)?;
    if ty != V_TYPE { r.err(VTYP_ERR); return Ok(format!("[vbl] VInstall {task:#x}: qType {ty} is not vType (1) -> vTypErr")); }
    if !m.vbl.queue.contains(&task) { m.vbl.queue.push(task); }
    r.err(0);
    let (addr, count) = (m.mem.read_u32(task + VBL_ADDR).map_err(&e)?, m.mem.read_u16(task + VBL_COUNT).map_err(&e)?);
    Ok(format!("[vbl] VInstall {task:#x}: code {addr:#x}, vblCount {count}"))
}

fn v_remove<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> Result<String, Refusal> {
    let task = r.a0;
    let was = m.vbl.queue.iter().position(|&t| t == task);
    if let Some(i) = was { m.vbl.queue.remove(i); }
    m.vbl.due.retain(|&t| t != task);
    // vTypErr when the task was not installed (the book: "if the task is not in the queue, VRemove returns qErr", -1)
    r.err(if was.is_some() { 0 } else { -1 });
    Ok(format!("[vbl] VRemove {task:#x} -> {}", if was.is_some() { "removed" } else { "qErr: it was not installed" }))
}

impl<U: GuestUi> Mac<U> {
    /// One retrace: count every installed task down, queue those that reached zero.
    pub(crate) fn vbl_tick(&mut self) {
        let tasks = self.vbl.queue.clone();
        for t in tasks {
            let Ok(c) = self.mem.read_u16(t + VBL_COUNT) else { continue };
            if c == 0 { continue; }
            let _ = self.mem.write_u16(t + VBL_COUNT, c - 1);
            if c == 1 && !self.vbl.due.contains(&t) { self.vbl.due.push_back(t); }
        }
    }

    /// At a trap boundary: if a task is due, enter it (and run the trap at `retry` afterwards). True = entered.
    pub(crate) fn enter_vbl(&mut self, cpu: &mut Cpu, retry: u32) -> bool {
        if self.vbl.running { return false; }
        while let Some(task) = self.vbl.due.pop_front() {
            if !self.vbl.queue.contains(&task) { continue; }
            let Ok(code) = self.mem.read_u32(task + VBL_ADDR) else { continue };
            let mut regs = [0u32; 15];
            for (i, r) in regs.iter_mut().enumerate() { *r = if i < 8 { cpu.dreg(i as u32) } else { cpu.areg(i as u32 - 8) }; }
            let mut sp = cpu.areg(7);
            for v in [retry, TRAMPOLINE] { sp = sp.wrapping_sub(4); let _ = self.mem.write_u32(sp, v); }
            cpu.set_areg(7, sp);
            cpu.set_areg(0, task);
            self.vbl.running = true;
            self.call_stack.push(Frame::Vbl(VblDone { regs, sr: cpu.sr() }));
            self.log.push(format!("[vbl] task {task:#x} runs (code {code:#x}) before the trap at {retry:#x}"));
            cpu.set_pc(self.mem.strip(code));
            return true;
        }
        false
    }

    pub(crate) fn finish_vbl(&mut self, cpu: &mut Cpu, done: VblDone) {
        for (i, v) in done.regs.iter().enumerate() { if i < 8 { cpu.set_dreg(i as u32, *v) } else { cpu.set_areg(i as u32 - 8, *v) } }
        cpu.set_sr(done.sr);
        self.vbl.running = false;
    }
}
