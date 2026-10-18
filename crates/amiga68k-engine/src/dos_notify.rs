//! dos.library StartNotify / EndNotify: the registration half of file-change notification.
//!
//! StartNotify(d1 = struct NotifyRequest) resolves nr_Name to a full path (a name without a volume is taken relative to the
//! current directory), stores it in nr_FullName (a block the request owns until EndNotify) and records the request;
//! EndNotify(d1) releases it. A request that was not started, or is started twice, is a named refusal.
//! NOT modelled: the notifications themselves. The engine's volume does not tell the notify table when a file changes, so no
//! NotifyMessage is sent and no signal raised, not even for NRF_NOTIFY_INITIAL; the log line says so, for a program that waits.
//! Offsets (NDK dos/notify.h): nr_Name 0, nr_FullName 4, nr_UserData 8, nr_Flags 12.
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use crate::libtable::LibraryBuilder;
use crate::machine::Amiga;
use crate::{fs, Refusal};
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

const NR_NAME: u32 = 0;
const NR_FULL_NAME: u32 = 4;
const NR_FLAGS: u32 = 12;
const DOSTRUE: u32 = 0xFFFF_FFFF;

/// request address -> (the nr_FullName block, its size)
#[derive(Default)]
pub struct State { started: BTreeMap<u32, (u32, u32)> }

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.named("StartNotify", start_notify::<U>).named("EndNotify", end_notify::<U>)
}

fn start_notify<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let nr = cpu.dreg(1);
    let m = Refusal::mem("NotifyRequest");
    if a.notify.started.contains_key(&nr) { return Err(Refusal(format!("StartNotify: {nr:#x} is already started"))); }
    let np = a.mem.read_u32(nr + NR_NAME).map_err(&m)?;
    let name = a.mem.read_cstr(np).map_err(Refusal::mem("nr_Name"))?;
    let full = if name.contains(':') { name.clone() } else {
        let cwd = a.fs.vol.full_path(fs::cur_dir(a));
        if cwd.ends_with(':') || cwd.ends_with('/') { format!("{cwd}{name}") } else { format!("{cwd}/{name}") }
    };
    let size = full.len() as u32 + 1;
    let Some(blk) = a.heap.alloc(size) else { return fs::fail(a, cpu, crate::dos_err::ERROR_NO_FREE_STORE, 0, "[dos] StartNotify".into()) };
    a.mem.write_bytes(blk, full.as_bytes()).map_err(&m)?;
    a.mem.write_u8(blk + full.len() as u32, 0).map_err(&m)?;
    a.mem.write_u32(nr + NR_FULL_NAME, blk).map_err(&m)?;
    let flags = a.mem.read_u32(nr + NR_FLAGS).map_err(&m)?;
    a.notify.started.insert(nr, (blk, size));
    cpu.set_d0(DOSTRUE);
    Ok(format!("[dos] StartNotify \"{full}\" flags {flags:#x} (registered; the engine sends no change notifications)"))
}

fn end_notify<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let nr = cpu.dreg(1);
    let Some((blk, size)) = a.notify.started.remove(&nr) else { return Err(Refusal(format!("EndNotify: {nr:#x} was not started with StartNotify"))) };
    a.heap.free(blk, size).map_err(|e| Refusal(format!("EndNotify: {e}")))?;
    a.mem.write_u32(nr + NR_FULL_NAME, 0).map_err(Refusal::mem("nr_FullName"))?;
    Ok(format!("[dos] EndNotify {nr:#x}"))
}
