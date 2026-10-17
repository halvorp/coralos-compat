//! exec.library memory: AllocMem / FreeMem / AllocVec / FreeVec / AvailMem / CopyMem over a guest heap, plus the
//! single-task trivia C startups call next (Forbid / Permit / SetSignal).
//!
//! The heap's bookkeeping lives HOST-side (the guest cannot corrupt it): a first-fit free list and the set of live
//! blocks. A FreeMem of an address this heap never returned, or with a size other than the one allocated, is a named
//! refusal, not a silent corruption. One task only: Forbid/Permit are counted, not enforced.
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::format;
use crate::{machine::Amiga, process, Refusal};
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
/// Every block is 8-byte aligned and a multiple of 8 bytes long (AmigaOS MEM_BLOCKSIZE).
const GRAIN: u32 = 8;
/// tc_SigRecvd in the program's Task (see process.rs).
const TC_SIG_RECVD: u32 = 26;

/// The guest heap: [start, end) of guest RAM, a first-fit free list (address -> length) and the live blocks.
pub struct Heap { free: BTreeMap<u32, u32>, live: BTreeMap<u32, u32>, pub forbid: u32 }

impl Heap {
    pub fn new(start: u32, end: u32) -> Heap {
        let s = (start + GRAIN - 1) & !(GRAIN - 1);
        Heap { free: BTreeMap::from([(s, end - s)]), live: BTreeMap::new(), forbid: 0 }
    }
    /// First fit; None = no block large enough (AllocMem's NULL, a legitimate answer).
    pub fn alloc(&mut self, size: u32) -> Option<u32> {
        let want = size.checked_add(GRAIN - 1)? & !(GRAIN - 1);
        let (&at, &len) = self.free.iter().find(|(_, &l)| l >= want)?;
        self.free.remove(&at);
        if len > want { self.free.insert(at + want, len - want); }
        self.live.insert(at, want);
        Some(at)
    }
    /// Return a block. Refused unless `at` is a live block whose rounded size is `size`'s.
    pub fn free(&mut self, at: u32, size: u32) -> Result<(), String> {
        let want = size.checked_add(GRAIN - 1).ok_or("size overflows")? & !(GRAIN - 1);
        match self.live.get(&at) {
            None => return Err(format!("{at:#x} is not a block this heap allocated")),
            Some(&l) if l != want => return Err(format!("{at:#x} is a {l}-byte block, freed as {size}")),
            _ => {}
        }
        self.live.remove(&at);
        let mut start = at; let mut len = want;
        if let Some((&p, &pl)) = self.free.range(..at).next_back() { if p + pl == at { self.free.remove(&p); start = p; len += pl; } }
        if let Some(&nl) = self.free.get(&(at + want)) { self.free.remove(&(at + want)); len += nl; }
        self.free.insert(start, len);
        Ok(())
    }
    pub fn available(&self, largest: bool) -> u32 {
        if largest { self.free.values().copied().max().unwrap_or(0) } else { self.free.values().sum() }
    }
    pub fn live_blocks(&self) -> usize { self.live.len() }
}

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

pub fn forbid<U: GuestUi>(a: &mut Amiga<U>, _: &mut Cpu) -> Result<String, Refusal> {
    a.heap.forbid += 1;
    Ok(format!("[exec] Forbid (depth {})", a.heap.forbid))
}

pub fn permit<U: GuestUi>(a: &mut Amiga<U>, _: &mut Cpu) -> Result<String, Refusal> {
    a.heap.forbid = a.heap.forbid.checked_sub(1).ok_or_else(|| Refusal("Permit without Forbid".into()))?;
    Ok(format!("[exec] Permit (depth {})", a.heap.forbid))
}

/// SetSignal(d0 = newSignals, d1 = signalMask) -> d0 = the old signals: the program's own tc_SigRecvd.
pub fn set_signal<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (new, mask) = (cpu.dreg(0), cpu.dreg(1));
    let at = process::PROCESS + TC_SIG_RECVD;
    let old = a.mem.read_u32(at).map_err(Refusal::mem("tc_SigRecvd"))?;
    a.mem.write_u32(at, (old & !mask) | (new & mask)).map_err(Refusal::mem("tc_SigRecvd"))?;
    cpu.set_d0(old);
    Ok(format!("[exec] SetSignal new={new:#x} mask={mask:#x} -> {old:#x}"))
}

fn zero<U: GuestUi>(a: &mut Amiga<U>, at: u32, n: u32) -> Result<(), Refusal> {
    for i in 0..n { a.mem.write_u8(at + i, 0).map_err(Refusal::mem("MEMF_CLEAR"))?; }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn first_fit_rounds_coalesces_and_refuses_bad_frees() {
        let mut h = Heap::new(0x1000, 0x2000);
        let a = h.alloc(10).unwrap();
        let b = h.alloc(16).unwrap();
        assert_eq!((a, b), (0x1000, 0x1010), "10 rounds to 16");
        assert!(h.free(a, 20).is_err(), "wrong size refused");
        assert!(h.free(0x1004, 8).is_err(), "not a block refused");
        h.free(a, 10).unwrap();
        assert!(h.free(a, 10).is_err(), "double free refused");
        h.free(b, 16).unwrap();
        assert_eq!(h.available(true), 0x1000, "everything coalesced back into one block");
        assert!(h.alloc(0x1001).is_none(), "too large is NULL, not a panic");
        assert_eq!(h.live_blocks(), 0);
    }
}
