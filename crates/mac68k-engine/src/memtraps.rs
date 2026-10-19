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
    pub fn dispose_handle(&mut self, h: u32) -> Result<(), i16> {
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
    m.register_os(0xA055, strip_address::<U>)?;
    m.register_os(0xA063, max_appl_zone::<U>)?;
    m.register_os(0xA040, resrv_mem::<U>)?;
    m.register_os(0xA02D, set_appl_limit::<U>)?;
    m.register_os(0xA04C, compact_mem::<U>)?;
    m.register_os(0xA04D, purge_mem::<U>)?;
    m.register_os(0xA11D, max_mem::<U>)?;
    m.register_os(0xA162, purge_space::<U>)?;
    m.register_os(0xA064, move_hhi::<U>)?;
    m.register_os(0xA065, stack_space::<U>)?;
    m.register_os(0xA128, recover_handle::<U>)?;
    m.register_os(0xA11A, get_zone::<U>)?;
    m.register_os(0xA01B, set_zone::<U>)?;
    m.register_os(0xA04B, set_grow_zone::<U>)?;
    m.register_os(0xA9E1, hand_to_hand::<U>)?;
    m.register_os(0xA9E3, ptr_to_hand::<U>)?;
    m.register_os(0xA9EF, ptr_and_hand::<U>)?;
    m.register_os(0xA9E4, hand_and_hand::<U>)
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
    let res = m.mm.set_lock(&mut m.mem, h, locked);
    m.mem_result(r, name, &format!("{h:#x}"), res);
    Ok(format!("[mem] {name} {h:#x}"))
}
fn hlock<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> R { lock_common(m, r, "HLock", true) }
fn hunlock<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> R { lock_common(m, r, "HUnlock", false) }

