//! commodities.library: BrokerCommand and the CxMsg readers (AROS workbench/libs/commodities brokercommand.c, cxmsgtype.c, cxmsgid.c,
//! cxmsgdata.c, eventfuncs.c; APL, read for facts only).
//!
//! BrokerCommand(name, command) tells the task that owns broker `name` (NULL = "Exchange", the user-facing commodity manager) about a
//! state change, the way Exchange does: it finds the broker in the object graph, builds a `struct CxMsg` (162 bytes; m68k gcc
//! -mno-align-int over AROS cxintern.h: Message 0, cxm_Routing 20, cxm_ID 24, cxm_Type 28 = CXM_COMMAND, cxm_Level 29, cxm_retObj 30,
//! cxm_Data 158) whose ID is the command and PutMsg's it to the broker's port. Results: 0 sent (or, for a broker with no port, CXCMD_KILL
//! signalled to its task as CTRL-E), -1 no such broker, -2 the broker has no port, -3 no memory. So with no Exchange running (the engine
//! has none: it has no commodity manager) `BrokerCommand(NULL, ...)` answers -1, exactly what an Exchange-less AmigaOS answers.
//! The broker replies with ReplyMsg: the message goes back to the engine's own port, and the next BrokerCommand frees what came back.
use alloc::format;
use alloc::string::String;
use crate::exec_wait;
use crate::libtable::LibraryBuilder;
use crate::machine::Amiga;
use crate::Refusal;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

const CXMSG_SIZE: u32 = 162;
pub(crate) const CXM_ID: u32 = 24;
pub(crate) const CXM_TYPE: u32 = 28;
const CXM_DATA: u32 = 158;
pub(crate) const CXM_COMMAND: u8 = 64;
// libraries/commodities.h (AROS): the commands a broker's port receives.
pub(crate) const CXCMD_DISABLE: u32 = 15;
pub(crate) const CXCMD_ENABLE: u32 = 17;
pub(crate) const CXCMD_KILL: u32 = 23;
pub(crate) const CXCMD_LIST_CHG: u32 = 27;
const SIGBREAKF_CTRL_E: u32 = 1 << 12;
const MN_REPLY_PORT: u32 = 14;
const MN_LENGTH: u32 = 18;
const MSGPORT_SIZE: u32 = 34;

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.named("BrokerCommand", broker_command::<U>).named("CopyBrokerList", copy_broker_list::<U>).named("FreeBrokerList", free_broker_list::<U>).named("CxMsgType", cx_msg_type::<U>).named("CxMsgID", cx_msg_id::<U>).named("CxMsgData", cx_msg_data::<U>)
}

/// The port replies come back to, made at the first use (PA_IGNORE: nothing waits on it, the next BrokerCommand frees its messages).
fn reply_port<U: GuestUi>(a: &mut Amiga<U>) -> Result<u32, Refusal> {
    if a.cx.reply_port == 0 {
        let p = crate::fs::alloc_zeroed_sys(a, MSGPORT_SIZE)?.ok_or_else(|| Refusal("commodities: no memory for the reply port (NOMEM)".into()))?;
        crate::ports::init_port(a, p, 0)?;
        a.mem.write_u8(p + 14, 2).map_err(Refusal::mem("mp_Flags"))?; // PA_IGNORE
        a.cx.reply_port = p;
    }
    Ok(a.cx.reply_port)
}

/// BrokerCommand(a0 = broker name or NULL, d0 = command) -> 0, or -1 / -2 / -3.
fn broker_command<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (name_p, command) = (cpu.areg(0), cpu.dreg(0));
    let name = if name_p == 0 { String::from("Exchange") } else { a.mem.read_cstr(name_p).map_err(Refusal::mem("BrokerCommand name"))? };
    let (r, what) = send_command_why(a, &name, command)?;
    cpu.set_d0(r as u32);
    Ok(format!("[commodities] BrokerCommand \"{name}\" {command} -> {r} ({what})"))
}

/// BrokerCommand without the registers: the result (0 sent, -1 / -2 / -3).
pub(crate) fn send_command<U: GuestUi>(a: &mut Amiga<U>, name: &str, command: u32) -> Result<i32, Refusal> { send_command_why(a, name, command).map(|(r, _)| r) }

fn send_command_why<U: GuestUi>(a: &mut Amiga<U>, name: &str, command: u32) -> Result<(i32, &'static str), Refusal> {
    let rp = reply_port(a)?;
    loop { // the replies of earlier commands: freed here
        let m = exec_wait::rem_head(a, rp + 20)?;
        if m == 0 { break; }
        a.heap.free(m, CXMSG_SIZE).map_err(|e| Refusal(format!("BrokerCommand: {e}")))?;
    }
    let found = a.cx.brokers.iter().copied().find(|b| a.cx.objs.get(b).map_or(false, |o| o.name == name));
    Ok(match found {
        None => (-1, "unknown broker"),
        Some(b) => {
            let (port, task) = { let o = &a.cx.objs[&b]; (o.port, o.task) };
            if port == 0 {
                if command == CXCMD_KILL && task != 0 { exec_wait::signal_task(a, task, SIGBREAKF_CTRL_E)?; (0, "no port: CXCMD_KILL signalled the task (CTRL-E)") } else { (-2, "the broker has no message port") }
            } else if let Some(msg) = a.heap.alloc(CXMSG_SIZE) {
                a.mem.zero(msg, CXMSG_SIZE).map_err(Refusal::mem("CxMsg"))?;
                let m = Refusal::mem("CxMsg");
                a.mem.write_u32(msg + MN_REPLY_PORT, rp).map_err(&m)?;
                a.mem.write_u16(msg + MN_LENGTH, CXMSG_SIZE as u16).map_err(&m)?;
                a.mem.write_u32(msg + CXM_ID, command).map_err(&m)?;
                a.mem.write_u8(msg + CXM_TYPE, CXM_COMMAND).map_err(&m)?;
                exec_wait::put_msg(a, port, msg, 5)?;
                (0, "sent")
            } else { (-3, "no memory") }
        }
    })
}

