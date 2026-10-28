//! The Dialog Manager's user items (Inside Macintosh I, "The Dialog Manager": `SetDialogItem`, `DrawDialog`, `UpdtDialog`,
//! `DialogSelect`, `ModalDialog`). A user item (type 0, the disabled bit aside) is the program's own drawing: its item handle is
//! the address of a PROCEDURE `UserItem(theWindow: WindowPtr; itemNo: INTEGER)` that the Dialog Manager calls whenever it draws
//! the dialog, with `thePort` set to the dialog, in item order. That procedure is guest code, so a draw is a RUN: the engine
//! calls the procedures one after the other through the completion trampoline ([`crate::guestcall`]) and puts every register,
//! the status register and `thePort` back afterwards. What the procedure draws is the program's own imagery, drawn with
//! QuickDraw into the dialog's content plane; the dialog's controls and text items stay native parts and are never redrawn here.
//!
//! Where a run starts:
//! * `DrawDialog(d)` draws every user item; `UpdtDialog(d, rgn)` only the items whose box meets `rgn` (local coordinates).
//! * `DialogSelect` on an update event for a dialog is `BeginUpdate`, `UpdtDialog` over the update region, `EndUpdate`, and
//!   answers FALSE.
//! * `ModalDialog` does the same drawing for a dialog whose update region is not empty (a fresh dialog; a program that drew
//!   it already still gets the update the window system would send). The run happens BEFORE the wait, and the trap word then
//!   runs again with its arguments still on the stack, so the wait itself is unchanged.
//!
//! From PowerPC code, `ModalDialog`'s draw runs through heirloom-macppc-engine's callback machine ([`Mac::begin_modal_user_items`] /
//! [`Mac::end_user_items`]: each procedure is a UPP called as PowerPC code). `DrawDialog`, `UpdtDialog` and `DialogSelect`'s update
//! draw from PowerPC code are still refused by name (owner: heirloom-macppc-engine; delete-when each has a PowerPC witness).
use super::dialogs::Item;
use crate::guestcall::{Frame, TRAMPOLINE};
use crate::machine::{Call, Done, Mac};
use crate::pascal::Arg;
use crate::qd::update::{begin_update_of, end_update_of};
use crate::Refusal;
use alloc::collections::VecDeque;
use alloc::format;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;
use heirloom_raster::Region;

/// The program's state while the procedures run.
#[derive(Debug, Clone)]
pub(super) struct Saved { regs: [u32; 15], sr: u32, pc: u32, sp: u32, port: u32 }

impl Saved {
    /// The program's state at a trap that is about to run guest code: resume at `pc` with `sp`. Returns it, the registers and `thePort`.
    pub(super) fn capture<U: GuestUi>(m: &Mac<U>, cpu: &Cpu, pc: u32, sp: u32) -> (Saved, [u32; 15], u32) {
        let mut regs = [0u32; 15];
        for (i, r) in regs.iter_mut().enumerate() { *r = if i < 8 { cpu.dreg(i as u32) } else { cpu.areg(i as u32 - 8) }; }
        let port = if m.qd_globals != 0 { m.mem.read_u32(m.qd_globals).unwrap_or(0) } else { 0 };
        (Saved { regs, sr: cpu.sr(), pc, sp, port }, regs, port)
    }

    /// Put the registers, the status register and `thePort` back; the caller sets `sp` and `pc` as the outcome needs.
    pub(super) fn restore<U: GuestUi>(&self, m: &mut Mac<U>, cpu: &mut Cpu, regs: &[u32; 15], port: u32) {
        if m.qd_globals != 0 { let _ = m.mem.write_u32(m.qd_globals, port); }
        for (i, v) in regs.iter().enumerate() { if i < 8 { cpu.set_dreg(i as u32, *v) } else { cpu.set_areg(i as u32 - 8, *v) } }
        cpu.set_sr(self.sr);
        cpu.set_areg(7, self.sp);
        cpu.set_pc(self.pc);
    }
}

/// One round of user-item drawing, waiting to start or running.
#[derive(Debug, Clone)]
pub(crate) struct Run {
    win: u32,
    /// (procedure address, item number), in the order they are called.
    todo: VecDeque<(u32, i16)>,
    /// `EndUpdate` the window when the last procedure has returned (the run is the body of a `BeginUpdate`).
    end_update: bool,
    /// Resume at the trap word again with its arguments (pc, sp) instead of after the trap.
    retry: Option<(u32, u32)>,
    saved: Option<Saved>,
}

