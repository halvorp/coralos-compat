//! dos.library process creation: CreateNewProc / CreateProc / Exit, and what a process owns until it ends.
//!
//! A new process gets a REAL `struct Process` (228 bytes, dos/dosextens.h; offsets as in process.rs), its own stack, a
//! `pr_MsgPort` on SIGB_DOS (the DOpus IPC startup message arrives there: Library/ipc.c:119-186 of the DOpus 5 source
//! CreateNewProc's with NP_Entry/NP_Name/NP_WindowPtr/NP_StackSize/NP_Priority [+ NP_Cli/NP_Path], PutMsg's an IPC_STARTUP
//! message to `proc->pr_MsgPort` and WaitPort's on its own reply port; the child does FindTask(0), WaitPort(&pr_MsgPort),
//! GetMsg, stores tc_UserData, CreateMsgPort x2 and ReplyMsg), a seglist array in `pr_SegList`, and the streams,
//! directory locks and arguments the tags gave it. It starts at NP_Entry (or the first segment, `BADDR(seglist)+4`) with
//! A0 = the argument string and D0 = its length, returning into the task finalizer (exec_task.rs), which calls
//! NP_ExitCode (D0 = return code, D1 = NP_ExitData, as AROS's DosEntry does) and then frees, per the NP_Free*/NP_Close*
//! tags, what the process owns: `Spec` below lists the tags; the defaults follow AROS rom/dos/createnewproc.c.
//!
//! Differences from AROS, all named: a stream tag left out gives BNULL (the NIL: handle) instead of an opened NIL:;
//! NP_Synchronous, NP_NotifyOnDeath and NP_Affinity are refused by name when asked for; CreateProc keeps its seglist
//! (AROS createproc.c passes NP_FreeSeglist FALSE).
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use crate::dos_err::*;
use crate::fs::*;
use crate::guest_call::Next;
use crate::libtable::LibraryBuilder;
use crate::machine::Amiga;
use crate::sched::{LN_PRI, Start};
use crate::{execlists, tags, Refusal};
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

const NP_DUMMY: u32 = tags::TAG_USER + 1000;
const NP_SEGLIST: u32 = NP_DUMMY + 1;
const NP_FREE_SEGLIST: u32 = NP_DUMMY + 2;
const NP_ENTRY: u32 = NP_DUMMY + 3;
const NP_INPUT: u32 = NP_DUMMY + 4;
const NP_OUTPUT: u32 = NP_DUMMY + 5;
const NP_CLOSE_INPUT: u32 = NP_DUMMY + 6;
const NP_CLOSE_OUTPUT: u32 = NP_DUMMY + 7;
const NP_ERROR: u32 = NP_DUMMY + 8;
const NP_CLOSE_ERROR: u32 = NP_DUMMY + 9;
const NP_CURRENT_DIR: u32 = NP_DUMMY + 10;
const NP_STACK_SIZE: u32 = NP_DUMMY + 11;
const NP_NAME: u32 = NP_DUMMY + 12;
const NP_PRIORITY: u32 = NP_DUMMY + 13;
const NP_CONSOLE_TASK: u32 = NP_DUMMY + 14;
const NP_WINDOW_PTR: u32 = NP_DUMMY + 15;
const NP_HOME_DIR: u32 = NP_DUMMY + 16;
const NP_COPY_VARS: u32 = NP_DUMMY + 17;
const NP_CLI: u32 = NP_DUMMY + 18;
const NP_PATH: u32 = NP_DUMMY + 19;
const NP_ARGUMENTS: u32 = NP_DUMMY + 21;
const NP_NOTIFY_ON_DEATH: u32 = NP_DUMMY + 22;
const NP_SYNCHRONOUS: u32 = NP_DUMMY + 23;
const NP_EXIT_CODE: u32 = NP_DUMMY + 24;
const NP_EXIT_DATA: u32 = NP_DUMMY + 25;
const NP_USER_DATA: u32 = NP_DUMMY + 26;
const NP_AFFINITY: u32 = NP_DUMMY + 27;

