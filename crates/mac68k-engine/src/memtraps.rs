//! The Memory Manager traps: OS-trap register convention over [`crate::memmgr`].
//!
//! Register contracts (Inside Macintosh II/V, "Memory Manager"): `NewHandle`/`NewPtr` take the size in D0 and return
//! the Handle/Ptr in A0; `DisposeHandle`/`DisposePtr`/`HLock`/`HUnlock`/`HGetState` take the Handle in A0;
//! `SetHandleSize` takes A0 = handle and D0 = size; `GetHandleSize`/`GetPtrSize` return the size in D0 (a negative
//! value is an error); `HSetState` takes A0 = handle and D0 = state; `BlockMove` takes A0 = source, A1 = destination,
//! D0 = count; `FreeMem` returns the free bytes in D0. Every call also leaves its result in the MemErr global
//! (`MemError` is a read of that word, not a trap) and in D0.
use crate::lowmem::MEM_ERR;
use crate::machine::Mac;
use crate::memmgr::{block_move, MEM_WZ_ERR, NIL_HANDLE_ERR};
use crate::osconv::Regs;
use crate::Refusal;
use alloc::format;
use alloc::string::String;
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

type R = Result<String, Refusal>;

impl<U: GuestUi> Mac<U> {
    /// Record a Memory Manager result: MemErr, D0, and (for a pointer/handle the manager does not own) a named
    /// refusal entry as well, because that is a guest bug, not an ordinary condition like running out of memory.
    pub(crate) fn mem_result(&mut self, regs: &mut Regs, name: &str, what: &str, r: Result<(), i16>) -> i16 {
        let e = r.err().unwrap_or(0);
        if e == MEM_WZ_ERR { self.note_refusal(name, format!("{what} is not a block the Memory Manager owns (memWZErr)")); }
        self.set_mem_err(e);
        regs.err(e);
        e
    }
    pub(crate) fn set_mem_err(&mut self, e: i16) { let _ = self.mem.write_u16(MEM_ERR, e as u16); }

    /// DisposeHandle for the engine itself (ReleaseResource, DisposeMenu ...): the handle's other bookkeeping goes too.
    pub(crate) fn dispose_handle(&mut self, h: u32) -> Result<(), i16> {
        self.rm.forget(h);
        self.menus.forget(h);
        self.mm.dispose_handle(&mut self.mem, &mut self.heap, h)
    }
}

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    m.register_os(0xA122, new_handle::<U>)?; // also _NewHandleClear ($A322): bit 9 is read from the opcode
    m.register_os(0xA11E, new_ptr::<U>)?; // also _NewPtrClear ($A31E)
    m.register_os(0xA023, dispose_handle::<U>)?;
    m.register_os(0xA01F, dispose_ptr::<U>)?;
    m.register_os(0xA025, get_handle_size::<U>)?;
    m.register_os(0xA024, set_handle_size::<U>)?;
    m.register_os(0xA021, get_ptr_size::<U>)?;
    m.register_os(0xA029, hlock::<U>)?;
    m.register_os(0xA02A, hunlock::<U>)?;
    m.register_os(0xA069, hget_state::<U>)?;
    m.register_os(0xA06A, hset_state::<U>)?;
    m.register_os(0xA049, hpurge::<U>)?;
    m.register_os(0xA04A, hnopurge::<U>)?;
    m.register_os(0xA02E, block_move_trap::<U>)?;
    m.register_os(0xA036, more_masters::<U>)?;
    m.register_os(0xA01C, free_mem::<U>)?;
    m.register_os(0xA055, strip_address::<U>)
}

fn new_handle<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> R {
    let (size, clear) = (r.d0, r.clear_flag());
    match m.mm.new_handle(&mut m.mem, &mut m.heap, size, clear) {
        Ok(h) => { r.a0 = h; m.set_mem_err(0); r.err(0); Ok(format!("[mem] NewHandle{} {size} -> {h:#x}", if clear { "Clear" } else { "" })) }
        Err(e) => { r.a0 = 0; m.set_mem_err(e); r.err(e); Ok(format!("[mem] NewHandle {size} failed ({e})")) }
    }
}

fn new_ptr<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> R {
    let (size, clear) = (r.d0, r.clear_flag());
    match m.mm.new_ptr(&mut m.mem, &mut m.heap, size, clear) {
        Ok(p) => { r.a0 = p; m.set_mem_err(0); r.err(0); Ok(format!("[mem] NewPtr{} {size} -> {p:#x}", if clear { "Clear" } else { "" })) }
        Err(e) => { r.a0 = 0; m.set_mem_err(e); r.err(e); Ok(format!("[mem] NewPtr {size} failed ({e})")) }
    }
}

