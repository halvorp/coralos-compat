//! `_OSDispatch` ($A88F): the Process Manager and Temporary Memory routines of System 7 (Inside Macintosh: Processes;
//! Memory). The selector is the word pushed last ([`crate::package`]).
//!
//! There is exactly ONE process, the application. Its serial number is `{0, 2}` (`kCurrentProcess`, which every routine accepts
//! for "me"); `GetNextProcess` from `kNoProcess` finds it and from it finds nothing (`procNotFound`, -600); `SetFrontProcess` and
//! `WakeUpProcess` on it succeed (it is already in front and awake). `GetProcessInformation` fills the `ProcessInfoRec` (60 bytes):
//! the name from `CurApName`, type 'APPL', the launch mode from the application's `SIZE` resource (ID 0, else -1) and the heap
//! sizes; the creator signature is '????' (the file's creator is not kept by the launcher) and the `FSSpec` (`processAppSpec`)
//! is not filled (named in the log). Temporary Memory is the application heap (there is no second pool): `TempNewHandle` is
//! `NewHandle`, `TempFreeMem`/`TempMaxMem` the free bytes, `TempTopMem` the top of RAM. High-level events never arrive:
//! `AcceptHighLevelEvent` answers `noOutstandingHLE` (-607). Any other selector halts the guest with a named refusal.
use crate::lowmem::{CUR_AP_NAME, MEM_TOP, TICKS};
use crate::machine::{Call, Done, Mac};
use crate::osconv::Regs;
use crate::package::{call, Routine};
use crate::pascal::Arg;
use crate::Refusal;
use alloc::format;
use alloc::string::String;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

pub const PROC_NOT_FOUND: i16 = -600;
pub const NO_OUTSTANDING_HLE: i16 = -607;
/// The application's serial number: high long, low long (`kCurrentProcess`).
pub const APP_PSN: (u32, u32) = (0, 2);
const NO_PROCESS: (u32, u32) = (0, 0);

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> { m.register_os(0xA88F, os_dispatch::<U>) }

fn os_dispatch<U: GuestUi>(m: &mut Mac<U>, c: &mut Cpu, r: &mut Regs) -> Result<String, Refusal> { call(m, c, r, "OSDispatch", lookup::<U>) }

const P: Arg = Arg::P;
const L: Arg = Arg::L;
const W: Option<Arg> = Some(Arg::W);

fn lookup<U: GuestUi>(sel: u16) -> Option<Routine<U>> {
    let r = |name, args, result, run| Some(Routine { name, args, result, run });
    match sel {
        0x15 => r("TempMaxMem", &[P], Some(L), temp_max_mem::<U>),
        0x16 => r("TempTopMem", &[], Some(L), temp_top_mem::<U>),
        0x18 => r("TempFreeMem", &[], Some(L), temp_free_mem::<U>),
        0x1D => r("TempNewHandle", &[L, P], Some(L), temp_new_handle::<U>),
        0x1E => r("TempHLock", &[P, P], None, temp_hlock::<U>),
        0x1F => r("TempHUnlock", &[P, P], None, temp_hunlock::<U>),
        0x20 => r("TempDisposeHandle", &[P, P], None, temp_dispose::<U>),
        0x33 => r("AcceptHighLevelEvent", &[P, P, P, P], W, accept_hle::<U>),
        0x37 => r("GetCurrentProcess", &[P], W, get_current::<U>),
        0x38 => r("GetNextProcess", &[P], W, get_next::<U>),
        0x39 => r("GetFrontProcess", &[P], W, get_current::<U>),
        0x3A => r("GetProcessInformation", &[P, P], W, get_info::<U>),
        0x3B | 0x3C => r(if sel == 0x3B { "SetFrontProcess" } else { "WakeUpProcess" }, &[P], W, ok_if_app::<U>),
        0x3D => r("SameProcess", &[P, P, P], W, same_process::<U>),
        _ => None,
    }
}

fn psn<U: GuestUi>(m: &Mac<U>, at: u32) -> Result<(u32, u32), Refusal> {
    Ok((m.mem.read_u32(at).map_err(Refusal::mem("ProcessSerialNumber"))?, m.mem.read_u32(at + 4).map_err(Refusal::mem("ProcessSerialNumber"))?))
}
fn put_psn<U: GuestUi>(m: &mut Mac<U>, at: u32, v: (u32, u32)) -> Result<(), Refusal> {
    m.mem.write_u32(at, v.0).and_then(|_| m.mem.write_u32(at + 4, v.1)).map_err(Refusal::mem("ProcessSerialNumber"))
}
fn is_app(p: (u32, u32)) -> bool { p == APP_PSN }
fn err(e: i16) -> u32 { e as u16 as u32 }