/// Process offsets (see process.rs for how they were computed): 92 pr_MsgPort, 128 pr_SegList, 132 pr_StackSize,
/// 144 pr_StackBase, 152 pr_CurrentDir, 156 pr_CIS, 160 pr_COS, 164 pr_ConsoleTask, 168 pr_FileSystemTask, 172 pr_CLI,
/// 184 pr_WindowPtr, 188 pr_HomeDir, 192 pr_Flags, 196 pr_ExitCode, 200 pr_ExitData, 204 pr_Arguments, 224 pr_CES.
const PROCESS_SIZE: u32 = 228;
const PR_SEGLIST: u32 = 128;
const PR_STACKSIZE: u32 = 132;
const PR_STACKBASE: u32 = 144;
const PR_CURRENT_DIR: u32 = 152;
const PR_CIS: u32 = 156;
const PR_COS: u32 = 160;
const PR_CONSOLE_TASK: u32 = 164;
const PR_FILESYSTEM_TASK: u32 = 168;
const PR_CLI: u32 = 172;
const PR_WINDOW_PTR: u32 = 184;
const PR_HOME_DIR: u32 = 188;
const PR_EXIT_CODE: u32 = 196;
const PR_EXIT_DATA: u32 = 200;
const PR_ARGUMENTS: u32 = 204;
const PR_CES: u32 = 224;
const TC_USER_DATA: u32 = 88;
const TC_MEM_ENTRY: u32 = 74;
/// CommandLineInterface (dos/dosextens.h): cli_CommandDir 8, cli_CommandName 16; 64 bytes as process.rs allocates.
const CLI_SIZE: u32 = 64;
const CLI_COMMAND_DIR: u32 = 8;
const CLI_COMMAND_NAME: u32 = 16;
const MIN_STACK: u32 = 4096;
const NT_PROCESS: u8 = 13;
const TS_READY: u8 = 3;
const SIGB_DOS: u8 = 8;
const ERROR_REQUIRED_ARG_MISSING: DosErr = DosErr(116);
const ERROR_BAD_NUMBER: DosErr = DosErr(115);

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.named("CreateNewProc", create_new_proc::<U>).named("CreateProc", create_proc::<U>).named("Exit", exit::<U>).with(crate::dos_notify::register::<U>).named("SetProgramName", crate::dos::set_program_name::<U>)
}

/// What a created process owns until it ends (host-side: the guest cannot corrupt it).
#[derive(Default)]
pub struct ProcOwn {
    /// Guest heap blocks (address, size): the Process, its name, stack, seglist array, CLI, argument line.
    blocks: Vec<(u32, u32)>,
    close_in: bool,
    close_out: bool,
    close_err: bool,
    /// A seglist to UnLoadSeg at exit (NP_FreeSeglist), 0 = none.
    free_seglist: u32,
    /// pr_CurrentDir is a lock the process owns (UnLocked at exit).
    free_curdir: bool,
    exit_code: u32,
    exit_data: u32,
}

/// Every tag, with AROS's defaults (rom/dos/createnewproc.c `defaults[]`).
struct Spec {
    seglist: u32, entry: u32, name: String, pri: Option<i8>, stack: u32, free_seglist: bool,
    input: Option<u32>, output: Option<u32>, error: Option<u32>, close_in: bool, close_out: bool, close_err: bool,
    curdir: Option<u32>, home: Option<u32>, copy_vars: bool, cli: bool, path: u32, args: Option<u32>,
    exit_code: u32, exit_data: u32, console: Option<u32>, window_ptr: Option<u32>, user_data: u32,
}

enum Fail { Dos(DosErr), Refused(Refusal) }
impl From<Refusal> for Fail { fn from(r: Refusal) -> Self { Fail::Refused(r) } }
impl From<DosErr> for Fail { fn from(e: DosErr) -> Self { Fail::Dos(e) } }

