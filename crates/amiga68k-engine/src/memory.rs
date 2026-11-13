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

/// A zeroed-or-not AllocVec block (size word in front) for host code that makes what the guest later FreeVec()s; 0 when out of memory.
pub fn alloc_vec_block<U: GuestUi>(a: &mut Amiga<U>, size: u32, clear: bool) -> Result<u32, Refusal> {
    let total = size.checked_add(GRAIN).ok_or_else(|| Refusal("AllocVec: size overflows".into()))?;
    let at = if size == 0 { None } else { a.heap.alloc(total) };
    Ok(match at {
        Some(p) => {
            a.mem.write_u32(p, total).map_err(Refusal::mem("AllocVec size word"))?;
            if clear { zero(a, p + GRAIN, size)?; }
            p + GRAIN
        }
        None => 0,
    })
}

/// FreeVec of a block from [`alloc_vec_block`] / AllocVec (NULL is a no-op).
pub fn free_vec_block<U: GuestUi>(a: &mut Amiga<U>, user: u32) -> Result<(), Refusal> {
    if user == 0 { return Ok(()); }
    let at = user.wrapping_sub(GRAIN);
    let total = a.mem.read_u32(at).map_err(Refusal::mem("FreeVec size word"))?;
    a.heap.free(at, total).map_err(|e| Refusal(format!("FreeVec: {e}")))
}

/// AllocVec(d0 = byteSize, d1 = requirements): AllocMem with the size kept in the longword before the block.
pub fn alloc_vec<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (size, req) = (cpu.dreg(0), cpu.dreg(1));
    let user = alloc_vec_block(a, size, req & MEMF_CLEAR != 0)?;
    cpu.set_d0(user);
    Ok(format!("[exec] AllocVec size={size} req={req:#x} -> {user:#x}"))
}

/// FreeVec(a1 = block from AllocVec, or NULL).
pub fn free_vec<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let user = cpu.areg(1);
    if user == 0 { return Ok("[exec] FreeVec NULL".into()); }
    free_vec_block(a, user)?;
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
    copy_guest(a, src, dst, n)?;
    Ok(format!("[exec] CopyMem {src:#x} -> {dst:#x} n={n}"))
}

/// CopyMem's copy: both ranges are checked against guest RAM first, then moved in place (no host buffer of the guest's size).
fn copy_guest<U: GuestUi>(a: &mut Amiga<U>, src: u32, dst: u32, n: u32) -> Result<(), Refusal> {
    a.mem.check(src, n).map_err(Refusal::mem("CopyMem source"))?;
    a.mem.check(dst, n).map_err(Refusal::mem("CopyMem dest"))?;
    a.mem.copy_within(src, dst, n).map_err(Refusal::mem("CopyMem"))
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
    let new_sp = swap_stacks(a, cpu.areg(0), cpu.areg(7))?;
    cpu.set_areg(7, new_sp);
    Ok(format!("[exec] StackSwap -> sp={new_sp:#x}"))
}

/// StackSwap's work on guest memory: `sss` is the StackSwapStruct, `old_sp` the caller's A7 (at the JSR's return address).
/// Returns the new A7.
fn swap_stacks<U: GuestUi>(a: &mut Amiga<U>, sss: u32, old_sp: u32) -> Result<u32, Refusal> {
    let m = Refusal::mem;
    let field = |off: u32, what: &'static str| sss.checked_add(off).ok_or_else(|| Refusal(format!("StackSwap: the StackSwapStruct at {sss:#x} runs past the address space ({what})")));
    let (new_lower, new_upper, new_ptr) = (a.mem.read_u32(sss).map_err(m("stk_Lower"))?, a.mem.read_u32(field(4, "stk_Upper")?).map_err(m("stk_Upper"))?, a.mem.read_u32(field(8, "stk_Pointer")?).map_err(m("stk_Pointer"))?);
    // The return slot (new_ptr - 4) must itself lie inside the new stack, and the caller's SP once the call returns (old_sp + 4) must exist.
    if !(new_lower < new_ptr && new_ptr <= new_upper) || new_ptr - 4 < new_lower { return Err(Refusal(format!("StackSwap: stk_Pointer {new_ptr:#x} outside [{new_lower:#x}, {new_upper:#x}]"))); }
    let caller_sp = old_sp.checked_add(4).ok_or_else(|| Refusal(format!("StackSwap: the caller's stack pointer {old_sp:#x} leaves no room for its return address")))?;
    let ret = a.mem.read_u32(old_sp).map_err(m("return address"))?;
    let task = a.sched.cur;
    let (old_lower, old_upper) = (a.mem.read_u32(task + TC_SP_LOWER).map_err(m("tc_SPLower"))?, a.mem.read_u32(task + TC_SP_UPPER).map_err(m("tc_SPUpper"))?);
    a.mem.write_u32(new_ptr - 4, ret).map_err(m("new stack"))?;
    a.mem.write_u32(task + TC_SP_LOWER, new_lower).map_err(m("tc_SPLower"))?;
    a.mem.write_u32(task + TC_SP_UPPER, new_upper).map_err(m("tc_SPUpper"))?;
    a.mem.write_u32(sss, old_lower).map_err(m("stk_Lower"))?;
    a.mem.write_u32(field(4, "stk_Upper")?, old_upper).map_err(m("stk_Upper"))?;
    a.mem.write_u32(field(8, "stk_Pointer")?, caller_sp).map_err(m("stk_Pointer"))?; // the caller's SP once the call has returned
    Ok(new_ptr - 4)
}

