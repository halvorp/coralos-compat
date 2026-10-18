//! dos.library, the first calls real C startups and CLI tools make: the program's name, its console streams and the
//! secondary error. The console is the CLI's: stdout bytes are collected (and logged line by line); stdin reads EOF.
//!
//! The streams are real, zeroed `struct FileHandle`s in guest RAM (dos/dosextens.h, 44 bytes) referenced as BPTRs
//! from pr_CIS / pr_COS, so a program that inspects them reads a plausible structure, not a magic number.
use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;
use crate::{machine::Amiga, process, Refusal};
use crate::fs::{DOSTRUE, FH_SIZE};
use guestui_abi::GuestUi;
use retro_guest_mem::GuestMem;
use retro_m68k::Cpu;

pub const LVO_READ: i32 = -42;
pub const LVO_WRITE: i32 = -48;
pub const LVO_INPUT: i32 = -54;
pub const LVO_OUTPUT: i32 = -60;
pub const LVO_IO_ERR: i32 = -132;
pub const LVO_DELAY: i32 = -198;
pub const LVO_FPUTS: i32 = -342;
pub const LVO_FLUSH: i32 = -360;
pub const LVO_SET_IO_ERR: i32 = -462;
pub const LVO_CLI: i32 = -492;
pub const LVO_GET_PROGRAM_NAME: i32 = -576;
pub const LVO_PUT_STR: i32 = -948;

/// The two console FileHandles (44 bytes each, longword aligned: a BPTR is the address / 4).
pub const STDIN_FH: u32 = 0x6300;
pub const STDOUT_FH: u32 = 0x6340;
/// Process fields this module owns the meaning of (offsets: see process.rs).
use crate::dos_err::PR_RESULT2;
const PR_CIS: u32 = 156;
const PR_COS: u32 = 160;
const PR_CLI: u32 = 172;
const CLI_COMMAND_NAME: u32 = 16;

/// Zero the two FileHandles and publish them as the Process's console streams.
pub fn setup(mem: &mut GuestMem) -> Result<(), Refusal> {
    for a in STDIN_FH..STDOUT_FH + FH_SIZE { mem.write_u8(a, 0).map_err(Refusal::mem("console FileHandle"))?; }
    mem.write_u32(process::PROCESS + PR_CIS, STDIN_FH >> 2).map_err(Refusal::mem("pr_CIS"))?;
    mem.write_u32(process::PROCESS + PR_COS, STDOUT_FH >> 2).map_err(Refusal::mem("pr_COS"))?;
    Ok(())
}

/// Everything the program wrote to its console, in order.
#[derive(Default)]
pub struct Console { pub stdout: Vec<u8> }

pub(crate) fn emit<U: GuestUi>(a: &mut Amiga<U>, bytes: &[u8]) -> String {
    a.console.stdout.extend_from_slice(bytes);
    String::from_utf8_lossy(bytes).trim_end_matches('\n').replace('\n', "\\n")
}

fn read_bytes<U: GuestUi>(a: &Amiga<U>, at: u32, n: u32, what: &str) -> Result<Vec<u8>, Refusal> {
    (0..n).map(|i| a.mem.read_u8(at + i).map_err(Refusal::mem(what))).collect()
}

/// Input() -> the Process's pr_CIS: the console until SelectInput() redirects it.
pub fn input<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let fh = a.mem.read_u32(process::PROCESS + PR_CIS).map_err(Refusal::mem("pr_CIS"))?;
    cpu.set_d0(fh);
    Ok(if fh == STDIN_FH >> 2 { "[dos] Input -> console".into() } else { format!("[dos] Input -> {:#x} (redirected)", fh << 2) })
}

/// Output() -> the Process's pr_COS: the console until SelectOutput() redirects it.
pub fn output<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let fh = a.mem.read_u32(process::PROCESS + PR_COS).map_err(Refusal::mem("pr_COS"))?;
    cpu.set_d0(fh);
    Ok(if fh == STDOUT_FH >> 2 { "[dos] Output -> console".into() } else { format!("[dos] Output -> {:#x} (redirected)", fh << 2) })
}

/// Write(d1 = file, d2 = buffer, d3 = length) -> bytes written, or -1. Only the console is a file so far.
pub fn write<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (fh, buf, n) = (cpu.dreg(1), cpu.dreg(2), cpu.dreg(3));
    if fh != STDOUT_FH >> 2 { return crate::dos_io::write(a, cpu); }
    let bytes = read_bytes(a, buf, n, "Write.buffer")?;
    let shown = emit(a, &bytes);
    cpu.set_d0(n);
    Ok(format!("[dos] Write console \"{shown}\""))
}

