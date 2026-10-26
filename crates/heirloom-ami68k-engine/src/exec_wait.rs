//! exec.library messages and waiting: PutMsg / GetMsg / ReplyMsg / Signal / Wait / WaitPort, over real Exec lists in
//! guest memory (exec/lists.h: lh_Head 0, lh_Tail 4, lh_TailPred 8; a Node's ln_Succ 0, ln_Pred 4).
//!
//! SUSPENSION (docs/SLICE-2-ENGINE.md, Astra 2026-10-17 option a): Wait / WaitPort with nothing pending record what
//! they wait for and `cpu.block()`; the host's next `Amiga::step` re-checks the predicate first and resumes the guest
//! only when it holds, with D0 set to the call's result (the matched signals, the port's head message). No guest
//! instruction runs while a wait is unsatisfied.
use alloc::string::String;
use alloc::format;
use crate::{machine::Amiga, Refusal};
use heirloom_ui_abi::GuestUi;
use heirloom_cpu_68k::Cpu;

pub const LVO_SIGNAL: i32 = -324;
pub const LVO_WAIT: i32 = -318;
pub const LVO_PUT_MSG: i32 = -366;
pub const LVO_GET_MSG: i32 = -372;
pub const LVO_REPLY_MSG: i32 = -378;
pub const LVO_WAIT_PORT: i32 = -384;

const TC_SIG_RECVD: u32 = 26;
const TC_SIG_ALLOC: u32 = 18;
const MP_SIGBIT: u32 = 15;
const MP_FLAGS: u32 = 14;
const MP_SIGTASK: u32 = 16;
const PA_SIGNAL: u8 = 0;
const PA_IGNORE: u8 = 2;
const MP_MSGLIST: u32 = 20;
const MN_REPLYPORT: u32 = 14;
const LN_TYPE: u32 = 8;
const NT_MESSAGE: u8 = 5;
const NT_REPLYMSG: u8 = 7;

/// What a blocked task is waiting for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waiting {
    /// Wait(set).
    Signals(u32),
    /// WaitPort(port): the port's first message.
    Port(u32),
    /// DoIO / WaitIO on a request still in flight: done when it is replied; `remove` takes the reply off its port.
    Io { io: u32, remove: bool },
    /// Delay(): until the virtual clock reaches this many microseconds.
    Until(u64),
    /// ObtainSemaphore on one another task holds: done when it is free (and then it is the task's).
    Sem { ss: u32, shared: bool },
    /// EasyRequestArgs / AutoRequest / SysReqHandler(WaitInput) on a requester the host has not answered yet (requester.rs).
    Request(u32),
}

