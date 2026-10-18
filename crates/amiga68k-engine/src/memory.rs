//! exec.library memory: AllocMem / FreeMem / AllocVec / FreeVec / AvailMem / CopyMem over a guest heap, plus SetSignal and StackSwap.
//!
//! (The heap itself is retro_guest_mem::heap, shared with the other engines.) Its bookkeeping lives HOST-side (the guest cannot corrupt it): a first-fit free list and the set of live
//! blocks. A FreeMem of an address this heap never returned, or with a size other than the one allocated, is a named
//! refusal, not a silent corruption. Forbid/Permit/Disable/Enable live in exec_task.rs.
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::format;
use crate::{machine::Amiga, Refusal};
pub use retro_guest_mem::heap::Heap;
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

pub const LVO_FORBID: i32 = -132;
pub const LVO_PERMIT: i32 = -138;
pub const LVO_ALLOC_MEM: i32 = -198;
pub const LVO_FREE_MEM: i32 = -210;
pub const LVO_AVAIL_MEM: i32 = -216;
pub const LVO_SET_SIGNAL: i32 = -306;
pub const LVO_COPY_MEM: i32 = -624;
pub const LVO_ALLOC_VEC: i32 = -684;
pub const LVO_FREE_VEC: i32 = -690;

/// exec/memory.h requirement and AvailMem flags.
const MEMF_CLEAR: u32 = 1 << 16;
const MEMF_LARGEST: u32 = 1 << 17;
/// AllocVec's size header: one heap grain (AmigaOS MEM_BLOCKSIZE = 8 bytes, retro_guest_mem::heap::GRAIN).
const GRAIN: u32 = retro_guest_mem::heap::GRAIN;
/// tc_SigRecvd in the program's Task (see process.rs).
const TC_SIG_RECVD: u32 = 26;

/// AllocMem(d0 = byteSize, d1 = requirements) -> d0 = block or NULL.
pub fn alloc_mem<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (size, req) = (cpu.dreg(0), cpu.dreg(1));
    let at = if size == 0 { None } else { a.heap.alloc(size) };
    if let (Some(p), true) = (at, req & MEMF_CLEAR != 0) { zero(a, p, size)?; }
    cpu.set_d0(at.unwrap_or(0));
    Ok(format!("[exec] AllocMem size={size} req={req:#x} -> {:#x}", at.unwrap_or(0)))
}

/// FreeMem(a1 = block, d0 = byteSize). FreeMem(NULL, n) is allowed (a no-op) by AmigaOS 2.0+.
pub fn free_mem<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (at, size) = (cpu.areg(1), cpu.dreg(0));
    if at != 0 { a.heap.free(at, size).map_err(|e| Refusal(format!("FreeMem: {e}")))?; }
    Ok(format!("[exec] FreeMem {at:#x} size={size}"))
}

/// AllocVec(d0 = byteSize, d1 = requirements): AllocMem with the size kept in the longword before the block.
pub fn alloc_vec<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (size, req) = (cpu.dreg(0), cpu.dreg(1));
    let total = size.checked_add(GRAIN).ok_or_else(|| Refusal("AllocVec: size overflows".into()))?;
    let at = if size == 0 { None } else { a.heap.alloc(total) };
    let user = match at {
        Some(p) => {
            a.mem.write_u32(p, total).map_err(Refusal::mem("AllocVec size word"))?;
            if req & MEMF_CLEAR != 0 { zero(a, p + GRAIN, size)?; }
            p + GRAIN
        }
        None => 0,
    };
    cpu.set_d0(user);
    Ok(format!("[exec] AllocVec size={size} req={req:#x} -> {user:#x}"))
}

/// FreeVec(a1 = block from AllocVec, or NULL).
pub fn free_vec<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let user = cpu.areg(1);
    if user == 0 { return Ok("[exec] FreeVec NULL".into()); }
    let at = user.wrapping_sub(GRAIN);
    let total = a.mem.read_u32(at).map_err(Refusal::mem("FreeVec size word"))?;
    a.heap.free(at, total).map_err(|e| Refusal(format!("FreeVec: {e}")))?;
    Ok(format!("[exec] FreeVec {user:#x}"))
}

