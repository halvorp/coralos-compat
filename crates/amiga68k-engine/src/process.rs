//! The program's own `struct Process` (dos/dosextens.h) and exec's `FindTask`.
//!
//! Every AmigaOS program runs as a Process. Its C startup asks `FindTask(NULL)` for it and reads `pr_CLI` to tell a CLI
//! start (arguments in A0/D0) from a Workbench start (wait on `pr_MsgPort` for a WBStartup message). This engine starts
//! programs as from a CLI, so `pr_CLI` points at a zeroed `struct CommandLineInterface` whose `cli_CommandName` is set.
//!
//! Offsets: m68k-linux-gnu-gcc -mno-align-int (the Amiga ABI) over AROS's exec/tasks.h and dos/dosextens.h field lists
//! (2026-10-17): Task is 92 bytes; tc_SigAlloc 18, tc_SigRecvd 26, tc_SPLower 58, tc_SPUpper 62; pr_MsgPort 92,
//! pr_StackSize 132, pr_CIS 156, pr_COS 160, pr_CLI 172. ExecBase.ThisTask is 276 (exec/execbase.h).
use alloc::string::String;
use alloc::format;
use crate::{machine::Amiga, Refusal};
use guestui_abi::GuestUi;
use retro_guest_mem::GuestMem;
use retro_m68k::Cpu;

pub const LVO_FIND_TASK: i32 = -294;

/// Where the program's Process, its CLI struct, the CLI's command-name BSTR and the argument line live.
pub const PROCESS: u32 = 0x6000;
const PROCESS_SIZE: u32 = 228;
const CLI: u32 = 0x6100;
const CLI_SIZE: u32 = 64;
const COMMAND_NAME: u32 = 0x6180;
pub const ARGS: u32 = 0x6200;

const NT_PROCESS: u8 = 13;
const NT_MSGPORT: u8 = 4;
const TS_RUN: u8 = 2;
/// Signals 0..15 are exec's own; SIGB_DOS (8) is the Process port's.
const SYSTEM_SIGNALS: u32 = 0x0000_FFFF;
const SIGB_DOS: u8 = 8;

const EXECBASE_THIS_TASK: u32 = 276;
/// ExecBase.AttnFlags (UWORD, exec/execbase.h): which CPU/FPU the machine has. The engine runs a 68020 with no FPU:
/// AFF_68010 | AFF_68020. Programs built for 68020+ (DOpus5's OS3 build) Alert at startup when these are clear.
const EXECBASE_ATTN_FLAGS: u32 = 296;
const AFF_68010_68020: u16 = 0x0003;
const TC_NODE_TYPE: u32 = 8;
const TC_NODE_NAME: u32 = 10;
const TC_STATE: u32 = 15;
const TC_SIG_ALLOC: u32 = 18;
const TC_SP_LOWER: u32 = 58;
const TC_SP_UPPER: u32 = 62;
const PR_MSGPORT: u32 = 92;
const PR_STACKSIZE: u32 = 132;
const PR_CLI: u32 = 172;
/// struct CommandLineInterface.cli_CommandName (a BSTR), dos/dosextens.h.
const CLI_COMMAND_NAME: u32 = 16;

/// Lay out the program's Process at [`PROCESS`] (zeroed first) and publish it as ExecBase->ThisTask. `name` is the
/// command name the CLI reports, `args` the argument line (AmigaDOS ends it with a newline).
pub fn setup(mem: &mut GuestMem, exec_base: u32, stack_lower: u32, stack_upper: u32, name: &str, args: &str) -> Result<u32, Refusal> {
    let m = Refusal::mem;
    for a in (PROCESS..PROCESS + PROCESS_SIZE).chain(CLI..CLI + CLI_SIZE) { mem.write_u8(a, 0).map_err(m("Process"))?; }
    mem.write_u8(PROCESS + TC_NODE_TYPE, NT_PROCESS).map_err(m("tc_Node.ln_Type"))?;
    mem.write_u8(PROCESS + TC_STATE, TS_RUN).map_err(m("tc_State"))?;
    mem.write_u32(PROCESS + TC_SIG_ALLOC, SYSTEM_SIGNALS).map_err(m("tc_SigAlloc"))?;
    mem.write_u32(PROCESS + TC_SP_LOWER, stack_lower).map_err(m("tc_SPLower"))?;
    mem.write_u32(PROCESS + TC_SP_UPPER, stack_upper).map_err(m("tc_SPUpper"))?;
    mem.write_u32(PROCESS + PR_STACKSIZE, stack_upper - stack_lower).map_err(m("pr_StackSize"))?;
    // pr_MsgPort: a signalling port of this task on SIGB_DOS with an empty list (the AROS oracle's layout).
    let port = PROCESS + PR_MSGPORT;
    mem.write_u8(port + 8, NT_MSGPORT).map_err(m("pr_MsgPort.ln_Type"))?;
    mem.write_u8(port + 15, SIGB_DOS).map_err(m("pr_MsgPort.mp_SigBit"))?;
    mem.write_u32(port + 16, PROCESS).map_err(m("pr_MsgPort.mp_SigTask"))?;
    mem.write_u32(port + 20, port + 24).map_err(m("mp_MsgList.lh_Head"))?;
    mem.write_u32(port + 28, port + 20).map_err(m("mp_MsgList.lh_TailPred"))?;
    // The command name: a BSTR (length byte, then the bytes), longword aligned, referenced as a BPTR (address / 4).
    let n = name.as_bytes();
    if n.len() > 255 || COMMAND_NAME + 1 + n.len() as u32 > ARGS { return Err(Refusal(format!("command name too long: {name}"))); }
    mem.write_u8(COMMAND_NAME, n.len() as u8).map_err(m("cli_CommandName"))?;
    for (i, b) in n.iter().enumerate() { mem.write_u8(COMMAND_NAME + 1 + i as u32, *b).map_err(m("cli_CommandName"))?; }
    mem.write_u32(CLI + CLI_COMMAND_NAME, COMMAND_NAME >> 2).map_err(m("cli_CommandName"))?;
    mem.write_u32(PROCESS + PR_CLI, CLI >> 2).map_err(m("pr_CLI"))?;
    mem.write_u32(PROCESS + TC_NODE_NAME, COMMAND_NAME + 1).map_err(m("tc_Node.ln_Name"))?;
    for (i, b) in args.bytes().chain(core::iter::once(0)).enumerate() { mem.write_u8(ARGS + i as u32, b).map_err(m("argument line"))?; }
    mem.write_u32(exec_base + EXECBASE_THIS_TASK, PROCESS).map_err(m("ExecBase.ThisTask"))?;
    mem.write_u16(exec_base + EXECBASE_ATTN_FLAGS, AFF_68010_68020).map_err(m("ExecBase.AttnFlags"))?;
    Ok(PROCESS)
}

/// FindTask(a1 = name): NULL means "this task" (the program's Process). A named task is not modelled: one task.
pub fn find_task<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let name = cpu.areg(1);
    if name != 0 {
        let n = a.mem.read_cstr(name).map_err(Refusal::mem("FindTask.name"))?;
        cpu.set_d0(0);
        return Ok(format!("[exec] FindTask name=\"{n}\" -> 0 (only the program's own task exists)"));
    }
    cpu.set_d0(PROCESS);
    Ok(format!("[exec] FindTask NULL -> {PROCESS:#x} (this Process)"))
}
