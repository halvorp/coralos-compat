//! exec.library signals and message ports: AllocSignal / FreeSignal / CreateMsgPort / DeleteMsgPort.
//!
//! Per task (the running one, `Amiga::sched.cur`): its tc_SigAlloc (Task offset 18) is the allocation map, signals 0..15 are exec's own. A port is the
//! shape the AROS oracle shows for a live one (tests/oracle/msgport.bin): NT_MSGPORT, PA_SIGNAL, the first free user
//! signal, mp_SigTask = the task that created it (PutMsg signals THAT task, exec_wait.rs), an empty list (lh_Head = &lh_Tail, lh_TailPred = &lh_Head, lh_Type 0).
use alloc::string::String;
use alloc::format;
use crate::{machine::Amiga, Refusal};
use heirloom_ui_abi::GuestUi;
use heirloom_cpu_68k::Cpu;

pub const LVO_ALLOC_SIGNAL: i32 = -330;
pub const LVO_FREE_SIGNAL: i32 = -336;
pub const LVO_CREATE_MSG_PORT: i32 = -666;
pub const LVO_DELETE_MSG_PORT: i32 = -672;

const TC_SIG_ALLOC: u32 = 18;
const MSGPORT_SIZE: u32 = 34;
const NT_MSGPORT: u8 = 4;
const PA_SIGNAL: u8 = 0;

fn sig_alloc<U: GuestUi>(a: &Amiga<U>) -> Result<u32, Refusal> {
    a.mem.read_u32(a.sched.cur + TC_SIG_ALLOC).map_err(Refusal::mem("tc_SigAlloc"))
}

/// Allocate `wanted` (-1 = any) of the task's signals; Some(bit) or None (all taken / that one taken).
pub fn alloc_signal_bit<U: GuestUi>(a: &mut Amiga<U>, wanted: i32) -> Result<Option<u32>, Refusal> {
    let map = sig_alloc(a)?;
    let bit = if wanted < 0 { (0..32).find(|b| map & (1 << b) == 0) } else if wanted < 32 && map & (1 << wanted) == 0 { Some(wanted as u32) } else { None };
    if let Some(b) = bit { a.mem.write_u32(a.sched.cur + TC_SIG_ALLOC, map | (1 << b)).map_err(Refusal::mem("tc_SigAlloc"))?; }
    Ok(bit)
}

/// AllocSignal(d0 = signalNum or -1) -> the bit, or -1.
pub fn alloc_signal<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let wanted = cpu.dreg(0) as i8 as i32; // signalNum is a BYTE (AROS AllocSignal): the rest of D0 is the caller's garbage
    let got = alloc_signal_bit(a, wanted)?;
    cpu.set_d0(got.map(|b| b).unwrap_or(u32::MAX));
    Ok(format!("[exec] AllocSignal {wanted} -> {}", got.map(|b| b as i64).unwrap_or(-1)))
}

/// FreeSignal(d0 = signalNum); -1 is a no-op. Freeing a signal that is not allocated is a named refusal.
pub fn free_signal<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let n = cpu.dreg(0) as i8 as i32; // a BYTE, like AllocSignal
    if n == -1 { return Ok("[exec] FreeSignal -1".into()); }
    let map = sig_alloc(a)?;
    if !(16..32).contains(&n) || map & (1 << n) == 0 { return Err(Refusal(format!("FreeSignal {n}: not an allocated user signal"))); }
    a.mem.write_u32(a.sched.cur + TC_SIG_ALLOC, map & !(1 << n)).map_err(Refusal::mem("tc_SigAlloc"))?;
    Ok(format!("[exec] FreeSignal {n}"))
}

/// Initialise a MsgPort at `port` for signal `bit` of the program's task.
pub fn init_port<U: GuestUi>(a: &mut Amiga<U>, port: u32, bit: u32) -> Result<(), Refusal> {
    let m = Refusal::mem;
    for i in 0..MSGPORT_SIZE { a.mem.write_u8(port + i, 0).map_err(m("MsgPort"))?; }
    a.mem.write_u8(port + 8, NT_MSGPORT).map_err(m("mp_Node.ln_Type"))?;
    a.mem.write_u8(port + 14, PA_SIGNAL).map_err(m("mp_Flags"))?;
    a.mem.write_u8(port + 15, bit as u8).map_err(m("mp_SigBit"))?;
    a.mem.write_u32(port + 16, a.sched.cur).map_err(m("mp_SigTask"))?;
    a.mem.write_u32(port + 20, port + 24).map_err(m("lh_Head"))?;
    a.mem.write_u32(port + 28, port + 20).map_err(m("lh_TailPred"))?;
    Ok(())
}