fn dispose_handle<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> R {
    let h = r.a0;
    let res = m.dispose_handle(h);
    m.mem_result(r, "DisposeHandle", &format!("{h:#x}"), res);
    Ok(format!("[mem] DisposeHandle {h:#x}"))
}

fn dispose_ptr<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> R {
    let p = r.a0;
    let res = m.mm.dispose_ptr(&mut m.heap, p);
    m.mem_result(r, "DisposePtr", &format!("{p:#x}"), res);
    Ok(format!("[mem] DisposePtr {p:#x}"))
}

fn get_handle_size<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> R {
    let h = r.a0;
    let res = m.mm.handle_size(h);
    m.mem_result(r, "GetHandleSize", &format!("{h:#x}"), res.map(|_| ()));
    if let Ok(s) = res { r.d0 = s; }
    Ok(format!("[mem] GetHandleSize {h:#x} -> {:?}", res))
}

fn set_handle_size<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> R {
    let (h, size) = (r.a0, r.d0);
    let res = m.mm.set_handle_size(&mut m.mem, &mut m.heap, h, size);
    m.mem_result(r, "SetHandleSize", &format!("{h:#x}"), res);
    Ok(format!("[mem] SetHandleSize {h:#x} {size} -> {}", r.d0 as i32))
}

fn get_ptr_size<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> R {
    let p = r.a0;
    let res = m.mm.ptr_size(p);
    m.mem_result(r, "GetPtrSize", &format!("{p:#x}"), res.map(|_| ()));
    if let Ok(s) = res { r.d0 = s; }
    Ok(format!("[mem] GetPtrSize {p:#x} -> {res:?}"))
}

fn lock_common<U: GuestUi>(m: &mut Mac<U>, r: &mut Regs, name: &str, locked: bool) -> R {
    let h = r.a0;
    let res = m.mm.set_lock(h, locked);
    m.mem_result(r, name, &format!("{h:#x}"), res);
    Ok(format!("[mem] {name} {h:#x}"))
}
fn hlock<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> R { lock_common(m, r, "HLock", true) }
fn hunlock<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> R { lock_common(m, r, "HUnlock", false) }

fn purge_common<U: GuestUi>(m: &mut Mac<U>, r: &mut Regs, name: &str, purgeable: bool) -> R {
    let h = r.a0;
    let res = match m.mm.block_mut(h) {
        Some(b) => { b.purgeable = purgeable; Ok(()) }
        None => Err(if h == 0 { NIL_HANDLE_ERR } else { MEM_WZ_ERR }),
    };
    m.mem_result(r, name, &format!("{h:#x}"), res);
    Ok(format!("[mem] {name} {h:#x} (flag only: nothing is ever purged)"))
}
fn hpurge<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> R { purge_common(m, r, "HPurge", true) }
fn hnopurge<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> R { purge_common(m, r, "HNoPurge", false) }

fn hget_state<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> R {
    let h = r.a0;
    let res = m.mm.state(h);
    m.mem_result(r, "HGetState", &format!("{h:#x}"), res.map(|_| ()));
    if let Ok(s) = res { r.d0 = u32::from(s); }
    Ok(format!("[mem] HGetState {h:#x} -> {res:?}"))
}

fn hset_state<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> R {
    let (h, s) = (r.a0, r.d0 as u8);
    let res = m.mm.set_state(h, s);
    m.mem_result(r, "HSetState", &format!("{h:#x}"), res);
    Ok(format!("[mem] HSetState {h:#x} {s:#04x}"))
}

fn block_move_trap<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> R {
    let (src, dst, n) = (r.a0, r.a1, r.d0);
    let res = block_move(&mut m.mem, src, dst, n);
    if res.is_err() { m.note_refusal("BlockMove", format!("{n} bytes {src:#x} -> {dst:#x} leave guest RAM")); }
    m.set_mem_err(res.err().unwrap_or(0));
    r.err(res.err().unwrap_or(0));
    Ok(format!("[mem] BlockMove {src:#x} -> {dst:#x} x{n}"))
}

fn more_masters<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> R {
    let res = m.mm.more_masters(&mut m.mem, &mut m.heap);
    m.set_mem_err(res.err().unwrap_or(0));
    r.err(res.err().unwrap_or(0));
    Ok("[mem] MoreMasters".into())
}

fn free_mem<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> R {
    r.d0 = m.heap.available(false);
    Ok(format!("[mem] FreeMem -> {}", r.d0))
}

/// The 24-bit-era address strip: this engine's addresses are 32-bit clean and below 16 MiB, so it returns D0 unchanged.
fn strip_address<U: GuestUi>(_: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> R { Ok(format!("[mem] StripAddress {:#x}", r.d0)) }
