//! Process-level dos.library services of the RUNNING process (`Amiga::sched.cur`): the standard streams a program can redirect, the
//! argument string, the console and filesystem task ports, signals, console modes, record locks, parent of a handle.
//!
//! Offsets: m68k gcc -m68020 -mno-align-int over AROS dosextens.h (see process.rs): Process.pr_ConsoleTask 164,
//! pr_FileSystemTask 168, pr_CIS 156, pr_COS 160, pr_Arguments 204; Task.tc_SigRecvd 26. There is exactly ONE CLI
//! (number 1; processes made with NP_Cli are not entered in the root's CLI table), so MaxCli is 1. Record locks are
//! always granted: contention between tasks is not enforced, and the log says so.
use alloc::format;
use alloc::string::String;
use crate::dos_err::*;
use crate::fs::*;
use crate::libtable::LibraryBuilder;
use crate::{dos, machine::Amiga, process, Refusal};
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

const PR_CIS: u32 = 156;
const PR_COS: u32 = 160;
const PR_CONSOLE_TASK: u32 = 164;
const PR_FILESYSTEM_TASK: u32 = 168;
const PR_ARGUMENTS: u32 = 204;
const TC_SIG_RECVD: u32 = 26;
const CHANGE_LOCK: u32 = 0;
const CHANGE_FH: u32 = 1;
const EXCLUSIVE_LOCK: i32 = -1;
const SHARED_LOCK: i32 = -2;

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.named("SelectInput", select_input::<U>).named("SelectOutput", select_output::<U>)
        .named("GetArgStr", get_arg_str::<U>).named("SetArgStr", set_arg_str::<U>)
        .named("GetConsoleTask", get_console_task::<U>).named("SetConsoleTask", set_console_task::<U>)
        .named("GetFileSysTask", get_fs_task::<U>).named("SetFileSysTask", set_fs_task::<U>)
        .named("MaxCli", max_cli::<U>).named("FindCliProc", find_cli_proc::<U>).named("CheckSignal", check_signal::<U>)
        .named("ParentOfFH", parent_of_fh::<U>).named("SetMode", set_mode::<U>).named("WaitForChar", wait_for_char::<U>)
        .named("ChangeMode", change_mode::<U>).named("ReadLink", read_link::<U>)
        .named("LockRecord", record_lock::<U>).named("LockRecords", record_lock::<U>).named("UnLockRecord", record_lock::<U>).named("UnLockRecords", record_lock::<U>)
}

fn field<U: GuestUi>(a: &Amiga<U>, off: u32) -> Result<u32, Refusal> { a.mem.read_u32(a.sched.cur + off).map_err(Refusal::mem("Process")) }
fn set_field<U: GuestUi>(a: &mut Amiga<U>, off: u32, v: u32) -> Result<(), Refusal> { a.mem.write_u32(a.sched.cur + off, v).map_err(Refusal::mem("Process")) }

/// Swap a Process stream field, returning the previous value. Input()/Output() read these fields.
fn select<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, off: u32, name: &str) -> Result<String, Refusal> {
    let new = cpu.dreg(1);
    let old = field(a, off)?;
    set_field(a, off, new)?;
    cpu.set_d0(old);
    Ok(format!("[dos] {name} {:#x} (was {:#x})", new << 2, old << 2))
}
fn select_input<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> { select(a, cpu, PR_CIS, "SelectInput") }
fn select_output<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> { select(a, cpu, PR_COS, "SelectOutput") }

/// GetArgStr() -> the argument line (the CLI's, until SetArgStr changes it).
fn get_arg_str<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let p = match field(a, PR_ARGUMENTS)? { 0 => process::ARGS, p => p };
    cpu.set_d0(p);
    Ok(format!("[dos] GetArgStr -> {p:#x}"))
}
/// SetArgStr(d1 = string) -> the previous argument string.
fn set_arg_str<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let old = match field(a, PR_ARGUMENTS)? { 0 => process::ARGS, p => p };
    set_field(a, PR_ARGUMENTS, cpu.dreg(1))?;
    cpu.set_d0(old);
    Ok(format!("[dos] SetArgStr {:#x}", cpu.dreg(1)))
}

/// The console handler / filesystem handler ports are private MsgPorts nothing serves (packets are not modelled).
fn get_task<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, off: u32, dev: &str) -> Result<String, Refusal> {
    let p = match field(a, off)? { 0 => crate::doslist::device_port(a, dev)?, p => p };
    cpu.set_d0(p);
    Ok(format!("[dos] Get{dev}Task -> {p:#x} (a port no handler serves)"))
}
fn set_task<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, off: u32, dev: &str) -> Result<String, Refusal> {
    let old = match field(a, off)? { 0 => crate::doslist::device_port(a, dev)?, p => p };
    set_field(a, off, cpu.dreg(1))?;
    cpu.set_d0(old);
    Ok(format!("[dos] Set{dev}Task {:#x}", cpu.dreg(1)))
}
fn get_console_task<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> { get_task(a, cpu, PR_CONSOLE_TASK, "CON") }
fn set_console_task<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> { set_task(a, cpu, PR_CONSOLE_TASK, "CON") }
fn get_fs_task<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> { get_task(a, cpu, PR_FILESYSTEM_TASK, "SYS") }
fn set_fs_task<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> { set_task(a, cpu, PR_FILESYSTEM_TASK, "SYS") }