/// A new signalling port of the running task (CreateMsgPort), or None when it has no free signal or there is no memory.
pub fn new_port<U: GuestUi>(a: &mut Amiga<U>) -> Result<Option<u32>, Refusal> {
    let Some(bit) = alloc_signal_bit(a, -1)? else { return Ok(None) };
    let Some(port) = a.heap.alloc(MSGPORT_SIZE) else { release_signal_bit(a, bit as i32)?; return Ok(None) };
    init_port(a, port, bit)?;
    Ok(Some(port))
}

/// Free a port from [`new_port`] and give its signal bit back to the running task (DeleteMsgPort).
pub fn free_port<U: GuestUi>(a: &mut Amiga<U>, port: u32) -> Result<(), Refusal> {
    let bit = a.mem.read_u8(port + 15).map_err(Refusal::mem("mp_SigBit"))? as u32;
    a.heap.free(port, MSGPORT_SIZE).map_err(|e| Refusal(format!("DeleteMsgPort: {e}")))?;
    release_signal_bit(a, bit as i32)
}

/// CreateMsgPort() -> a new signalling port of this task, or NULL (no signal / no memory).
pub fn create_msg_port<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let Some(port) = new_port(a)? else { cpu.set_d0(0); return Ok("[exec] CreateMsgPort -> 0 (no free signal, or no memory)".into()) };
    cpu.set_d0(port);
    Ok(format!("[exec] CreateMsgPort -> {port:#x} (signal {})", a.mem.read_u8(port + 15).map_err(Refusal::mem("mp_SigBit"))?))
}

/// DeleteMsgPort(a0 = port from CreateMsgPort, or NULL).
pub fn delete_msg_port<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let port = cpu.areg(0);
    if port == 0 { return Ok("[exec] DeleteMsgPort NULL".into()); }
    free_port(a, port)?;
    Ok(format!("[exec] DeleteMsgPort {port:#x}"))
}

pub const LVO_CREATE_IO_REQUEST: i32 = -654;
pub const LVO_DELETE_IO_REQUEST: i32 = -660;
/// exec/nodes.h NT_REPLYMSG: a fresh IORequest is "replied" until it is sent.
const NT_REPLYMSG: u8 = 7;

/// CreateIORequest(a0 = reply port, d0 = size) -> a zeroed IORequest of `size` bytes whose mn_ReplyPort is the port,
/// or NULL (no port, a size below struct Message, or no memory).
pub fn create_io_request<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (port, size) = (cpu.areg(0), cpu.dreg(0));
    let io = if port == 0 || size < 20 { None } else { a.heap.alloc(size) };
    if let Some(io) = io {
        for i in 0..size { a.mem.write_u8(io + i, 0).map_err(Refusal::mem("IORequest"))?; }
        a.mem.write_u8(io + 8, NT_REPLYMSG).map_err(Refusal::mem("io_Message.ln_Type"))?;
        a.mem.write_u32(io + 14, port).map_err(Refusal::mem("mn_ReplyPort"))?;
        a.mem.write_u16(io + 18, size as u16).map_err(Refusal::mem("mn_Length"))?;
    }
    cpu.set_d0(io.unwrap_or(0));
    Ok(format!("[exec] CreateIORequest port={port:#x} size={size} -> {:#x}", io.unwrap_or(0)))
}

/// DeleteIORequest(a0 = request from CreateIORequest, or NULL).
pub fn delete_io_request<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let io = cpu.areg(0);
    if io == 0 { return Ok("[exec] DeleteIORequest NULL".into()); }
    let size = a.mem.read_u16(io + 18).map_err(Refusal::mem("mn_Length"))? as u32;
    a.heap.free(io, size).map_err(|e| Refusal(format!("DeleteIORequest: {e}")))?;
    Ok(format!("[exec] DeleteIORequest {io:#x}"))
}

/// Give signal `bit` back to the task (a window's UserPort closing); -1 = nothing allocated.
pub fn release_signal_bit<U: GuestUi>(a: &mut Amiga<U>, bit: i32) -> Result<(), Refusal> {
    if bit < 0 { return Ok(()); }
    let map = sig_alloc(a)?;
    a.mem.write_u32(a.sched.cur + TC_SIG_ALLOC, map & !(1 << bit)).map_err(Refusal::mem("tc_SigAlloc"))
}