impl<U: GuestUi> Mac<U> {
    /// (procedure, item number) of the user items of `win` whose box meets `clip` (all of them without one), in item order.
    fn user_items(&self, win: u32, clip: Option<&Region>) -> VecDeque<(u32, i16)> {
        let Some(d) = self.tr.dlg.dialogs.get(&win) else { return VecDeque::new() };
        d.list.iter().enumerate()
            .filter(|(_, s)| s.item == Item::User && s.handle != 0)
            .filter(|(_, s)| clip.is_none_or(|g| g.rects().iter().any(|r| !r.intersect(&crate::qd::port::raster(s.rect)).is_empty())))
            .map(|(i, s)| (self.mem.strip(s.handle), i as i16 + 1))
            .collect()
    }

    /// Queue `run` for the A-line bridge, which starts it once the trap has finished (the stack and registers are final then).
    fn schedule_user_items(&mut self, cpu: &Cpu, name: &str, run: Run) -> Result<(), Refusal> {
        let _ = cpu.pc(); // reading a register marks a CPU-less caller (the PowerPC glue)
        if cpu.touched_registers() { return Err(Refusal(format!("{name}: the dialog has user items, whose draw procedures are 68k code the PowerPC glue cannot run from here"))); }
        self.pending_user_items = Some(run);
        Ok(())
    }

    /// The bridge finished a trap that scheduled a run: start it.
    pub(crate) fn enter_user_items(&mut self, cpu: &mut Cpu, mut run: Run) {
        let mut regs = [0u32; 15];
        for (i, r) in regs.iter_mut().enumerate() { *r = if i < 8 { cpu.dreg(i as u32) } else { cpu.areg(i as u32 - 8) }; }
        let (pc, sp) = run.retry.unwrap_or_else(|| (cpu.pc(), cpu.areg(7)));
        let port = if self.qd_globals != 0 { self.mem.read_u32(self.qd_globals).unwrap_or(0) } else { 0 };
        run.saved = Some(Saved { regs, sr: cpu.sr(), pc, sp, port });
        if let Err(Refusal(e)) = crate::quickdraw::set_the_port(self, run.win) { self.note_refusal("DrawDialog", e); }
        self.next_user_item(cpu, run);
    }

    /// A procedure returned (or the run starts): call the next one, or finish.
    pub(crate) fn next_user_item(&mut self, cpu: &mut Cpu, mut run: Run) {
        let Some(s) = run.saved.clone() else { return };
        let Some((proc, item)) = run.todo.pop_front() else { return self.finish_user_items(cpu, run, s) };
        // Pascal: the window (a long) and the item number (a word) are pushed, the procedure pops them and returns to the trampoline.
        let mut sp = s.sp;
        let push = |m: &mut Self, sp: &mut u32, v: u32, long: bool| { *sp = sp.wrapping_sub(if long { 4 } else { 2 }); let _ = if long { m.mem.write_u32(*sp, v) } else { m.mem.write_u16(*sp, v as u16) }; };
        push(self, &mut sp, run.win, true);
        push(self, &mut sp, item as u16 as u32, false);
        push(self, &mut sp, TRAMPOLINE, true);
        cpu.set_areg(7, sp);
        self.log.push(format!("[dialog] user item {item} of {:#x}: the program's draw procedure {proc:#x} runs", run.win));
        cpu.set_pc(proc);
        self.call_stack.push(Frame::UserItems(run));
    }

    fn finish_user_items(&mut self, cpu: &mut Cpu, run: Run, s: Saved) {
        if run.end_update { if let Err(Refusal(e)) = end_update_of(self, run.win) { self.note_refusal("EndUpdate", e); } }
        if self.qd_globals != 0 { let _ = self.mem.write_u32(self.qd_globals, s.port); }
        for (i, v) in s.regs.iter().enumerate() { if i < 8 { cpu.set_dreg(i as u32, *v) } else { cpu.set_areg(i as u32 - 8, *v) } }
        cpu.set_sr(s.sr);
        cpu.set_areg(7, s.sp);
        cpu.set_pc(s.pc);
    }

    /// `DialogSelect` on an update event for dialog `win`: `BeginUpdate`, the user items over the update region, `EndUpdate`.
    pub(crate) fn update_dialog_event(&mut self, cpu: &Cpu, win: u32) -> Result<(), Refusal> {
        let region = begin_update_of(self, win)?;
        let todo = self.user_items(win, Some(&region));
        if todo.is_empty() { return end_update_of(self, win); }
        self.schedule_user_items(cpu, "DialogSelect", Run { win, todo, end_update: true, retry: None, saved: None })
            .inspect_err(|_| { let _ = end_update_of(self, win); })
    }