/// Read(d1 = file, d2 = buffer, d3 = length): the console's stdin has nothing to give (EOF, 0).
pub fn read<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    if cpu.dreg(1) != STDIN_FH >> 2 { return crate::dos_io::read(a, cpu); }
    cpu.set_d0(0);
    Ok("[dos] Read console -> 0 (EOF)".into())
}

/// PutStr(d1 = string) -> 0 on success: the string to Output().
pub fn put_str<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let s = a.mem.read_cstr(cpu.dreg(1)).map_err(Refusal::mem("PutStr.str"))?;
    let shown = emit(a, s.as_bytes());
    cpu.set_d0(0);
    Ok(format!("[dos] PutStr \"{shown}\""))
}

/// FPuts(d1 = file, d2 = string) -> 0 on success.
pub fn fputs<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    if cpu.dreg(1) != STDOUT_FH >> 2 { return crate::dos_io::fputs(a, cpu); }
    let s = a.mem.read_cstr(cpu.dreg(2)).map_err(Refusal::mem("FPuts.str"))?;
    let shown = emit(a, s.as_bytes());
    cpu.set_d0(0);
    Ok(format!("[dos] FPuts console \"{shown}\""))
}

pub fn flush<U: GuestUi>(_: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    cpu.set_d0(DOSTRUE);
    Ok("[dos] Flush".into())
}

/// IoErr() -> the Process's pr_Result2.
pub fn io_err<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let v = a.mem.read_u32(process::PROCESS + PR_RESULT2).map_err(Refusal::mem("pr_Result2"))?;
    cpu.set_d0(v);
    Ok(format!("[dos] IoErr -> {v}"))
}

/// SetIoErr(d1 = code) -> the previous code.
pub fn set_io_err<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let at = process::PROCESS + PR_RESULT2;
    let old = a.mem.read_u32(at).map_err(Refusal::mem("pr_Result2"))?;
    a.mem.write_u32(at, cpu.dreg(1)).map_err(Refusal::mem("pr_Result2"))?;
    cpu.set_d0(old);
    Ok(format!("[dos] SetIoErr {} (was {old})", cpu.dreg(1)))
}

/// Cli() -> the CommandLineInterface (an APTR, not a BPTR).
pub fn cli<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let b = a.mem.read_u32(process::PROCESS + PR_CLI).map_err(Refusal::mem("pr_CLI"))?;
    cpu.set_d0(b << 2);
    Ok(format!("[dos] Cli -> {:#x}", b << 2))
}

/// GetProgramName(d1 = buffer, d2 = length) -> DOSTRUE: the CLI's cli_CommandName as a C string, truncated to fit.
pub fn get_program_name<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (buf, len) = (cpu.dreg(1), cpu.dreg(2));
    let cli = a.mem.read_u32(process::PROCESS + PR_CLI).map_err(Refusal::mem("pr_CLI"))? << 2;
    let bstr = a.mem.read_u32(cli + CLI_COMMAND_NAME).map_err(Refusal::mem("cli_CommandName"))? << 2;
    let n = a.mem.read_u8(bstr).map_err(Refusal::mem("cli_CommandName"))? as u32;
    if len == 0 { cpu.set_d0(0); return Ok("[dos] GetProgramName buffer of 0 bytes -> FALSE".into()); }
    let take = n.min(len - 1);
    let name = read_bytes(a, bstr + 1, take, "cli_CommandName")?;
    for (i, b) in name.iter().chain(core::iter::once(&0)).enumerate() { a.mem.write_u8(buf + i as u32, *b).map_err(Refusal::mem("GetProgramName.buffer"))?; }
    cpu.set_d0(DOSTRUE);
    Ok(format!("[dos] GetProgramName -> \"{}\"", String::from_utf8_lossy(&name)))
}

/// Delay(d1 = ticks): one task and no clock source yet, so it returns at once (logged so a timing-dependent program
/// shows why it misbehaves).
pub fn delay<U: GuestUi>(_: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    Ok(format!("[dos] Delay {} ticks (not modelled: returns at once)", cpu.dreg(1)))
}

pub const LVO_IS_INTERACTIVE: i32 = -216;

/// IsInteractive(d1 = file) -> DOSTRUE for the console streams (a CLI window is interactive), FALSE otherwise.
pub fn is_interactive<U: GuestUi>(_: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let fh = cpu.dreg(1);
    let yes = fh == STDIN_FH >> 2 || fh == STDOUT_FH >> 2;
    cpu.set_d0(if yes { DOSTRUE } else { 0 });
    Ok(format!("[dos] IsInteractive {fh:#x} -> {yes}"))
}