fn get_current<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    put_psn(m, c.args.get(0), APP_PSN)?;
    Ok(Done { result: 0, log: format!("[process] {} -> {{0,2}}", c.name) })
}

fn get_next<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let p = psn(m, c.args.get(0))?;
    let (found, e) = if p == NO_PROCESS { (APP_PSN, 0) } else { (NO_PROCESS, PROC_NOT_FOUND) };
    put_psn(m, c.args.get(0), found)?;
    Ok(Done { result: err(e), log: format!("[process] GetNextProcess {{{},{}}} -> {{{},{}}} err {e}", p.0, p.1, found.0, found.1) })
}

fn ok_if_app<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let p = psn(m, c.args.get(0))?;
    let e = if is_app(p) { 0 } else { PROC_NOT_FOUND };
    Ok(Done { result: err(e), log: format!("[process] {} {{{},{}}} -> {e}", c.name, p.0, p.1) })
}

fn same_process<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (a, b) = (psn(m, c.args.get(0))?, psn(m, c.args.get(1))?);
    let (e, same) = if is_app(a) && is_app(b) { (0, true) } else if (is_app(a) || a == b) && (is_app(b) || a == b) && a != NO_PROCESS { (0, a == b) } else { (PROC_NOT_FOUND, false) };
    m.mem.write_u8(c.args.get(2), same as u8).map_err(Refusal::mem("SameProcess result"))?;
    Ok(Done { result: err(e), log: format!("[process] SameProcess -> {same} err {e}") })
}

/// `ProcessInfoRec` offsets (Inside Macintosh: Processes): processInfoLength 0, processName 4, processNumber 8, processType 16,
/// processSignature 20, processMode 24, processLocation 28, processSize 32, processFreeMem 36, processLauncher 40,
/// processLaunchDate 48, processActiveTime 52, processAppSpec 56; 60 bytes.
fn get_info<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (p, info) = (psn(m, c.args.get(0))?, c.args.get(1));
    if !is_app(p) { return Ok(Done { result: err(PROC_NOT_FOUND), log: format!("[process] GetProcessInformation {{{},{}}}: no such process", p.0, p.1) }); }
    let e = Refusal::mem("ProcessInfoRec");
    let len = m.mem.read_u32(info).map_err(&e)?;
    if len < 60 { return Err(Refusal(format!("GetProcessInformation: processInfoLength is {len}, a ProcessInfoRec is 60 bytes (the caller must set it)"))); }
    let name_ptr = m.mem.read_u32(info + 4).map_err(&e)?;
    if name_ptr != 0 {
        let name = crate::pascal::read_pstring_bytes(&m.mem, CUR_AP_NAME)?;
        crate::pascal::write_pstring(&mut m.mem, name_ptr, &name)?;
    }
    let mode = m.size_resource_flags();
    let (size, free) = (crate::machine::HEAP_END - crate::machine::HEAP_START, m.heap.available(false));
    let ticks = m.mem.read_u32(TICKS).map_err(&e)?;
    put_psn(m, info + 8, APP_PSN)?;
    for (off, v) in [(16, u32::from_be_bytes(*b"APPL")), (20, u32::from_be_bytes(*b"????")), (24, mode), (28, m.seg.a5), (32, size), (36, free), (40, 0), (44, 1), (48, ticks), (52, ticks)] {
        m.mem.write_u32(info + off, v).map_err(&e)?;
    }
    let spec = m.mem.read_u32(info + 56).map_err(&e)?;
    let note = if spec != 0 { "; processAppSpec is not filled (no FSSpec of the launched file is kept)" } else { "" };
    Ok(Done { result: 0, log: format!("[process] GetProcessInformation -> mode {mode:#x}, size {size}, free {free}{note}") })
}

fn accept_hle<U: GuestUi>(_: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    Ok(Done { result: err(NO_OUTSTANDING_HLE), log: "[process] AcceptHighLevelEvent -> noOutstandingHLE (no program sends high-level events here)".into() })
}

