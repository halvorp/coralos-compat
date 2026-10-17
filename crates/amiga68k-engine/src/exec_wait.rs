//! exec.library messages and waiting: PutMsg / GetMsg / ReplyMsg / Signal / Wait / WaitPort, over real Exec lists in
//! guest memory (exec/lists.h: lh_Head 0, lh_Tail 4, lh_TailPred 8; a Node's ln_Succ 0, ln_Pred 4).
//!
//! SUSPENSION (docs/SLICE-2-ENGINE.md, Astra 2026-10-17 option a): Wait / WaitPort with nothing pending record what
//! they wait for and `cpu.block()`; the host's next `Amiga::step` re-checks the predicate first and resumes the guest
//! only when it holds, with D0 set to the call's result (the matched signals, the port's head message). No guest
//! instruction runs while a wait is unsatisfied.
use alloc::string::String;
use alloc::format;
use crate::{machine::Amiga, process, Refusal};
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

pub const LVO_SIGNAL: i32 = -324;
pub const LVO_WAIT: i32 = -318;
pub const LVO_PUT_MSG: i32 = -366;
pub const LVO_GET_MSG: i32 = -372;
pub const LVO_REPLY_MSG: i32 = -378;
pub const LVO_WAIT_PORT: i32 = -384;

const TC_SIG_RECVD: u32 = 26;
const MP_SIGBIT: u32 = 15;
const MP_MSGLIST: u32 = 20;
const MN_REPLYPORT: u32 = 14;
const LN_TYPE: u32 = 8;
const NT_MESSAGE: u8 = 5;
const NT_REPLYMSG: u8 = 7;

/// What a blocked guest is waiting for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waiting { Signals(u32), Port(u32) }

fn m(what: &'static str) -> impl Fn(retro_guest_mem::MemError) -> Refusal { Refusal::mem(what) }

pub fn sig_recvd<U: GuestUi>(a: &Amiga<U>) -> Result<u32, Refusal> { a.mem.read_u32(process::PROCESS + TC_SIG_RECVD).map_err(m("tc_SigRecvd")) }
fn set_sig_recvd<U: GuestUi>(a: &mut Amiga<U>, v: u32) -> Result<(), Refusal> { a.mem.write_u32(process::PROCESS + TC_SIG_RECVD, v).map_err(m("tc_SigRecvd")) }

/// Raise `sigs` on the (one) task.
pub fn signal_task<U: GuestUi>(a: &mut Amiga<U>, sigs: u32) -> Result<(), Refusal> { let r = sig_recvd(a)?; set_sig_recvd(a, r | sigs) }

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
    a.mem.write_u8(msg + LN_TYPE, ty).map_err(m("ln_Type"))?;
    add_tail(a, port + MP_MSGLIST, msg)?;
    let bit = a.mem.read_u8(port + MP_SIGBIT).map_err(m("mp_SigBit"))? as u32;
    signal_task(a, 1 << bit)
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
    let rp = a.mem.read_u32(msg + MN_REPLYPORT).map_err(m("mn_ReplyPort"))?;
    if rp == 0 { a.mem.write_u8(msg + LN_TYPE, NT_REPLYMSG).map_err(m("ln_Type"))?; return Ok(format!("[exec] ReplyMsg {msg:#x} (no reply port)")); }
    put_msg(a, rp, msg, NT_REPLYMSG)?;
    Ok(format!("[exec] ReplyMsg {msg:#x} -> port {rp:#x}"))
}

/// Signal(a1 = task, d0 = signals): one task, so `task` must be it.
pub fn signal<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    if cpu.areg(1) != process::PROCESS { return Err(Refusal(format!("Signal: {:#x} is not a task (one task only)", cpu.areg(1)))); }
    signal_task(a, cpu.dreg(0))?;
    Ok(format!("[exec] Signal {:#x}", cpu.dreg(0)))
}

/// Wait(d0 = signal set) -> the received signals of that set (cleared). Blocks while none is set.
pub fn wait<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let set = cpu.dreg(0);
    match a.try_satisfy(Waiting::Signals(set))? {
        Some(got) => { cpu.set_d0(got); Ok(format!("[exec] Wait {set:#x} -> {got:#x}")) }
        None => { a.waiting = Some(Waiting::Signals(set)); cpu.block(); Ok(format!("[exec] Wait {set:#x} (blocked)")) }
    }
}

/// WaitPort(a0 = port) -> the first message (left queued). Blocks while the port is empty.
pub fn wait_port<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let port = cpu.areg(0);
    match a.try_satisfy(Waiting::Port(port))? {
        Some(head) => { cpu.set_d0(head); Ok(format!("[exec] WaitPort {port:#x} -> {head:#x}")) }
        None => { a.waiting = Some(Waiting::Port(port)); cpu.block(); Ok(format!("[exec] WaitPort {port:#x} (blocked)")) }
    }
}

impl<U: GuestUi> Amiga<U> {
    /// Is `w` satisfied now? Some(the call's result), consuming the matched signals; None = still waiting.
    pub fn try_satisfy(&mut self, w: Waiting) -> Result<Option<u32>, Refusal> {
        match w {
            Waiting::Signals(set) => {
                let r = sig_recvd(self)?;
                if r & set == 0 { return Ok(None); }
                set_sig_recvd(self, r & !set)?;
                Ok(Some(r & set))
            }
            Waiting::Port(port) => {
                let head = port_head(self, port)?;
                if head == 0 { return Ok(None); }
                let bit = self.mem.read_u8(port + MP_SIGBIT).map_err(m("mp_SigBit"))? as u32;
                let r = sig_recvd(self)?;
                set_sig_recvd(self, r & !(1 << bit))?;
                Ok(Some(head))
            }
        }
    }
}