fn spec_from_tags<U: GuestUi>(a: &Amiga<U>, list: u32) -> Result<Spec, Fail> {
    let t = tags::walk(&a.mem, list)?;
    let g = |tag| tags::get(&t, tag);
    let flag = |tag, default: bool| g(tag).map_or(default, |v| v != 0);
    for (tag, what) in [(NP_SYNCHRONOUS, "NP_Synchronous"), (NP_NOTIFY_ON_DEATH, "NP_NotifyOnDeath")] {
        if flag(tag, false) { return Err(Fail::Refused(Refusal(format!("CreateNewProc: {what} is not modelled (a parent that waits for its child's death)")))); }
    }
    if g(NP_AFFINITY).unwrap_or(0) != 0 { return Err(Fail::Refused(Refusal("CreateNewProc: NP_Affinity: there is one CPU".into()))); }
    let name = match g(NP_NAME) { Some(p) if p != 0 => a.mem.read_cstr(p).map_err(Refusal::mem("NP_Name"))?, _ => "New Process".into() };
    Ok(Spec {
        seglist: g(NP_SEGLIST).unwrap_or(0), entry: g(NP_ENTRY).unwrap_or(0), name, pri: g(NP_PRIORITY).map(|p| p as i8),
        stack: g(NP_STACK_SIZE).unwrap_or(MIN_STACK), free_seglist: flag(NP_FREE_SEGLIST, true),
        input: g(NP_INPUT), output: g(NP_OUTPUT), error: g(NP_ERROR),
        close_in: flag(NP_CLOSE_INPUT, true), close_out: flag(NP_CLOSE_OUTPUT, true), close_err: flag(NP_CLOSE_ERROR, true),
        curdir: g(NP_CURRENT_DIR), home: g(NP_HOME_DIR), copy_vars: flag(NP_COPY_VARS, true), cli: flag(NP_CLI, false),
        path: g(NP_PATH).unwrap_or(0), args: g(NP_ARGUMENTS), exit_code: g(NP_EXIT_CODE).unwrap_or(0), exit_data: g(NP_EXIT_DATA).unwrap_or(0),
        console: g(NP_CONSOLE_TASK), window_ptr: g(NP_WINDOW_PTR), user_data: g(NP_USER_DATA).unwrap_or(0),
    })
}

/// A zeroed block the process owns.
fn grab<U: GuestUi>(a: &mut Amiga<U>, own: &mut ProcOwn, size: u32) -> Result<u32, Fail> {
    let size = (size + 3) & !3;
    let p = a.heap.alloc(size).ok_or(DosErr(ERROR_NO_FREE_STORE.0))?;
    own.blocks.push((p, size));
    a.mem.zero(p, size).map_err(Refusal::mem("process block"))?;
    Ok(p)
}

/// DupLock of a lock BPTR (0 stays 0).
fn dup_lock_bptr<U: GuestUi>(a: &mut Amiga<U>, b: u32) -> Result<u32, Fail> {
    if b == 0 { return Ok(0); }
    let node = lock_node(a, b)?;
    Ok(new_lock(a, node, false)?.ok_or(ERROR_NO_FREE_STORE)? >> 2)
}

