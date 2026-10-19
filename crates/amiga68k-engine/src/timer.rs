//! exec.library device I/O (SendIO / DoIO / CheckIO / WaitIO / AbortIO) and the one device that does any yet:
//! timer.device, on the engine's VIRTUAL clock (microseconds since start, advanced by the host). struct timerequest
//! (devices/timer.h) = an IORequest (32 bytes) + struct timeval { tv_secs 32, tv_micro 36 }.
//!
//! TR_ADDREQUEST queues a request that completes when the clock passes its deadline; completing = io_Error 0 and the
//! request replied to its mn_ReplyPort (which signals the waiting program). DoIO / WaitIO on a still-pending timer
//! park the calling task until it completes when other tasks exist (the host then advances the clock only when all
//! are blocked); with ONE task nothing else could run in between, so the clock is fast-forwarded to the deadline.
use alloc::string::String;
use alloc::format;
use crate::{exec_wait, machine::Amiga, Refusal};
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

pub const LVO_DO_IO: i32 = -456;
pub const LVO_SEND_IO: i32 = -462;
pub const LVO_CHECK_IO: i32 = -468;
pub const LVO_WAIT_IO: i32 = -474;
pub const LVO_ABORT_IO: i32 = -480;
/// timer.device's own library vectors (devices/timer.h / the AROS timer.device fd): GetSysTime(a0 = timeval), GetUpTime(a0).
const LVO_GET_SYS_TIME: i32 = -66;
const LVO_GET_UP_TIME: i32 = -72;

const IO_DEVICE: u32 = 20;
const IO_COMMAND: u32 = 28;
const IO_FLAGS: u32 = 30;
const IO_ERROR: u32 = 31;
const MN_REPLYPORT: u32 = 14;
const LN_TYPE: u32 = 8;
const TV_SECS: u32 = 32;
const TV_MICRO: u32 = 36;
const NT_MESSAGE: u8 = 5;
const NT_REPLYMSG: u8 = 7;
const IOF_QUICK: u8 = 1;
const TR_ADDREQUEST: u16 = 9;
const TR_GETSYSTIME: u16 = 10;
const IOERR_NOCMD: i8 = -3;
const IOERR_ABORTED: i8 = -2;

