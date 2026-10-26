//! Small exec.library services a real program calls once at startup and the engine can answer without a model behind them.
//!
//! AddPort / RemPort / FindPort: the public port list (ExecBase.PortList, a real list at offset 392; AROS addport.c enqueues
//! by priority, findport.c is FindName). AddPort makes the port's message list empty and sets ln_Type NT_MSGPORT; RemPort of a
//! port that is not on the list is a named refusal; the lists.rs routines do the list work, so a guest walking the list sees it.
//!
//! AddMemHandler(a1 = struct Interrupt) / RemMemHandler(a1): low-memory handlers (AROS rom/exec/addmemhandler.c). exec calls
//! them (is_Code, with a MemHandlerData in A0 and is_Data in A1) when an allocation fails, to let the owner free caches. The
//! engine records them, in the order added, and RemMemHandler removes by address (an unknown one is a named refusal), but
//! AllocMem does not call them yet: the guest heap is 14 MB and no program in the corpus gets near it. That is the one
//! behaviour missing, and the log says "(not called: AllocMem failures do not run handlers)" at registration.
use alloc::format;
use alloc::string::String;
use crate::libtable::LibraryBuilder;
use crate::lists;
use crate::machine::EXEC_BASE;
use crate::machine::Amiga;
use crate::Refusal;
use heirloom_ui_abi::GuestUi;
use heirloom_cpu_68k::Cpu;

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.named("AddPort", add_port::<U>).named("RemPort", rem_port::<U>).named("FindPort", find_port::<U>)
        .named("AddMemHandler", add_mem_handler::<U>).named("RemMemHandler", rem_mem_handler::<U>).named("RawPutChar", raw_put_char::<U>)
}

/// RawPutChar(d0 = character): one byte to the low-level debug port, which is the ENGINE LOG here (never the program's stdout):
/// bytes collect until a newline, then the line is logged as `[serial] ...`. A zero byte is ignored (AROS rom/exec/rawputchar.c:
/// "Don't write 0 bytes"). An unfinished line is logged when the program ends is not modelled: it stays in `Console::serial`.
fn raw_put_char<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let c = cpu.dreg(0) as u8;
    if c == 0 { return Ok("[exec] RawPutChar 0 (ignored)".into()); }
    if c == b'\n' {
        let line: String = a.console.serial.drain(..).map(|b| b as char).collect();
        a.log.push(format!("[serial] {line}"));
    } else { a.console.serial.push(c); }
    Ok(format!("[exec] RawPutChar {:?}", c as char))
}

fn add_mem_handler<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let h = cpu.areg(1);
    if h == 0 { return Err(Refusal("AddMemHandler: NULL interrupt".into())); }
    if a.sched.mem_handlers.contains(&h) { return Err(Refusal(format!("AddMemHandler: {h:#x} is already a memory handler"))); }
    a.sched.mem_handlers.push(h);
    Ok(format!("[exec] AddMemHandler {h:#x} (not called: AllocMem failures do not run handlers)"))
}

fn rem_mem_handler<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let h = cpu.areg(1);
    let Some(i) = a.sched.mem_handlers.iter().position(|&x| x == h) else { return Err(Refusal(format!("RemMemHandler: {h:#x} is not a memory handler AddMemHandler added"))) };
    a.sched.mem_handlers.remove(i);
    Ok(format!("[exec] RemMemHandler {h:#x}"))
}

const PORT_LIST: u32 = EXEC_BASE + 392;
const MAX_WALK: u32 = 100_000;
const NT_MSGPORT: u8 = 4;

fn on_port_list<U: GuestUi>(a: &Amiga<U>, port: u32) -> Result<bool, Refusal> {
    let mut cur = a.mem.read_u32(PORT_LIST).map_err(Refusal::mem("PortList"))?;
    for _ in 0..MAX_WALK {
        let succ = a.mem.read_u32(cur).map_err(Refusal::mem("PortList"))?;
        if succ == 0 { return Ok(false); }
        if cur == port { return Ok(true); }
        cur = succ;
    }
    Err(Refusal("PortList is longer than 100000 nodes (cyclic?)".into()))
}

fn add_port<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let port = cpu.areg(1);
    if port == 0 { return Err(Refusal("AddPort: NULL port".into())); }
    if on_port_list(a, port)? { return Err(Refusal(format!("AddPort: {port:#x} is already on the port list"))); }
    let m = Refusal::mem("MsgPort");
    a.mem.write_u8(port + 8, NT_MSGPORT).map_err(&m)?;
    a.mem.write_u32(port + 20, port + 24).map_err(&m)?;
    a.mem.write_u32(port + 24, 0).map_err(&m)?;
    a.mem.write_u32(port + 28, port + 20).map_err(&m)?;
    cpu.set_areg(0, PORT_LIST);
    lists::enqueue(a, cpu)?;
    Ok(format!("[exec] AddPort {port:#x}"))
}

fn rem_port<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let port = cpu.areg(1);
    if !on_port_list(a, port)? { return Err(Refusal(format!("RemPort: {port:#x} is not on the port list"))); }
    lists::remove(a, cpu)?;
    Ok(format!("[exec] RemPort {port:#x}"))
}

/// FindPort(a1 = name) -> the public port of that name, or NULL.
fn find_port<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    cpu.set_areg(0, PORT_LIST);
    let line = lists::find_name(a, cpu)?;
    Ok(line.replace("FindName", "FindPort"))
}