fn build<U: GuestUi>(a: &mut Amiga<U>, own: &mut ProcOwn, s: &Spec) -> Result<u32, Fail> {
    let m = Refusal::mem;
    if s.seglist == 0 && s.entry == 0 { return Err(Fail::Dos(ERROR_REQUIRED_ARG_MISSING)); }
    if s.seglist != 0 && !a.fs.segs.contains_key(&s.seglist) {
        return Err(Fail::Refused(Refusal(format!("CreateNewProc: NP_Seglist {:#x} is not a seglist LoadSeg returned", s.seglist << 2))));
    }
    let parent = a.sched.cur;
    let entry = if s.entry != 0 { s.entry } else { (s.seglist << 2) + 4 };
    let stack_size = ((s.stack.max(MIN_STACK)) + 3) & !3;
    let p = grab(a, own, PROCESS_SIZE)?;
    let stack = grab(a, own, stack_size)?;
    let name = grab(a, own, s.name.len() as u32 + 1)?;
    a.mem.write_bytes(name, s.name.as_bytes()).map_err(m("NP_Name"))?;
    let upper = stack + stack_size;
    let rd = |a: &Amiga<U>, off| a.mem.read_u32(parent + off).map_err(Refusal::mem("parent Process"));
    // Task part.
    a.mem.write_u8(p + 8, NT_PROCESS).map_err(m("ln_Type"))?;
    a.mem.write_u8(p + LN_PRI, s.pri.unwrap_or(a.task_pri(parent)) as u8).map_err(m("ln_Pri"))?;
    a.mem.write_u32(p + 10, name).map_err(m("ln_Name"))?;
    a.mem.write_u8(p + 15, TS_READY).map_err(m("tc_State"))?;
    a.mem.write_u32(p + 18, crate::exec_task::SYSTEM_SIGNALS).map_err(m("tc_SigAlloc"))?;
    a.mem.write_u32(p + 58, stack).map_err(m("tc_SPLower"))?;
    a.mem.write_u32(p + 62, upper).map_err(m("tc_SPUpper"))?;
    execlists::new_list(&mut a.mem, p + TC_MEM_ENTRY, 0)?;
    a.mem.write_u32(p + TC_USER_DATA, s.user_data).map_err(m("tc_UserData"))?;
    // pr_MsgPort: a signalling port of this task on SIGB_DOS with an empty list (process.rs / the AROS oracle).
    let port = p + 92;
    a.mem.write_u8(port + 8, 4).map_err(m("pr_MsgPort.ln_Type"))?;
    a.mem.write_u8(port + 15, SIGB_DOS).map_err(m("pr_MsgPort.mp_SigBit"))?;
    a.mem.write_u32(port + 16, p).map_err(m("pr_MsgPort.mp_SigTask"))?;
    a.mem.write_u32(port + 20, port + 24).map_err(m("mp_MsgList"))?;
    a.mem.write_u32(port + 28, port + 20).map_err(m("mp_MsgList"))?;
    // Seglist array: [4, -1, -2, seglist] as AROS builds it; BPTR to it.
    let arr = grab(a, own, 16)?;
    for (i, v) in [4u32, u32::MAX, u32::MAX - 1, s.seglist].into_iter().enumerate() { a.mem.write_u32(arr + 4 * i as u32, v).map_err(m("segarray"))?; }
    a.mem.write_u32(p + PR_SEGLIST, arr >> 2).map_err(m("pr_SegList"))?;
    a.mem.write_u32(p + PR_STACKSIZE, stack_size).map_err(m("pr_StackSize"))?;
    a.mem.write_u32(p + PR_STACKBASE, upper >> 2).map_err(m("pr_StackBase"))?;
    // Streams and directories.
    a.mem.write_u32(p + PR_CIS, s.input.unwrap_or(0)).map_err(m("pr_CIS"))?;
    a.mem.write_u32(p + PR_COS, s.output.unwrap_or(0)).map_err(m("pr_COS"))?;
    a.mem.write_u32(p + PR_CES, s.error.unwrap_or(0)).map_err(m("pr_CES"))?;
    own.close_in = s.close_in && s.input.unwrap_or(0) != 0;
    own.close_out = s.close_out && s.output.unwrap_or(0) != 0;
    own.close_err = s.close_err && s.error.unwrap_or(0) != 0;
    let cur = match s.curdir { Some(b) => b, None => { let c = rd(a, PR_CURRENT_DIR)?; dup_lock_bptr(a, c)? } };
    a.mem.write_u32(p + PR_CURRENT_DIR, cur).map_err(m("pr_CurrentDir"))?;
    own.free_curdir = cur != 0;
    let home = match s.home { Some(b) => b, None => { let h = rd(a, PR_HOME_DIR)?; dup_lock_bptr(a, h)? } };
    a.mem.write_u32(p + PR_HOME_DIR, home).map_err(m("pr_HomeDir"))?;
    let console = match s.console { Some(c) => c, None => rd(a, PR_CONSOLE_TASK)? };
    a.mem.write_u32(p + PR_CONSOLE_TASK, console).map_err(m("pr_ConsoleTask"))?;
    let fst = rd(a, PR_FILESYSTEM_TASK)?;
    a.mem.write_u32(p + PR_FILESYSTEM_TASK, fst).map_err(m("pr_FileSystemTask"))?;
    let win = match s.window_ptr { Some(w) => w, None => rd(a, PR_WINDOW_PTR)? };
    a.mem.write_u32(p + PR_WINDOW_PTR, win).map_err(m("pr_WindowPtr"))?;
    // Arguments, CLI.
    let (args, arglen) = match s.args {
        None => (0, 0),
        Some(ptr) => {
            let text = if ptr == 0 { String::new() } else { a.mem.read_cstr(ptr).map_err(m("NP_Arguments"))? };
            let blk = grab(a, own, text.len() as u32 + 1)?;
            a.mem.write_bytes(blk, text.as_bytes()).map_err(m("NP_Arguments"))?;
            (blk, text.len() as u32)
        }
    };
    a.mem.write_u32(p + PR_ARGUMENTS, args).map_err(m("pr_Arguments"))?;
    if s.cli {
        let cli = grab(a, own, CLI_SIZE)?;
        let bstr = grab(a, own, crate::process::CLI_NAME_MAX as u32 + 1)?; // room for SetProgramName
        if s.name.len() > crate::process::CLI_NAME_MAX { return Err(Fail::Dos(ERROR_BAD_NUMBER)); }
        a.mem.write_u8(bstr, s.name.len() as u8).map_err(m("cli_CommandName"))?;
        a.mem.write_bytes(bstr + 1, s.name.as_bytes()).map_err(m("cli_CommandName"))?;
        a.mem.write_u32(cli + CLI_COMMAND_NAME, bstr >> 2).map_err(m("cli_CommandName"))?;
        a.mem.write_u32(cli + CLI_COMMAND_DIR, s.path).map_err(m("cli_CommandDir"))?;
        a.mem.write_u32(p + PR_CLI, cli >> 2).map_err(m("pr_CLI"))?;
    }
    a.mem.write_u32(p + PR_EXIT_CODE, s.exit_code).map_err(m("pr_ExitCode"))?;
    a.mem.write_u32(p + PR_EXIT_DATA, s.exit_data).map_err(m("pr_ExitData"))?;
    own.exit_code = s.exit_code;
    own.exit_data = s.exit_data;
    if s.free_seglist && s.seglist != 0 { own.free_seglist = s.seglist; }
    // Initial stack: the entry function's return address is the task finalizer.
    let sp = upper - 4;
    a.mem.write_u32(sp, crate::exec_task::TASK_END).map_err(m("process stack"))?;
    a.mem.write_u32(p + 54, sp).map_err(m("tc_SPReg"))?;
    let regs = alloc::vec![(retro_m68k::REG_A0, args), (retro_m68k::REG_D0, arglen)];
    let owned = core::mem::take(own);
    a.add_sched_task(p, s.name.clone(), false, Start { pc: entry, sp, regs }, Some(owned));
    // NP_CopyVars: the child starts with copies of the parent's local variables.
    if s.copy_vars { copy_vars(a, p)?; }
    a.sync_task_lists()?;
    Ok(p)
}