fn temp_max_mem<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let free = m.heap.available(true);
    m.mem.write_u32(c.args.get(0), 0).map_err(Refusal::mem("TempMaxMem grow"))?; // the heap never grows into more room
    Ok(Done { result: free, log: format!("[mem] TempMaxMem -> {free} (the application heap; grow = 0)") })
}
fn temp_top_mem<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    let top = m.mem.read_u32(MEM_TOP).map_err(Refusal::mem("MemTop"))?;
    Ok(Done { result: top, log: format!("[mem] TempTopMem -> {top:#x}") })
}
fn temp_free_mem<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    let free = m.heap.available(false);
    Ok(Done { result: free, log: format!("[mem] TempFreeMem -> {free}") })
}
fn temp_new_handle<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (size, out) = (c.args.get(0), c.args.get(1));
    let (h, e) = match m.mm.new_handle(&mut m.mem, &mut m.heap, size, false) { Ok(h) => (h, 0), Err(e) => (0, e) };
    m.mem.write_u16(out, e as u16).map_err(Refusal::mem("TempNewHandle resultCode"))?;
    Ok(Done { result: h, log: format!("[mem] TempNewHandle {size} -> {h:#x} err {e}") })
}
fn temp_op<U: GuestUi>(m: &mut Mac<U>, c: &Call, f: fn(&mut Mac<U>, u32) -> Result<(), i16>) -> Result<Done, Refusal> {
    let (h, out) = (c.args.get(0), c.args.get(1));
    let e = f(m, h).err().unwrap_or(0);
    m.mem.write_u16(out, e as u16).map_err(Refusal::mem("Temp resultCode"))?;
    Ok(Done { result: 0, log: format!("[mem] {} {h:#x} err {e}", c.name) })
}
fn temp_hlock<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    temp_op(m, c, |m, h| { m.mm.block_mut(h).ok_or(crate::memmgr::MEM_WZ_ERR)?.locked = true; m.mm.sync_master(&mut m.mem, h) })
}
fn temp_hunlock<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    temp_op(m, c, |m, h| { m.mm.block_mut(h).ok_or(crate::memmgr::MEM_WZ_ERR)?.locked = false; m.mm.sync_master(&mut m.mem, h) })
}
fn temp_dispose<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { temp_op(m, c, |m, h| m.dispose_handle(h)) }

impl<U: GuestUi> Mac<U> {
    /// The application's `SIZE` resource flags (ID 0, else -1): the first long's high word; 0 without one.
    pub(crate) fn size_resource_flags(&mut self) -> u32 {
        let h = self.get_resource(*b"SIZE", 0, true).or_else(|_| self.get_resource(*b"SIZE", -1, true));
        h.ok().and_then(|h| self.handle_bytes(h)).filter(|b| b.len() >= 2).map_or(0, |b| u32::from(u16::from_be_bytes([b[0], b[1]])))
    }
}

/// The selector of the Process Manager / Temporary Memory routine named `name` (`"GetCurrentProcess"` -> 0x37), if this engine has it.
pub fn selector_of<U: GuestUi>(name: &str) -> Option<u16> { (0u16..0x40).find(|&s| lookup::<U>(s).is_some_and(|r| r.name == name)) }

/// Run routine `selector` with its Pascal arguments as VALUES (declaration order), as the PowerPC adapter does: the same body `_OSDispatch` runs for a 68k
/// caller. Returns the result and its width (`None` for a procedure), or the handler's refusal.
pub fn call_selector<U: GuestUi>(m: &mut Mac<U>, selector: u16, values: &[u32]) -> Result<(u32, Option<Arg>), Refusal> {
    let rt = lookup::<U>(selector).ok_or_else(|| Refusal(format!("OSDispatch selector {selector:#x} is not a routine this engine implements")))?;
    let args = crate::pascal::Args::from_values(rt.args, values)?;
    let mut cpu = Cpu::detached();
    let Done { result, .. } = (rt.run)(m, &mut cpu, &Call { opcode: 0xA88F, pc: 0, name: rt.name, args })?;
    Ok((result, rt.result))
}

/// How many arguments routine `selector` takes.
pub fn arg_count<U: GuestUi>(selector: u16) -> Option<usize> { lookup::<U>(selector).map(|r| r.args.len()) }