fn m(what: &'static str) -> impl Fn(retro_guest_mem::MemError) -> Refusal { Refusal::mem(what) }

impl<U: GuestUi> Amiga<U> {
    /// The earliest pending timer deadline, if any (the host advances the clock to it when the guest is blocked).
    pub fn next_timer_deadline(&self) -> Option<u64> { self.timers.iter().map(|t| t.0).chain(crate::mui::handlers::next_deadline(self)).min() }

    /// Move the virtual clock to `t` (never backwards) and complete every timer request due by then.
    pub fn advance_clock_to(&mut self, t: u64) -> Result<(), Refusal> {
        self.clock_us = self.clock_us.max(t);
        let now = self.clock_us;
        let (due, keep): (alloc::vec::Vec<_>, alloc::vec::Vec<_>) = self.timers.drain(..).partition(|x| x.0 <= now);
        self.timers = keep;
        for (_, io) in due { if io != 0 { complete(self, io, 0)?; } } // io 0 = a Delay() deadline: the clock reaching it is the wake-up
        crate::mui::handlers::fire_due(self) // the timers of MUI input handlers (mui/handlers.rs)
    }
}

/// Finish `io` with `err` and reply it (exec semantics: a quick, synchronous request is not replied).
fn complete<U: GuestUi>(a: &mut Amiga<U>, io: u32, err: i8) -> Result<(), Refusal> {
    a.mem.write_u8(io + IO_ERROR, err as u8).map_err(m("io_Error"))?;
    let quick = a.mem.read_u8(io + IO_FLAGS).map_err(m("io_Flags"))? & IOF_QUICK != 0;
    if quick { return a.mem.write_u8(io + LN_TYPE, NT_REPLYMSG).map_err(m("ln_Type")); }
    let rp = a.mem.read_u32(io + MN_REPLYPORT).map_err(m("mn_ReplyPort"))?;
    if rp == 0 { return a.mem.write_u8(io + LN_TYPE, NT_REPLYMSG).map_err(m("ln_Type")); }
    exec_wait::put_msg(a, rp, io, NT_REPLYMSG)
}

/// Has `io` completed? Some(io_Error) if so; `remove` also takes the reply off its reply port (WaitIO).
pub(crate) fn io_done<U: GuestUi>(a: &mut Amiga<U>, io: u32, remove: bool) -> Result<Option<u32>, Refusal> {
    if a.mem.read_u8(io + LN_TYPE).map_err(m("ln_Type"))? != NT_REPLYMSG { return Ok(None); }
    if remove {
        let rp = a.mem.read_u32(io + MN_REPLYPORT).map_err(m("mn_ReplyPort"))?;
        if rp != 0 && a.mem.read_u32(io).map_err(m("ln_Succ"))? != 0 { exec_wait::remove(a, io)?; }
    }
    Ok(Some(a.mem.read_u8(io + IO_ERROR).map_err(m("io_Error"))? as i8 as i32 as u32))
}

/// Start `io` on its device. Ok(true) = done already (completed in `start`), Ok(false) = queued.
fn start<U: GuestUi>(a: &mut Amiga<U>, io: u32) -> Result<bool, Refusal> {
    let dev = a.mem.read_u32(io + IO_DEVICE).map_err(m("io_Device"))?;
    let name = a.libs.iter().find(|l| l.base == dev).map(|l| l.name).ok_or_else(|| Refusal(format!("I/O on {dev:#x}: not an open device")))?;
    if name != "timer.device" { return Err(Refusal(format!("I/O on {name}: no command of it is bridged"))); }
    let cmd = a.mem.read_u16(io + IO_COMMAND).map_err(m("io_Command"))?;
    a.mem.write_u8(io + LN_TYPE, NT_MESSAGE).map_err(m("ln_Type"))?;
    match cmd {
        TR_ADDREQUEST => {
            let (s, us) = (a.mem.read_u32(io + TV_SECS).map_err(m("tv_secs"))? as u64, a.mem.read_u32(io + TV_MICRO).map_err(m("tv_micro"))? as u64);
            a.timers.push((a.clock_us + s * 1_000_000 + us, io));
            Ok(false)
        }
        TR_GETSYSTIME => {
            a.mem.write_u32(io + TV_SECS, (a.clock_us / 1_000_000) as u32).map_err(m("tv_secs"))?;
            a.mem.write_u32(io + TV_MICRO, (a.clock_us % 1_000_000) as u32).map_err(m("tv_micro"))?;
            complete(a, io, 0)?;
            Ok(true)
        }
        _ => { complete(a, io, IOERR_NOCMD)?; Ok(true) }
    }
}

/// SendIO(a1 = io): start it, return at once.
pub fn send_io<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let io = cpu.areg(1);
    a.mem.write_u8(io + IO_FLAGS, a.mem.read_u8(io + IO_FLAGS).map_err(m("io_Flags"))? & !IOF_QUICK).map_err(m("io_Flags"))?;
    start(a, io)?;
    Ok(format!("[exec] SendIO {io:#x} (clock {} us, {} pending)", a.clock_us, a.timers.len()))
}

/// Finish a pending timer `io` now by fast-forwarding the clock to its deadline.
fn finish_pending<U: GuestUi>(a: &mut Amiga<U>, io: u32) -> Result<(), Refusal> {
    if let Some(&(t, _)) = a.timers.iter().find(|x| x.1 == io) { a.advance_clock_to(t)?; }
    Ok(())
}