/// AvailMem(d1 = requirements): free bytes, or the largest free block with MEMF_LARGEST.
pub fn avail_mem<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let n = a.heap.available(cpu.dreg(1) & MEMF_LARGEST != 0);
    cpu.set_d0(n);
    Ok(format!("[exec] AvailMem req={:#x} -> {n}", cpu.dreg(1)))
}

/// CopyMem(a0 = source, a1 = dest, d0 = size); overlapping copies are undefined on AmigaOS, copied forwards here.
pub fn copy_mem<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (src, dst, n) = (cpu.areg(0), cpu.areg(1), cpu.dreg(0));
    let mut buf = vec![0u8; n as usize];
    for (i, b) in buf.iter_mut().enumerate() { *b = a.mem.read_u8(src + i as u32).map_err(Refusal::mem("CopyMem source"))?; }
    for (i, b) in buf.iter().enumerate() { a.mem.write_u8(dst + i as u32, *b).map_err(Refusal::mem("CopyMem dest"))?; }
    Ok(format!("[exec] CopyMem {src:#x} -> {dst:#x} n={n}"))
}

/// SetSignal(d0 = newSignals, d1 = signalMask) -> d0 = the old signals: the running task's own tc_SigRecvd.
pub fn set_signal<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (new, mask) = (cpu.dreg(0), cpu.dreg(1));
    let at = a.sched.cur + TC_SIG_RECVD;
    let old = a.mem.read_u32(at).map_err(Refusal::mem("tc_SigRecvd"))?;
    a.mem.write_u32(at, (old & !mask) | (new & mask)).map_err(Refusal::mem("tc_SigRecvd"))?;
    cpu.set_d0(old);
    Ok(format!("[exec] SetSignal new={new:#x} mask={mask:#x} -> {old:#x}"))
}

/// A zeroed guest block of `size` bytes, or a named NOMEM refusal (`what` says for what).
pub fn alloc_zeroed<U: GuestUi>(a: &mut Amiga<U>, size: u32, what: &str) -> Result<u32, Refusal> {
    let at = a.heap.alloc(size).ok_or_else(|| Refusal(format!("{what}: no memory (NOMEM)")))?;
    a.mem.zero(at, size).map_err(Refusal::mem(what))?;
    Ok(at)
}

fn zero<U: GuestUi>(a: &mut Amiga<U>, at: u32, n: u32) -> Result<(), Refusal> {
    for i in 0..n { a.mem.write_u8(at + i, 0).map_err(Refusal::mem("MEMF_CLEAR"))?; }
    Ok(())
}

pub const LVO_STACK_SWAP: i32 = -732;
const TC_SP_LOWER: u32 = 58;
const TC_SP_UPPER: u32 = 62;

/// StackSwap(a0 = struct StackSwapStruct { stk_Lower, stk_Upper, stk_Pointer }): run on the caller's new stack from
/// the return onwards. The struct is swapped with the task's current bounds and stack pointer, so a second StackSwap
/// with the same struct swaps back (exec/tasks.h). The trap runs with A7 at the JSR's return address; it is moved to
/// the top of the new stack so the slot's RTS returns there.
pub fn stack_swap<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let m = Refusal::mem;
    let sss = cpu.areg(0);
    let (new_lower, new_upper, new_ptr) = (a.mem.read_u32(sss).map_err(m("stk_Lower"))?, a.mem.read_u32(sss + 4).map_err(m("stk_Upper"))?, a.mem.read_u32(sss + 8).map_err(m("stk_Pointer"))?);
    if !(new_lower < new_ptr && new_ptr <= new_upper) { return Err(Refusal(format!("StackSwap: stk_Pointer {new_ptr:#x} outside [{new_lower:#x}, {new_upper:#x}]"))); }
    let old_sp = cpu.areg(7);
    let ret = a.mem.read_u32(old_sp).map_err(m("return address"))?;
    let task = a.sched.cur;
    let (old_lower, old_upper) = (a.mem.read_u32(task + TC_SP_LOWER).map_err(m("tc_SPLower"))?, a.mem.read_u32(task + TC_SP_UPPER).map_err(m("tc_SPUpper"))?);
    a.mem.write_u32(new_ptr - 4, ret).map_err(m("new stack"))?;
    a.mem.write_u32(task + TC_SP_LOWER, new_lower).map_err(m("tc_SPLower"))?;
    a.mem.write_u32(task + TC_SP_UPPER, new_upper).map_err(m("tc_SPUpper"))?;
    a.mem.write_u32(sss, old_lower).map_err(m("stk_Lower"))?;
    a.mem.write_u32(sss + 4, old_upper).map_err(m("stk_Upper"))?;
    a.mem.write_u32(sss + 8, old_sp + 4).map_err(m("stk_Pointer"))?; // the caller's SP once the call has returned
    cpu.set_areg(7, new_ptr - 4);
    Ok(format!("[exec] StackSwap -> [{new_lower:#x}, {new_upper:#x}] sp={:#x}", new_ptr - 4))
}