fn purge_common<U: GuestUi>(m: &mut Mac<U>, r: &mut Regs, name: &str, purgeable: bool) -> R {
    let h = r.a0;
    let res = match m.mm.block_mut(h) {
        Some(b) => { b.purgeable = purgeable; m.mm.sync_master(&mut m.mem, h) }
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
    let res = m.mm.set_state(&mut m.mem, h, s);
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

/// `StripAddress(D0)`: the address as the bus sees it. On a 24-bit machine the top byte (the application's flag bits)
/// goes; on a 32-bit one the address is returned unchanged (Inside Macintosh: Memory).
fn strip_address<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> R {
    let was = r.d0;
    r.d0 = m.mem.strip(was);
    Ok(format!("[mem] StripAddress {was:#x} -> {:#x}", r.d0))
}

// ---- zone-level calls: the heap is one fixed zone that never compacts or purges ------------------------------------

fn max_appl_zone<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> R {
    m.set_mem_err(0);
    r.err(0);
    Ok("[mem] MaxApplZone (accepted: the heap is already one fixed zone)".into())
}

/// `ResrvMem(D0 = bytes)`: succeeds when one free block can hold that many bytes (nothing needs moving here).
fn resrv_mem<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> R {
    let want = r.d0;
    let e = if m.heap.available(true) >= want { 0 } else { crate::memmgr::MEM_FULL_ERR };
    m.set_mem_err(e);
    r.err(e);
    Ok(format!("[mem] ResrvMem {want} -> {e}"))
}

fn set_appl_limit<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> R {
    m.set_mem_err(0);
    r.err(0);
    Ok(format!("[mem] SetApplLimit {:#x} (accepted, ApplLimit unchanged: the heap does not grow into the stack here)", r.a0))
}

fn compact_mem<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> R {
    r.d0 = m.heap.available(true);
    Ok(format!("[mem] CompactMem -> largest free block {}", r.d0))
}

fn purge_mem<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> R {
    let want = r.d0;
    let e = if m.heap.available(true) >= want { 0 } else { crate::memmgr::MEM_FULL_ERR };
    m.set_mem_err(e);
    r.err(e);
    Ok(format!("[mem] PurgeMem {want} -> {e} (nothing is purged)"))
}

fn max_mem<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> R {
    r.d0 = m.heap.available(true);
    r.a0 = 0; // the heap cannot grow
    Ok(format!("[mem] MaxMem -> {}", r.d0))
}

/// `PurgeSpace`: D0 = the total free bytes if every purgeable block were purged, A0 = the largest contiguous block that would give.
/// Nothing is ever purged here, so these are the heap's free bytes now (purgeable blocks are not counted as space).
fn purge_space<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> R {
    r.d0 = m.heap.available(false);
    r.a0 = m.heap.available(true);
    Ok(format!("[mem] PurgeSpace -> total {}, contiguous {}", r.d0, r.a0))
}

fn move_hhi<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> R {
    let h = r.a0;
    let res = m.mm.state(h).map(|_| ());
    m.mem_result(r, "MoveHHi", &format!("{h:#x}"), res);
    Ok(format!("[mem] MoveHHi {h:#x} (blocks never move)"))
}

/// `StackSpace`: bytes between SP and ApplLimit.
fn stack_space<U: GuestUi>(m: &mut Mac<U>, cpu: &mut Cpu, r: &mut Regs) -> R {
    let limit = m.mem.read_u32(crate::lowmem::APPL_LIMIT).map_err(Refusal::mem("ApplLimit"))?;
    r.d0 = cpu.areg(7).saturating_sub(limit);
    Ok(format!("[mem] StackSpace -> {}", r.d0))
}

fn recover_handle<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> R {
    let p = r.a0;
    match m.mm.recover_handle(p) {
        Some(h) => { r.a0 = h; m.set_mem_err(0); r.err(0); }
        None => { r.a0 = 0; m.note_refusal("RecoverHandle", format!("{p:#x} is not the start of a relocatable block (memBCErr)")); m.set_mem_err(-115); r.err(-115); }
    }
    Ok(format!("[mem] RecoverHandle {p:#x} -> {:#x}", r.a0))
}

fn get_zone<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> R {
    r.a0 = m.mem.read_u32(crate::lowmem::THE_ZONE).map_err(Refusal::mem("TheZone"))?;
    r.err(0);
    Ok(format!("[mem] GetZone -> {:#x}", r.a0))
}

fn set_zone<U: GuestUi>(_: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> R {
    // there is exactly one zone: SetZone to anything else is a bug in the program
    if r.a0 != crate::lowmem::ZONE_STUB { return Err(Refusal(format!("SetZone {:#x}: the only zone is {:#x}", r.a0, crate::lowmem::ZONE_STUB))); }
    r.err(0);
    Ok("[mem] SetZone (the only zone)".into())
}

fn set_grow_zone<U: GuestUi>(_: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> R {
    r.err(0);
    Ok(format!("[mem] SetGrowZone {:#x} (accepted; never called: the heap does not compact)", r.a0))
}

// ---- Toolbox utilities with register conventions -------------------------------------------------------------------

fn read_block<U: GuestUi>(m: &Mac<U>, p: u32, n: u32) -> Option<alloc::vec::Vec<u8>> { m.mem.bytes(m.mem.span(p, n).ok()?).ok().map(<[u8]>::to_vec) }

fn new_handle_with<U: GuestUi>(m: &mut Mac<U>, data: &[u8]) -> Result<u32, i16> {
    let h = m.mm.new_handle(&mut m.mem, &mut m.heap, data.len() as u32, false)?;
    let p = m.mm.block(h).map(|b| b.ptr).ok_or(MEM_WZ_ERR)?;
    m.mem.write_bytes(p, data).map_err(|_| MEM_WZ_ERR)?;
    Ok(h)
}

fn finish_h<U: GuestUi>(m: &mut Mac<U>, r: &mut Regs, name: &str, res: Result<u32, i16>) -> R {
    match res {
        Ok(h) => { r.a0 = h; m.set_mem_err(0); r.err(0); Ok(format!("[mem] {name} -> {h:#x}")) }
        Err(e) => {
            r.a0 = 0;
            if e == MEM_WZ_ERR { m.note_refusal(name, "argument is not a block the Memory Manager owns (memWZErr)".into()); }
            m.set_mem_err(e);
            r.err(e);
            Ok(format!("[mem] {name} failed ({e})"))
        }
    }
}

/// `HandToHand(A0 = handle)`: a new handle with a copy of the block, in A0.
fn hand_to_hand<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> R {
    let src = r.a0;
    let res = m.handle_bytes(src).ok_or(MEM_WZ_ERR).and_then(|d| new_handle_with(m, &d));
    finish_h(m, r, "HandToHand", res)
}

/// `PtrToHand(A0 = ptr, D0 = size)`: a new handle holding a copy of `size` bytes at `ptr`.
fn ptr_to_hand<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> R {
    let (p, n) = (r.a0, r.d0);
    let res = read_block(m, p, n).ok_or(MEM_WZ_ERR).and_then(|d| new_handle_with(m, &d));
    finish_h(m, r, "PtrToHand", res)
}

fn append_to<U: GuestUi>(m: &mut Mac<U>, h: u32, data: &[u8]) -> Result<u32, i16> {
    let old = m.mm.handle_size(h)?;
    m.mm.set_handle_size(&mut m.mem, &mut m.heap, h, old + data.len() as u32)?;
    let p = m.mm.block(h).map(|b| b.ptr).ok_or(MEM_WZ_ERR)?;
    m.mem.write_bytes(p + old, data).map_err(|_| MEM_WZ_ERR)?;
    Ok(h)
}

/// `PtrAndHand(A0 = ptr, A1 = handle, D0 = size)`: append `size` bytes at `ptr` to the handle's block.
fn ptr_and_hand<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> R {
    let (p, h, n) = (r.a0, r.a1, r.d0);
    let res = read_block(m, p, n).ok_or(MEM_WZ_ERR).and_then(|d| append_to(m, h, &d));
    let out = finish_h(m, r, "PtrAndHand", res);
    r.a0 = if r.d0 == 0 { h } else { 0 }; // the manual leaves A0 undefined; keep it the handle on success
    out
}

/// `HandAndHand(A0 = source handle, A1 = destination handle)`: append the source block to the destination.
fn hand_and_hand<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> R {
    let (src, dst) = (r.a0, r.a1);
    let res = m.handle_bytes(src).ok_or(MEM_WZ_ERR).and_then(|d| append_to(m, dst, &d));
    let out = finish_h(m, r, "HandAndHand", res);
    r.a0 = if r.d0 == 0 { dst } else { 0 };
    out
}