/// DoIO(a1 = io) -> io_Error: synchronous (IOF_QUICK, not replied).
pub fn do_io<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let io = cpu.areg(1);
    a.mem.write_u8(io + IO_FLAGS, a.mem.read_u8(io + IO_FLAGS).map_err(m("io_Flags"))? | IOF_QUICK).map_err(m("io_Flags"))?;
    if !start(a, io)? {
        if a.sched.tasks.len() > 1 { return crate::exec_wait::block_on(a, cpu, crate::exec_wait::Waiting::Io { io, remove: false }, format!("DoIO {io:#x}")); }
        finish_pending(a, io)?;
    }
    let err = a.mem.read_u8(io + IO_ERROR).map_err(m("io_Error"))? as i8;
    cpu.set_d0(err as i32 as u32);
    Ok(format!("[exec] DoIO {io:#x} -> {err} (clock {} us)", a.clock_us))
}

/// CheckIO(a1 = io) -> io if it has completed, else NULL.
pub fn check_io<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let io = cpu.areg(1);
    let done = a.mem.read_u8(io + LN_TYPE).map_err(m("ln_Type"))? == NT_REPLYMSG;
    cpu.set_d0(if done { io } else { 0 });
    Ok(format!("[exec] CheckIO {io:#x} -> {}", if done { "done" } else { "pending" }))
}

/// WaitIO(a1 = io) -> io_Error, with the reply removed from the reply port.
pub fn wait_io<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let io = cpu.areg(1);
    if a.sched.tasks.len() > 1 { return crate::exec_wait::block_on(a, cpu, crate::exec_wait::Waiting::Io { io, remove: true }, format!("WaitIO {io:#x}")); }
    finish_pending(a, io)?;
    let rp = a.mem.read_u32(io + MN_REPLYPORT).map_err(m("mn_ReplyPort"))?;
    let queued = rp != 0 && a.mem.read_u8(io + LN_TYPE).map_err(m("ln_Type"))? == NT_REPLYMSG && a.mem.read_u32(io).map_err(m("ln_Succ"))? != 0;
    if queued { exec_wait::remove(a, io)?; }
    let err = a.mem.read_u8(io + IO_ERROR).map_err(m("io_Error"))? as i8;
    cpu.set_d0(err as i32 as u32);
    Ok(format!("[exec] WaitIO {io:#x} -> {err} (clock {} us)", a.clock_us))
}

/// AbortIO(a1 = io): a pending timer request ends with IOERR_ABORTED and is replied; a finished one is untouched.
pub fn abort_io<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let io = cpu.areg(1);
    let pending = a.timers.iter().position(|x| x.1 == io);
    if let Some(i) = pending { a.timers.remove(i); complete(a, io, IOERR_ABORTED)?; }
    cpu.set_d0(0);
    Ok(format!("[exec] AbortIO {io:#x} ({})", if pending.is_some() { "aborted" } else { "already done" }))
}

/// The vectors of the `timer.device` base OpenDevice hands out that are implemented; every other one stays a named refusal.
pub fn register<U: GuestUi>(b: crate::libtable::LibraryBuilder<U>) -> crate::libtable::LibraryBuilder<U> {
    b.func(LVO_GET_SYS_TIME, "GetSysTime", get_time::<U>).func(LVO_GET_UP_TIME, "GetUpTime", get_time::<U>)
}

/// GetSysTime(a0 = timeval) / GetUpTime(a0 = timeval): the virtual clock as seconds and microseconds, the same value
/// TR_GETSYSTIME reports (the engine's clock starts at 0 when the program starts, so system time and up time coincide).
fn get_time<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let tv = cpu.areg(0);
    let (s, us) = ((a.clock_us / 1_000_000) as u32, (a.clock_us % 1_000_000) as u32);
    a.mem.write_u32(tv, s).map_err(m("timeval.tv_secs"))?;
    a.mem.write_u32(tv + 4, us).map_err(m("timeval.tv_micro"))?;
    Ok(format!("[timer.device] GetSysTime -> {s}.{us:06}"))
}