pub const LVO_CREATE_POOL: i32 = -696;
pub const LVO_DELETE_POOL: i32 = -702;
pub const LVO_ALLOC_POOLED: i32 = -708;
pub const LVO_FREE_POOLED: i32 = -714;
/// A pool handle is a small guest block (programs only pass it back); its blocks are tracked host-side.
const POOL_HANDLE_SIZE: u32 = 16;

/// CreatePool(d0 = requirements, d1 = puddleSize, d2 = threshSize) -> a pool, or NULL. Puddles are not modelled:
/// every AllocPooled is its own heap block, which is what a pool's caller can observe (an address, freed by
/// FreePooled or all at once by DeletePool).
pub fn create_pool<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let req = cpu.dreg(0);
    let pool = a.heap.alloc(POOL_HANDLE_SIZE).unwrap_or(0);
    if pool != 0 { a.pools.insert(pool, (req, BTreeMap::new())); }
    cpu.set_d0(pool);
    Ok(format!("[exec] CreatePool req={req:#x} puddle={} thresh={} -> {pool:#x}", cpu.dreg(1), cpu.dreg(2)))
}

/// AllocPooled(a0 = pool, d0 = size) -> a block or NULL; MEMF_CLEAR from the pool's requirements.
pub fn alloc_pooled<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (pool, size) = (cpu.areg(0), cpu.dreg(0));
    let req = a.pools.get(&pool).map(|p| p.0).ok_or_else(|| Refusal(format!("AllocPooled: {pool:#x} is not a pool")))?;
    let at = if size == 0 { None } else { a.heap.alloc(size) };
    if let Some(p) = at {
        if req & MEMF_CLEAR != 0 { zero(a, p, size)?; }
        if let Some(pl) = a.pools.get_mut(&pool) { pl.1.insert(p, size); }
    }
    cpu.set_d0(at.unwrap_or(0));
    Ok(format!("[exec] AllocPooled {pool:#x} size={size} -> {:#x}", at.unwrap_or(0)))
}

/// FreePooled(a0 = pool, a1 = block, d0 = size). A block of another pool, or a wrong size, is refused by name.
pub fn free_pooled<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (pool, at, size) = (cpu.areg(0), cpu.areg(1), cpu.dreg(0));
    let pl = a.pools.get_mut(&pool).ok_or_else(|| Refusal(format!("FreePooled: {pool:#x} is not a pool")))?;
    match pl.1.get(&at) {
        Some(&s) if s == size => { pl.1.remove(&at); }
        Some(&s) => return Err(Refusal(format!("FreePooled: {at:#x} is a {s}-byte block, freed as {size}"))),
        None => return Err(Refusal(format!("FreePooled: {at:#x} is not a block of pool {pool:#x}"))),
    }
    a.heap.free(at, size).map_err(|e| Refusal(format!("FreePooled: {e}")))?;
    Ok(format!("[exec] FreePooled {pool:#x} {at:#x} size={size}"))
}

/// DeletePool(a0 = pool, or NULL): every block still in it goes back, then the pool.
pub fn delete_pool<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let pool = cpu.areg(0);
    if pool == 0 { return Ok("[exec] DeletePool NULL".into()); }
    let (_, blocks) = a.pools.remove(&pool).ok_or_else(|| Refusal(format!("DeletePool: {pool:#x} is not a pool")))?;
    let n = blocks.len();
    for (at, size) in blocks { a.heap.free(at, size).map_err(|e| Refusal(format!("DeletePool: {e}")))?; }
    a.heap.free(pool, POOL_HANDLE_SIZE).map_err(|e| Refusal(format!("DeletePool: {e}")))?;
    Ok(format!("[exec] DeletePool {pool:#x} ({n} blocks returned)"))
}