// struct BrokerCopy (libraries/commodities.h, m68k gcc -mno-align-int): Node 14, bc_Name[24] 14, bc_Title[40] 38, bc_Descr[40] 78, bc_Task 118,
// bc_Port 122, bc_Dummy 126, bc_Flags 128; 132 bytes.
const BC_SIZE: u32 = 132;
const BC_NAME: u32 = 14;
const BC_TITLE: u32 = 38;
const BC_DESCR: u32 = 78;
const BC_TASK: u32 = 118;
const BC_PORT: u32 = 122;
const BC_FLAGS: u32 = 128;
const LN_NAME: u32 = 10;
const COF_ACTIVE: u32 = 1 << 1;

/// FreeBrokerList(a0 = list): every BrokerCopy node is removed and freed.
fn free_broker_list<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let list = cpu.areg(0);
    let n = free_nodes(a, list)?;
    Ok(format!("[commodities] FreeBrokerList {list:#x} ({n} nodes)"))
}

fn free_nodes<U: GuestUi>(a: &mut Amiga<U>, list: u32) -> Result<u32, Refusal> {
    let mut n = 0;
    loop {
        let node = exec_wait::rem_head(a, list)?;
        if node == 0 { return Ok(n); }
        a.heap.free(node, BC_SIZE).map_err(|e| Refusal(format!("FreeBrokerList: {e}")))?;
        n += 1;
    }
}

/// CopyBrokerList(a0 = list) -> the number of brokers: the old nodes are freed, then one BrokerCopy per broker, in priority order
/// (AROS commodities copybrokerlist.c). Name, title and description are cut to CBD_NAMELEN / TITLELEN / DESCRLEN with a NUL.
fn copy_broker_list<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let list = cpu.areg(0);
    free_nodes(a, list)?;
    let mut count = 0;
    for b in a.cx.brokers.clone() {
        let Some(node) = a.heap.alloc(BC_SIZE) else { break };
        a.mem.zero(node, BC_SIZE).map_err(Refusal::mem("BrokerCopy"))?;
        let o = &a.cx.objs[&b];
        let (name, title, descr, task, port, active) = (o.name.clone(), o.title.clone(), o.descr.clone(), o.task, o.port, o.active);
        let m = Refusal::mem("BrokerCopy");
        for (off, s, cap) in [(BC_NAME, &name, 24usize), (BC_TITLE, &title, 40), (BC_DESCR, &descr, 40)] {
            let bytes: alloc::vec::Vec<u8> = s.chars().map(|c| c as u32 as u8).take(cap - 1).collect();
            a.mem.write_bytes(node + off, &bytes).map_err(&m)?;
        }
        a.mem.write_u32(node + LN_NAME, node + BC_NAME).map_err(&m)?;
        a.mem.write_u32(node + BC_TASK, task).map_err(&m)?;
        a.mem.write_u32(node + BC_PORT, port).map_err(&m)?;
        a.mem.write_u32(node + BC_FLAGS, if active { COF_ACTIVE } else { 0 }).map_err(&m)?;
        exec_wait::add_tail(a, list, node)?;
        count += 1;
    }
    cpu.set_d0(count);
    Ok(format!("[commodities] CopyBrokerList {list:#x} -> {count}"))
}

fn cx_msg_type<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let m = cpu.areg(0);
    let t = a.mem.read_u8(m + CXM_TYPE).map_err(Refusal::mem("CxMsg.cxm_Type"))?;
    cpu.set_d0(t as u32);
    Ok(format!("[commodities] CxMsgType {m:#x} -> {t}"))
}

fn cx_msg_id<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let m = cpu.areg(0);
    let id = a.mem.read_u32(m + CXM_ID).map_err(Refusal::mem("CxMsg.cxm_ID"))?;
    cpu.set_d0(id);
    Ok(format!("[commodities] CxMsgID {m:#x} -> {id}"))
}

/// CxMsgData(a0 = message) -> the InputEvent it carries (a command message carries none), or NULL for NULL.
fn cx_msg_data<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let m = cpu.areg(0);
    let d = if m == 0 { 0 } else { a.mem.read_u32(m + CXM_DATA).map_err(Refusal::mem("CxMsg.cxm_Data"))? };
    cpu.set_d0(d);
    Ok(format!("[commodities] CxMsgData {m:#x} -> {d:#x}"))
}