pub const LVO_CREATE_POOL: i32 = -696;
pub const LVO_DELETE_POOL: i32 = -702;
pub const LVO_ALLOC_POOLED: i32 = -708;
pub const LVO_FREE_POOLED: i32 = -714;
/// A pool handle is a small guest block (programs only pass it back); its blocks are tracked host-side.
const POOL_HANDLE_SIZE: u32 = 16;

/// A pool for the engine's own use (a MUI List without `MUIA_List_Pool` makes one, as Zune's list.c does); 0 when out of memory.
pub fn new_pool<U: GuestUi>(a: &mut Amiga<U>, req: u32) -> u32 {
    let pool = a.heap.alloc(POOL_HANDLE_SIZE).unwrap_or(0);
    if pool != 0 { a.pools.insert(pool, (req, BTreeMap::new())); }
    pool
}

/// Give back a pool from [`new_pool`] with everything still in it.
pub fn drop_pool<U: GuestUi>(a: &mut Amiga<U>, pool: u32) {
    if let Some((_, blocks)) = a.pools.remove(&pool) {
        for (at, size) in blocks { let _ = a.heap.free(at, size); }
        let _ = a.heap.free(pool, POOL_HANDLE_SIZE);
    }
}

/// CreatePool(d0 = requirements, d1 = puddleSize, d2 = threshSize) -> a pool, or NULL. Puddles are not modelled:
/// every AllocPooled is its own heap block, which is what a pool's caller can observe (an address, freed by
/// FreePooled or all at once by DeletePool).
pub fn create_pool<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let req = cpu.dreg(0);
    let pool = new_pool(a, req);
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

/// AllocVecPooled(a0 = pool, d0 = size) -> a block or NULL: AllocPooled with the size kept in a longword in front, so FreeVecPooled needs
/// none (AROS rom/exec/allocvecpooled.c). The block the caller sees starts after that longword.
pub fn alloc_vec_pooled<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (pool, size) = (cpu.areg(0), cpu.dreg(0));
    if size == 0 || size > u32::MAX - 4 { cpu.set_d0(0); return Ok(format!("[exec] AllocVecPooled {pool:#x} size={size} -> NULL")); }
    cpu.set_dreg(0, size + 4);
    alloc_pooled(a, cpu)?;
    let at = cpu.dreg(0);
    if at != 0 {
        a.mem.write_u32(at, size + 4).map_err(Refusal::mem("AllocVecPooled size"))?;
        cpu.set_d0(at + 4);
    }
    Ok(format!("[exec] AllocVecPooled {pool:#x} size={size} -> {:#x}", cpu.dreg(0)))
}

/// FreeVecPooled(a0 = pool, a1 = block): NULL is ignored; otherwise FreePooled of the block and the longword before it.
pub fn free_vec_pooled<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (pool, at) = (cpu.areg(0), cpu.areg(1));
    if at == 0 { return Ok("[exec] FreeVecPooled NULL".into()); }
    free_vec_pooled_block(a, pool, at)
}

