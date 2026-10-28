//! exec.library list functions over guest Exec lists (exec/lists.h: lh_Head 0, lh_Tail 4, lh_TailPred 8; a Node's
//! ln_Succ 0, ln_Pred 4, ln_Type 8, ln_Pri 9, ln_Name 10). The primitives live once in exec_wait.rs (the message
//! queues use them); these are the program-facing LVOs. Every walk is bounded so a corrupt list is refused, not looped.
use alloc::string::String;
use alloc::format;
use crate::{exec_wait, machine::Amiga, Refusal};
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

pub const LVO_INSERT: i32 = -234;
pub const LVO_ADD_HEAD: i32 = -240;
pub const LVO_ADD_TAIL: i32 = -246;
pub const LVO_REMOVE: i32 = -252;
pub const LVO_REM_HEAD: i32 = -258;
pub const LVO_REM_TAIL: i32 = -264;
pub const LVO_ENQUEUE: i32 = -270;
pub const LVO_FIND_NAME: i32 = -276;
/// More nodes than any real list; past it the list is taken to be corrupt (cyclic).
const MAX_WALK: u32 = 1 << 20;

fn m(what: &'static str) -> impl Fn(heirloom_mem_flat32::MemError) -> Refusal { Refusal::mem(what) }

/// Link `node` after `pred` (Insert's core; pred = the list header for AddHead).
pub(crate) fn link_after<U: GuestUi>(a: &mut Amiga<U>, pred: u32, node: u32) -> Result<(), Refusal> {
    let succ = a.mem.read_u32(pred).map_err(m("pred.ln_Succ"))?;
    a.mem.write_u32(node, succ).map_err(m("ln_Succ"))?;
    a.mem.write_u32(node + 4, pred).map_err(m("ln_Pred"))?;
    a.mem.write_u32(succ + 4, node).map_err(m("succ.ln_Pred"))?;
    a.mem.write_u32(pred, node).map_err(m("pred.ln_Succ"))
}

pub fn add_head<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (list, node) = (cpu.areg(0), cpu.areg(1));
    link_after(a, list, node)?;
    Ok(format!("[exec] AddHead {list:#x} {node:#x}"))
}

pub fn add_tail<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (list, node) = (cpu.areg(0), cpu.areg(1));
    exec_wait::add_tail(a, list, node)?;
    Ok(format!("[exec] AddTail {list:#x} {node:#x}"))
}

/// Insert(a0 = list, a1 = node, a2 = pred): after `pred`, or at the head when pred is NULL.
pub fn insert<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (list, node, pred) = (cpu.areg(0), cpu.areg(1), cpu.areg(2));
    link_after(a, if pred == 0 { list } else { pred }, node)?;
    Ok(format!("[exec] Insert {list:#x} {node:#x} after {pred:#x}"))
}

pub fn remove<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let node = cpu.areg(1);
    exec_wait::remove(a, node)?;
    Ok(format!("[exec] Remove {node:#x}"))
}

pub fn rem_head<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let list = cpu.areg(0);
    let n = exec_wait::rem_head(a, list)?;
    cpu.set_d0(n);
    Ok(format!("[exec] RemHead {list:#x} -> {n:#x}"))
}

/// RemTail(a0 = list) -> the last node (removed), or NULL for an empty list (lh_TailPred == the header).
pub fn rem_tail<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let list = cpu.areg(0);
    let last = a.mem.read_u32(list + 8).map_err(m("lh_TailPred"))?;
    let n = if last == list { 0 } else { exec_wait::remove(a, last)?; last };
    cpu.set_d0(n);
    Ok(format!("[exec] RemTail {list:#x} -> {n:#x}"))
}

/// Enqueue(a0 = list, a1 = node): before the first node of LOWER priority (FIFO among equals).
pub fn enqueue<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (list, node) = (cpu.areg(0), cpu.areg(1));
    let pri = a.mem.read_u8(node + 9).map_err(m("ln_Pri"))? as i8;
    let mut cur = a.mem.read_u32(list).map_err(m("lh_Head"))?;
    for _ in 0..MAX_WALK {
        let succ = a.mem.read_u32(cur).map_err(m("ln_Succ"))?;
        if succ == 0 || (a.mem.read_u8(cur + 9).map_err(m("ln_Pri"))? as i8) < pri {
            let pred = a.mem.read_u32(cur + 4).map_err(m("ln_Pred"))?;
            link_after(a, pred, node)?;
            return Ok(format!("[exec] Enqueue {list:#x} {node:#x} pri={pri}"));
        }
        cur = succ;
    }
    Err(Refusal(format!("Enqueue: list {list:#x} longer than {MAX_WALK} nodes (cyclic?)")))
}

/// FindName(a0 = list or node to start AFTER, a1 = name) -> the first node of that name, or NULL. Case-sensitive.
pub fn find_name<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (start, np) = (cpu.areg(0), cpu.areg(1));
    let want = a.mem.read_cstr(np).map_err(m("FindName.name"))?;
    let mut cur = a.mem.read_u32(start).map_err(m("ln_Succ"))?;
    for _ in 0..MAX_WALK {
        let succ = a.mem.read_u32(cur).map_err(m("ln_Succ"))?;
        if succ == 0 { cpu.set_d0(0); return Ok(format!("[exec] FindName \"{want}\" -> NULL")); }
        let name = a.mem.read_u32(cur + 10).map_err(m("ln_Name"))?;
        if name != 0 && a.mem.read_cstr(name).map_err(m("ln_Name"))? == want { cpu.set_d0(cur); return Ok(format!("[exec] FindName \"{want}\" -> {cur:#x}")); }
        cur = succ;
    }
    Err(Refusal(format!("FindName: list longer than {MAX_WALK} nodes (cyclic?)")))
}