/// Give the new process (`child`, not yet run) copies of the running task's local variables, built the way SetVar
/// would build them for it (real LocalVar nodes on the child's pr_LocalVars).
fn copy_vars<U: GuestUi>(a: &mut Amiga<U>, child: u32) -> Result<(), Refusal> {
    let snapshot: Vec<(Vec<u8>, u8, Vec<u8>)> = a.fs.vars.locals.iter().map(|r| (r.name.clone(), r.ty, r.value.clone())).collect();
    if snapshot.is_empty() { return Ok(()); }
    let (saved_cur, saved_vars) = (a.sched.cur, core::mem::take(&mut a.fs.vars));
    a.sched.cur = child;
    let mut res = Ok(());
    for (n, ty, v) in &snapshot {
        match crate::vars::set_local(a, n, *ty, v) { Ok(true) => {}, Ok(false) => { res = Err(Refusal("CreateNewProc: NP_CopyVars: out of memory".into())); break }, Err(e) => { res = Err(e); break } }
    }
    let child_vars = core::mem::replace(&mut a.fs.vars, saved_vars);
    a.sched.cur = saved_cur;
    a.sched.tasks.get_mut(&child).expect("the new process").vars = child_vars;
    res
}

/// Create the process; on a dos failure everything built so far is released and the DosErr returned.
fn spawn<U: GuestUi>(a: &mut Amiga<U>, s: &Spec) -> Result<Result<u32, DosErr>, Refusal> {
    let mut own = ProcOwn::default();
    match build(a, &mut own, s) {
        Ok(p) => Ok(Ok(p)),
        Err(Fail::Refused(r)) => { release_blocks(a, &own); Err(r) }
        Err(Fail::Dos(e)) => { release_blocks(a, &own); Ok(Err(e)) }
    }
}

fn release_blocks<U: GuestUi>(a: &mut Amiga<U>, own: &ProcOwn) {
    for &(p, n) in &own.blocks { if let Err(e) = a.heap.free(p, n) { a.log.push(format!("[dos] process block {p:#x}+{n} not freed: {e}")); } }
}

/// CreateNewProc(d1 = tags) -> the new Process, or NULL with IoErr.
fn create_new_proc<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let list = cpu.dreg(1);
    let s = match spec_from_tags(a, list) { Ok(s) => s, Err(Fail::Refused(r)) => return Err(r), Err(Fail::Dos(e)) => return fail(a, cpu, e, 0, "[dos] CreateNewProc".into()) };
    let what = format!("[dos] CreateNewProc \"{}\" entry={:#x} seglist={:#x} stack={} pri={:?}", s.name, if s.entry != 0 { s.entry } else { (s.seglist << 2) + 4 }, s.seglist << 2, s.stack, s.pri);
    match spawn(a, &s)? { Ok(p) => ok(a, cpu, p, format!("{what} -> {p:#x}")), Err(e) => fail(a, cpu, e, 0, what) }
}