fn max_cli<U: GuestUi>(_: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> { cpu.set_d0(1); Ok("[dos] MaxCli -> 1 (one CLI)".into()) }
fn find_cli_proc<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let n = cpu.dreg(1);
    cpu.set_d0(if n == 1 { a.sched.cur } else { 0 });
    Ok(format!("[dos] FindCliProc {n}"))
}

/// CheckSignal(d1 = mask) -> the masked signals that were pending, which it clears.
fn check_signal<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let mask = cpu.dreg(1);
    let got = field(a, TC_SIG_RECVD)? & mask;
    let rest = field(a, TC_SIG_RECVD)? & !mask;
    set_field(a, TC_SIG_RECVD, rest)?;
    cpu.set_d0(got);
    Ok(format!("[dos] CheckSignal {mask:#x} -> {got:#x}"))
}

/// ParentOfFH(d1 = file) -> a lock on the directory the file is in.
fn parent_of_fh<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let b = cpu.dreg(1);
    let node = crate::dos_io::handle(a, b, "ParentOfFH")?.node;
    let what = format!("[dos] ParentOfFH {:#x}", b << 2);
    match a.fs.vol.parent_of(node) {
        Err(e) => fail(a, cpu, e, 0, what),
        Ok(p) => match new_lock(a, p, false)? { Some(l) => ok(a, cpu, l >> 2, what), None => fail(a, cpu, ERROR_NO_FREE_STORE, 0, what) },
    }
}

fn is_console(b: u32) -> bool { b == dos::STDIN_FH >> 2 || b == dos::STDOUT_FH >> 2 }

/// SetMode(d1 = file, d2 = mode): the console accepts raw (1) and cooked (0); anything else is not interactive.
fn set_mode<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (b, mode) = (cpu.dreg(1), cpu.dreg(2));
    let what = format!("[dos] SetMode {:#x} {mode}", b << 2);
    if is_console(b) { ok(a, cpu, DOSTRUE, format!("{what} (console; the engine's console is always line-at-a-time)")) } else { fail(a, cpu, ERROR_OBJECT_WRONG_TYPE, 0, what) }
}

/// WaitForChar(d1 = file, d2 = microseconds) -> TRUE if a character can be read: the console has no input, a file has
/// data until its end.
fn wait_for_char<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let b = cpu.dreg(1);
    let ready = if is_console(b) { false } else {
        let f = crate::dos_io::handle(a, b, "WaitForChar")?;
        let (node, pos) = (f.node, f.pos);
        a.fs.vol.file_data(node).map_or(false, |d| (pos as usize) < d.len())
    };
    cpu.set_d0(if ready { DOSTRUE } else { 0 });
    Ok(format!("[dos] WaitForChar {:#x} -> {ready}", b << 2))
}

/// ChangeMode(d1 = CHANGE_LOCK|CHANGE_FH, d2 = object, d3 = new mode): a lock becomes shared/exclusive unless another
/// lock or handle holds the object; a file handle's mode change is accepted (the volume has no per-handle kernel state).
fn change_mode<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (ty, obj, mode) = (cpu.dreg(1), cpu.dreg(2), cpu.dreg(3) as i32);
    let what = format!("[dos] ChangeMode type {ty} {:#x} -> {mode}", obj << 2);
    match ty {
        CHANGE_LOCK => {
            let Some(rec) = a.fs.locks.get(&(obj << 2)) else { return fail(a, cpu, ERROR_INVALID_LOCK, 0, what) };
            let node = rec.node;
            if mode != EXCLUSIVE_LOCK && mode != SHARED_LOCK { return fail(a, cpu, ERROR_BAD_NUMBER, 0, what); }
            let others = a.fs.locks.iter().any(|(&k, l)| k != obj << 2 && l.node == node) || a.fs.files.values().any(|f| f.node == node);
            if mode == EXCLUSIVE_LOCK && others { return fail(a, cpu, ERROR_OBJECT_IN_USE, 0, what); }
            a.fs.locks.get_mut(&(obj << 2)).expect("lock").exclusive = mode == EXCLUSIVE_LOCK;
            ok(a, cpu, DOSTRUE, what)
        }
        CHANGE_FH => { crate::dos_io::handle(a, obj, "ChangeMode")?; ok(a, cpu, DOSTRUE, what) }
        t => Err(Refusal(format!("ChangeMode: type {t} is neither CHANGE_LOCK nor CHANGE_FH"))),
    }
}
const ERROR_BAD_NUMBER: DosErr = DosErr(115);

/// ReadLink(d1 = port, d2 = lock, d3 = path, d4 = buffer, d5 = size): the volume has no soft links, so a path that
/// exists is "not a link" (ERROR_OBJECT_WRONG_TYPE) and one that does not is ERROR_OBJECT_NOT_FOUND; result -1.
fn read_link<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (lock, path) = (cpu.dreg(2), str_arg(a, cpu.dreg(3), "ReadLink.path")?);
    let what = format!("[dos] ReadLink \"{path}\"");
    let base = match lock_node(a, lock) { Ok(n) => n, Err(e) => return fail(a, cpu, e, u32::MAX, what) };
    match a.fs.vol.resolve(base, &path) { Ok(_) => fail(a, cpu, ERROR_OBJECT_WRONG_TYPE, u32::MAX, what), Err(e) => fail(a, cpu, e, u32::MAX, what) }
}

/// LockRecord / LockRecords / UnLockRecord / UnLockRecords: record contention between tasks is not
/// enforced, so every request succeeds (logged, so a program relying on contention shows why it never sees any).
fn record_lock<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    ok(a, cpu, DOSTRUE, "[dos] record lock request granted (record contention between tasks is not enforced)".into())
}