fn free_vec_pooled_block<U: GuestUi>(a: &mut Amiga<U>, pool: u32, at: u32) -> Result<String, Refusal> {
    let head = at.checked_sub(4).ok_or_else(|| Refusal(format!("FreeVecPooled: {at:#x} is too low to have a size longword before it")))?;
    let size = a.mem.read_u32(head).map_err(Refusal::mem("FreeVecPooled size"))?;
    free_pooled_block(a, pool, head, size)
}

/// FreePooled(a0 = pool, a1 = block, d0 = size). A block of another pool, or a wrong size, is refused by name.
pub fn free_pooled<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    free_pooled_block(a, cpu.areg(0), cpu.areg(1), cpu.dreg(0))
}

fn free_pooled_block<U: GuestUi>(a: &mut Amiga<U>, pool: u32, at: u32, size: u32) -> Result<String, Refusal> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use guestui_abi::RecordingGuestUi;

    fn machine() -> Amiga<RecordingGuestUi> { Amiga::new(RecordingGuestUi::new()).unwrap() }

    #[test]
    fn copymem_of_a_range_outside_ram_is_a_named_refusal_and_allocates_nothing_the_size_of_the_request() {
        let mut a = machine();
        let ram = a.mem.len() as u32;
        let src = a.heap.alloc(64).unwrap();
        // 4 GiB - 1 bytes: the old code built a host Vec of that size before reading a single byte.
        let e = copy_guest(&mut a, src, src, u32::MAX).unwrap_err();
        assert!(e.0.contains("CopyMem"), "{}", e.0);
        let e = copy_guest(&mut a, ram - 8, src, 64).unwrap_err();
        assert!(e.0.contains("CopyMem source"), "{}", e.0);
        let e = copy_guest(&mut a, src, ram - 8, 64).unwrap_err();
        assert!(e.0.contains("CopyMem dest"), "{}", e.0);
        // a source range that wraps the 32-bit space must be refused, not read as low RAM
        assert!(copy_guest(&mut a, u32::MAX - 3, src, 16).is_err());
    }

    #[test]
    fn copymem_copies_forwards_and_an_overlapping_copy_reads_the_source_before_it_is_written() {
        let mut a = machine();
        let p = a.heap.alloc(32).unwrap();
        a.mem.write_bytes(p, &(1u8..=16).collect::<alloc::vec::Vec<_>>()).unwrap();
        copy_guest(&mut a, p, p + 4, 12).unwrap(); // overlapping, destination above the source
        let got: alloc::vec::Vec<u8> = (0..16).map(|i| a.mem.read_u8(p + i).unwrap()).collect();
        assert_eq!(got, [1, 2, 3, 4, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12], "memmove semantics, as before");
        copy_guest(&mut a, p, p, 0).unwrap(); // control: zero bytes
    }

    #[test]
    fn stackswap_with_a_struct_or_stack_at_the_top_of_the_address_space_is_refused_not_wrapped() {
        let mut a = machine();
        let e = swap_stacks(&mut a, u32::MAX - 5, 0x1000).unwrap_err();
        assert!(e.0.contains("stk_"), "{}", e.0);
        // the new stack's return slot must lie inside [lower, upper]: stk_Pointer one above lower leaves no room for it
        let sss = a.heap.alloc(12).unwrap();
        let stack = a.heap.alloc(256).unwrap();
        for (i, v) in [stack, stack + 256, stack + 1].into_iter().enumerate() { a.mem.write_u32(sss + 4 * i as u32, v).unwrap(); }
        let sp = a.heap.alloc(8).unwrap();
        a.mem.write_u32(sp, 0x4000).unwrap();
        let e = swap_stacks(&mut a, sss, sp).unwrap_err();
        assert!(e.0.contains("StackSwap"), "{}", e.0);
        // an A7 so high that the saved caller SP (old_sp + 4) wraps
        a.mem.write_u32(sss + 8, stack + 256).unwrap();
        assert!(swap_stacks(&mut a, sss, u32::MAX - 1).is_err());
        // control: a sound swap returns the slot below stk_Pointer
        assert_eq!(swap_stacks(&mut a, sss, sp).unwrap(), stack + 256 - 4);
    }

    #[test]
    fn freevecpooled_of_a_pointer_below_the_size_longword_is_refused_not_underflowed() {
        let mut a = machine();
        let pool = new_pool(&mut a, 0);
        for bad in [1u32, 3] {
            let e = free_vec_pooled_block(&mut a, pool, bad).unwrap_err();
            assert!(e.0.contains("FreeVecPooled"), "{}", e.0);
        }
    }
}