/// CreateProc(d1 = name, d2 = priority, d3 = seglist, d4 = stackSize) -> the process's pr_MsgPort (its "process id"), or 0.
/// Tags as AROS rom/dos/createproc.c passes them: no streams, no current/home directory of its own, seglist kept.
fn create_proc<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (np, pri, seg, stack) = (cpu.dreg(1), cpu.dreg(2) as i8, cpu.dreg(3), cpu.dreg(4));
    let name = if np == 0 { "New Process".into() } else { str_arg(a, np, "CreateProc.name")? };
    let what = format!("[dos] CreateProc \"{name}\" pri {pri} seglist {:#x} stack {stack}", seg << 2);
    let s = Spec {
        seglist: seg, entry: 0, name, pri: Some(pri), stack, free_seglist: false, input: Some(0), output: Some(0), error: None,
        close_in: false, close_out: false, close_err: false, curdir: Some(0), home: Some(0), copy_vars: true, cli: false, path: 0,
        args: None, exit_code: 0, exit_data: 0, console: None, window_ptr: None, user_data: 0,
    };
    if seg == 0 { return fail(a, cpu, ERROR_BAD_NUMBER, 0, what); }
    match spawn(a, &s)? { Ok(p) => ok(a, cpu, p + 92, format!("{what} -> port {:#x}", p + 92)), Err(e) => fail(a, cpu, e, 0, what) }
}

/// Exit(d1 = returnCode): the process ends now with that return code (as if its entry had returned it); in the main
/// program it ends the run.
fn exit<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let rc = cpu.dreg(1);
    cpu.set_d0(rc);
    let line = crate::exec_task::task_end(a, cpu)?;
    Ok(format!("[dos] Exit({}) -> {line}", rc as i32))
}

/// The finalizer reached by a created process: run NP_ExitCode first (once). Ok(None) = not a created process, or
/// nothing more to do here.
pub fn process_end<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, rc: u32) -> Result<Option<String>, Refusal> {
    let cur = a.sched.cur;
    let Some(own) = a.sched.tasks.get_mut(&cur).and_then(|t| t.own.as_mut()) else { return Ok(None) };
    if own.exit_code == 0 { return Ok(None); }
    let (code, data) = (own.exit_code, own.exit_data);
    // When the exit code returns the process is over: it never goes back to the code that called Exit() or returned.
    let end = Box::new(move |a: &mut Amiga<U>, cpu: &mut Cpu, _| {
        if let Some(t) = a.sched.tasks.get_mut(&cur) { t.dead = true; }
        cpu.block();
        Ok(Next::Return(rc))
    });
    crate::guest_call::start(a, cpu, Next::Call { target: code, d0: rc, aregs: alloc::vec![(1, data)], then: end })?;
    cpu.set_dreg(1, data);
    Ok(Some(format!("[dos] process {cur:#x} returned {}: calling NP_ExitCode {code:#x}", rc as i32)))
}

/// Release what the process owned, per the tags (AROS DosEntry's epilogue, createnewproc.c): local variables, the
/// streams it was told to close, the seglist it was told to free, the current/home directory locks, then the blocks.
pub(crate) fn teardown<U: GuestUi>(a: &mut Amiga<U>, p: u32, own: ProcOwn, vars: crate::vars::Vars) -> Result<(), Refusal> {
    let rd = |a: &Amiga<U>, off| a.mem.read_u32(p + off).unwrap_or(0);
    crate::vars::free_locals(a, vars);
    for (flag, off, what) in [(own.close_in, PR_CIS, "input"), (own.close_out, PR_COS, "output"), (own.close_err, PR_CES, "error")] {
        let b = rd(a, off);
        if flag && b != 0 {
            if let Err(Refusal(e)) = crate::dos_io::close_fh(a, b) { a.log.push(format!("[dos] process {p:#x}: closing its {what} stream: {e}")); }
        }
    }
    if own.free_seglist != 0 { if let Err(Refusal(e)) = crate::loadseg::unload(a, own.free_seglist).and_then(|k| if k { Ok(()) } else { Err(Refusal("seglist already freed".into())) }) { a.log.push(format!("[dos] process {p:#x}: freeing its seglist: {e}")); } }
    let (cd, home) = (rd(a, PR_CURRENT_DIR), rd(a, PR_HOME_DIR));
    for (flag, b, what) in [(own.free_curdir, cd, "current directory"), (true, home, "home directory")] {
        if flag { if let Err(Refusal(e)) = crate::dos_files::free_lock(a, b, "process exit") { a.log.push(format!("[dos] process {p:#x}: unlocking its {what}: {e}")); } }
    }
    release_blocks(a, &own);
    Ok(())
}