fn m(what: &'static str) -> impl Fn(heirloom_mem_flat32::MemError) -> Refusal { Refusal::mem(what) }

pub fn sig_recvd<U: GuestUi>(a: &Amiga<U>, task: u32) -> Result<u32, Refusal> { a.mem.read_u32(task + TC_SIG_RECVD).map_err(m("tc_SigRecvd")) }
fn set_sig_recvd<U: GuestUi>(a: &mut Amiga<U>, task: u32, v: u32) -> Result<(), Refusal> { a.mem.write_u32(task + TC_SIG_RECVD, v).map_err(m("tc_SigRecvd")) }

/// Raise `sigs` on `task` and wake it if that satisfies its wait. A `task` the scheduler does not know is a named
/// refusal (a port whose mp_SigTask died, or a made-up address), never a write into arbitrary memory.
pub fn signal_task<U: GuestUi>(a: &mut Amiga<U>, task: u32, sigs: u32) -> Result<(), Refusal> {
    if !a.known_task(task) { return Err(Refusal(format!("Signal: {task:#x} is not a task"))); }
    let r = sig_recvd(a, task)?;
    set_sig_recvd(a, task, r | sigs)?;
    a.wake(task)
}

/// AddTail(list, node).
pub fn add_tail<U: GuestUi>(a: &mut Amiga<U>, list: u32, node: u32) -> Result<(), Refusal> {
    let pred = a.mem.read_u32(list + 8).map_err(m("lh_TailPred"))?;
    a.mem.write_u32(node, list + 4).map_err(m("ln_Succ"))?;
    a.mem.write_u32(node + 4, pred).map_err(m("ln_Pred"))?;
    a.mem.write_u32(pred, node).map_err(m("pred.ln_Succ"))?;
    a.mem.write_u32(list + 8, node).map_err(m("lh_TailPred"))
}

/// RemHead(list) -> the first node, or 0.
pub fn rem_head<U: GuestUi>(a: &mut Amiga<U>, list: u32) -> Result<u32, Refusal> {
    let head = a.mem.read_u32(list).map_err(m("lh_Head"))?;
    let next = a.mem.read_u32(head).map_err(m("ln_Succ"))?;
    if next == 0 { return Ok(0); } // head is the lh_Tail sentinel: empty
    a.mem.write_u32(list, next).map_err(m("lh_Head"))?;
    a.mem.write_u32(next + 4, list).map_err(m("ln_Pred"))?;
    Ok(head)
}

/// Remove(node) from whatever list it is on.
pub fn remove<U: GuestUi>(a: &mut Amiga<U>, node: u32) -> Result<(), Refusal> {
    let (succ, pred) = (a.mem.read_u32(node).map_err(m("ln_Succ"))?, a.mem.read_u32(node + 4).map_err(m("ln_Pred"))?);
    a.mem.write_u32(pred, succ).map_err(m("pred.ln_Succ"))?;
    a.mem.write_u32(succ + 4, pred).map_err(m("succ.ln_Pred"))
}

pub fn port_head<U: GuestUi>(a: &Amiga<U>, port: u32) -> Result<u32, Refusal> {
    let head = a.mem.read_u32(port + MP_MSGLIST).map_err(m("lh_Head"))?;
    Ok(if head == port + MP_MSGLIST + 4 { 0 } else { head })
}

/// PutMsg semantics: queue `msg` on `port` and signal the port's task (PA_SIGNAL: the only kind made here).
pub fn put_msg<U: GuestUi>(a: &mut Amiga<U>, port: u32, msg: u32, ty: u8) -> Result<(), Refusal> {
    let (flags, task) = (a.mem.read_u8(port + MP_FLAGS).map_err(m("mp_Flags"))? & 3, a.mem.read_u32(port + MP_SIGTASK).map_err(m("mp_SigTask"))?);
    match flags {
        PA_SIGNAL if !a.known_task(task) => return Err(Refusal(format!("PutMsg: port {port:#x}'s mp_SigTask {task:#x} is not a live task"))),
        PA_SIGNAL | PA_IGNORE => {}
        f => return Err(Refusal(format!("PutMsg: port {port:#x} has mp_Flags {f} (PA_SOFTINT is not modelled)"))),
    }
    a.mem.write_u8(msg + LN_TYPE, ty).map_err(m("ln_Type"))?;
    add_tail(a, port + MP_MSGLIST, msg)?;
    if flags == PA_IGNORE { return Ok(()); }
    let bit = a.mem.read_u8(port + MP_SIGBIT).map_err(m("mp_SigBit"))? as u32;
    signal_task(a, task, 1 << bit)
}

/// PutMsg(a0 = port, a1 = message).
pub fn put_msg_lvo<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (port, msg) = (cpu.areg(0), cpu.areg(1));
    put_msg(a, port, msg, NT_MESSAGE)?;
    Ok(format!("[exec] PutMsg {msg:#x} -> port {port:#x}"))
}

/// GetMsg(a0 = port) -> the first message (removed from the port), or NULL.
pub fn get_msg<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let port = cpu.areg(0);
    let msg = rem_head(a, port + MP_MSGLIST)?;
    if msg != 0 { a.delivered_msg(msg); }
    cpu.set_d0(msg);
    Ok(format!("[exec] GetMsg port={port:#x} -> {msg:#x}"))
}

/// ReplyMsg(a1 = message): back to its mn_ReplyPort; a message the ENGINE sent (an IntuiMessage, reply port 0) goes
/// back to the engine's pool instead. A reply of a message that was never delivered is a named refusal.
pub fn reply_msg<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let msg = cpu.areg(1);
    if msg == 0 { return Ok("[exec] ReplyMsg NULL".into()); }
    if a.reclaim_engine_msg(msg)? { return Ok(format!("[exec] ReplyMsg {msg:#x} (engine message, reclaimed)")); }
    reply_to_port(a, msg)
}