    /// `ModalDialog` is about to wait: a dialog whose update region is not empty is drawn first. True = a run was scheduled (the
    /// trap word runs again afterwards).
    pub(crate) fn update_modal_dialog(&mut self, cpu: &Cpu, win: u32) -> Result<bool, Refusal> {
        if self.user_items(win, None).is_empty() || self.qd.update.get(&win).is_none_or(|g| g.is_empty()) { return Ok(false); }
        let trap_pc = self.trap_pc;
        if crate::traptable::in_stub(trap_pc) { return Err(Refusal("ModalDialog: reached through a GetTrapAddress stub, it cannot be run again after the dialog's user items are drawn".into())); }
        let region = begin_update_of(self, win)?;
        let todo = self.user_items(win, Some(&region));
        if todo.is_empty() { end_update_of(self, win)?; return Ok(false); }
        self.schedule_user_items(cpu, "ModalDialog", Run { win, todo, end_update: true, retry: Some((trap_pc, cpu.areg(7))), saved: None })
            .inspect_err(|_| { let _ = end_update_of(self, win); })?;
        Ok(true)
    }
}

/// A user-item draw a PowerPC caller runs itself: there is no 68k CPU for the A-line bridge to schedule on, so the PowerPC
/// adapter (heirloom-macppc-engine `callback`) calls each procedure through Mixed Mode and hands the draw back to [`Mac::end_user_items`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserItemDraw {
    pub win: u32,
    /// (the item's procedure: a UPP for a PowerPC program, item number), in item order.
    pub todo: VecDeque<(u32, i16)>,
    /// `thePort` before the draw, put back by [`Mac::end_user_items`].
    pub port: u32,
}

impl<U: GuestUi> Mac<U> {
    /// `ModalDialog`'s first step for a PowerPC caller (the 68k form is [`Mac::update_modal_dialog`]): when the front dialog has
    /// user items and a non-empty update region, `BeginUpdate` it, make it the port and return the procedures to call; `None`
    /// when there is nothing to draw. The caller then runs `ModalDialog` again: the update region is empty by then, so the
    /// second call goes straight to the wait (nothing the first call did is repeated).
    pub fn begin_modal_user_items(&mut self) -> Result<Option<UserItemDraw>, Refusal> {
        let Some(win) = self.front_dialog() else { return Ok(None) };
        if self.user_items(win, None).is_empty() || self.qd.update.get(&win).is_none_or(|g| g.is_empty()) { return Ok(None); }
        let region = begin_update_of(self, win)?;
        let todo = self.user_items(win, Some(&region));
        if todo.is_empty() { end_update_of(self, win)?; return Ok(None); }
        let port = if self.qd_globals != 0 { self.mem.read_u32(self.qd_globals).unwrap_or(0) } else { 0 };
        if let Err(Refusal(e)) = crate::quickdraw::set_the_port(self, win) { self.note_refusal("ModalDialog", e); }
        self.log.push(format!("[dialog] ModalDialog {win:#x}: {} user item(s) drawn by the program's procedures first", todo.len()));
        Ok(Some(UserItemDraw { win, todo, port }))
    }

    /// The last procedure of a [`UserItemDraw`] returned: `EndUpdate` and put `thePort` back.
    pub fn end_user_items(&mut self, d: &UserItemDraw) {
        if let Err(Refusal(e)) = end_update_of(self, d.win) { self.note_refusal("EndUpdate", e); }
        if self.qd_globals != 0 { let _ = self.mem.write_u32(self.qd_globals, d.port); }
    }
}

/// `DrawDialog(theDialog)` / `UpdtDialog(theDialog, updateRgn)`: the controls and text items are the host's native parts, already
/// shown and kept current as the program changes them; the user items are drawn by the program's own procedures (see the module).
pub(super) fn draw_dialog<U: GuestUi>(m: &mut Mac<U>, cpu: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let win = c.args.get(0);
    if !m.tr.dlg.dialogs.contains_key(&win) { return Err(Refusal(format!("{}: {win:#x} is not a dialog", c.name))); }
    let clip = if c.args.len() > 1 { Some(m.read_region(c.args.get(1))?) } else { None };
    let todo = m.user_items(win, clip.as_ref());
    let n = todo.len();
    if n > 0 { m.schedule_user_items(cpu, c.name, Run { win, todo, end_update: false, retry: None, saved: None })?; }
    Ok(Done { result: 0, log: format!("[dialog] {} {win:#x} ({n} user item(s) drawn by the program's procedures; the other items are native parts, already current)", c.name) })
}

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    use Arg::L;
    m.register(0xA981, &[L], None, draw_dialog::<U>)?;
    m.register(0xA978, &[L, L], None, draw_dialog::<U>)
}