/// The rest of ReplyMsg for a message that is not the engine's own: it goes back to its mn_ReplyPort (or is just marked replied).
pub fn reply_to_port<U: GuestUi>(a: &mut Amiga<U>, msg: u32) -> Result<String, Refusal> {
    let rp = a.mem.read_u32(msg + MN_REPLYPORT).map_err(m("mn_ReplyPort"))?;
    if rp == 0 { a.mem.write_u8(msg + LN_TYPE, NT_REPLYMSG).map_err(m("ln_Type"))?; return Ok(format!("[exec] ReplyMsg {msg:#x} (no reply port)")); }
    put_msg(a, rp, msg, NT_REPLYMSG)?;
    Ok(format!("[exec] ReplyMsg {msg:#x} -> port {rp:#x}"))
}

/// Signal(a1 = task, d0 = signals).
pub fn signal<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (task, sigs) = (cpu.areg(1), cpu.dreg(0));
    signal_task(a, task, sigs)?;
    Ok(format!("[exec] Signal {task:#x} {sigs:#x}"))
}

/// Wait(d0 = signal set) -> the received signals of that set (cleared). Blocks while none is set. Waiting for a signal
/// the task never allocated could never end by exec's own doing (only an explicit Signal() with that mask could),
/// so it is refused by name instead of hanging the task.
pub fn wait<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let set = cpu.dreg(0);
    let alloc = a.mem.read_u32(a.sched.cur + TC_SIG_ALLOC).map_err(m("tc_SigAlloc"))?;
    if set == 0 || set & !alloc != 0 {
        return Err(Refusal(format!("Wait {set:#x}: signals {:#x} are not allocated to this task (AllocSignal); it could never wake", if set == 0 { 0 } else { set & !alloc })));
    }
    block_on(a, cpu, Waiting::Signals(set), format!("Wait {set:#x}"))
}

/// WaitPort(a0 = port) -> the first message (left queued). Blocks while the port is empty.
pub fn wait_port<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let port = cpu.areg(0);
    block_on(a, cpu, Waiting::Port(port), format!("WaitPort {port:#x}"))
}

/// Complete `w` now if it holds, else park the running task on it and end its slice.
pub fn block_on<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, w: Waiting, what: String) -> Result<String, Refusal> {
    let cur = a.sched.cur;
    if matches!(w, Waiting::Signals(_) | Waiting::Port(_)) {
        let trap = a.input.trap_pc;
        a.input.wait_trap.insert(cur, trap);
        if crate::boopsi_input::pump(a, cpu, w)? { return Ok(format!("[exec] {what} (pointer input for gadgets first)")); }
    }
    match a.satisfy(cur, w, true)? {
        Some(v) => { cpu.set_d0(v); Ok(format!("[exec] {what} -> {v:#x}")) }
        None => { a.park(w); cpu.block(); Ok(format!("[exec] {what} (blocked)")) }
    }
}

impl<U: GuestUi> Amiga<U> {
    /// Does `w` hold for `task`? Some(the call's result) if so; with `commit`, consuming what the call consumes
    /// (the matched signals, the semaphore). None = still waiting.
    pub fn satisfy(&mut self, task: u32, w: Waiting, commit: bool) -> Result<Option<u32>, Refusal> {
        match w {
            Waiting::Signals(set) => {
                let r = sig_recvd(self, task)?;
                if r & set == 0 { return Ok(None); }
                if commit { set_sig_recvd(self, task, r & !set)?; }
                Ok(Some(r & set))
            }
            Waiting::Port(port) => {
                let head = port_head(self, port)?;
                if head == 0 { return Ok(None); }
                if commit {
                    let bit = self.mem.read_u8(port + MP_SIGBIT).map_err(m("mp_SigBit"))? as u32;
                    let r = sig_recvd(self, task)?;
                    set_sig_recvd(self, task, r & !(1 << bit))?;
                }
                Ok(Some(head))
            }
            Waiting::Io { io, remove } => crate::timer::io_done(self, io, remove && commit),
            Waiting::Until(t) => Ok(if self.clock_us >= t { Some(0) } else { None }),
            Waiting::Sem { ss, shared } => crate::semaphores::try_take(self, task, ss, shared, commit),
            Waiting::Request(k) => self.request_done(k, commit),
        }
    }
}
