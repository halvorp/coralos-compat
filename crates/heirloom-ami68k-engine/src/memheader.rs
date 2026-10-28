//! exec.library Allocate / Deallocate: suballocation from a MemHeader the PROGRAM owns (exec/memory.h). The free list
//! is guest memory, so this is exec's own algorithm run on guest structures: MemHeader { Node 0..14, mh_Attributes 14,
//! mh_First 16, mh_Lower 20, mh_Upper 24, mh_Free 28 } (32 bytes); MemChunk { mc_Next 0, mc_Bytes 4 }; sizes rounded
//! to MEM_BLOCKSIZE (8). Allocate is first-fit taking from the FRONT of a chunk; Deallocate inserts in address order and
//! merges with both neighbours. Walks are bounded; a block outside [mh_Lower, mh_Upper) is refused by name.
use alloc::string::String;
use alloc::format;
use crate::{machine::Amiga, Refusal};
use heirloom_ui_abi::GuestUi;
use heirloom_mem_flat32::GuestMem;
use crate::cpu::Cpu;

pub const LVO_ALLOCATE: i32 = -186;
pub const LVO_DEALLOCATE: i32 = -192;
const MH_FIRST: u32 = 16;
const MH_LOWER: u32 = 20;
const MH_UPPER: u32 = 24;
const MH_FREE: u32 = 28;
const BLOCK: u32 = 8;
const MAX_WALK: u32 = 1 << 20;

fn m(what: &'static str) -> impl Fn(heirloom_mem_flat32::MemError) -> Refusal { Refusal::mem(what) }
fn round(n: u32) -> Option<u32> { n.checked_add(BLOCK - 1).map(|v| v & !(BLOCK - 1)) }

/// First fit from `mh`; Ok(None) = no chunk large enough (Allocate's NULL).
pub fn allocate(mem: &mut GuestMem, mh: u32, size: u32) -> Result<Option<u32>, Refusal> {
    let Some(want) = round(size).filter(|&w| w != 0) else { return Ok(None) };
    let mut prev = mh + MH_FIRST; // the field that points at `cur`
    let mut cur = mem.read_u32(prev).map_err(m("mh_First"))?;
    for _ in 0..MAX_WALK {
        if cur == 0 { return Ok(None); }
        let (next, bytes) = (mem.read_u32(cur).map_err(m("mc_Next"))?, mem.read_u32(cur + 4).map_err(m("mc_Bytes"))?);
        if bytes >= want {
            let rest = if bytes == want { next } else {
                let r = cur + want;
                mem.write_u32(r, next).map_err(m("mc_Next"))?;
                mem.write_u32(r + 4, bytes - want).map_err(m("mc_Bytes"))?;
                r
            };
            mem.write_u32(prev, rest).map_err(m("chunk link"))?;
            let free = mem.read_u32(mh + MH_FREE).map_err(m("mh_Free"))?;
            mem.write_u32(mh + MH_FREE, free.saturating_sub(want)).map_err(m("mh_Free"))?;
            return Ok(Some(cur));
        }
        prev = cur;
        cur = next;
    }
    Err(Refusal(format!("Allocate: MemHeader {mh:#x} chunk list longer than {MAX_WALK} (corrupt?)")))
}

/// Give `size` bytes at `block` back to `mh`, merging with its neighbours.
pub fn deallocate(mem: &mut GuestMem, mh: u32, block: u32, size: u32) -> Result<(), Refusal> {
    let Some(len) = round(size).filter(|&l| l != 0) else { return Ok(()) };
    let (lo, hi) = (mem.read_u32(mh + MH_LOWER).map_err(m("mh_Lower"))?, mem.read_u32(mh + MH_UPPER).map_err(m("mh_Upper"))?);
    if block < lo || block.checked_add(len).map_or(true, |e| e > hi) || block % BLOCK != 0 {
        return Err(Refusal(format!("Deallocate: {block:#x}+{len} is not inside MemHeader {mh:#x} [{lo:#x},{hi:#x})")));
    }
    let mut prev_field = mh + MH_FIRST;
    let mut prev_chunk = 0u32;
    let mut cur = mem.read_u32(prev_field).map_err(m("mh_First"))?;
    for _ in 0..MAX_WALK {
        if cur == 0 || cur > block { break; }
        if cur + mem.read_u32(cur + 4).map_err(m("mc_Bytes"))? > block { return Err(Refusal(format!("Deallocate: {block:#x} is already free"))); }
        prev_chunk = cur;
        prev_field = cur;
        cur = mem.read_u32(cur).map_err(m("mc_Next"))?;
    }
    if cur != 0 && block + len > cur { return Err(Refusal(format!("Deallocate: {block:#x}+{len} overlaps the free chunk at {cur:#x}"))); }
    // Merge with the following chunk, then with the preceding one.
    let (mut start, mut bytes, next) = if cur != 0 && block + len == cur {
        (block, len + mem.read_u32(cur + 4).map_err(m("mc_Bytes"))?, mem.read_u32(cur).map_err(m("mc_Next"))?)
    } else { (block, len, cur) };
    if prev_chunk != 0 && prev_chunk + mem.read_u32(prev_chunk + 4).map_err(m("mc_Bytes"))? == start {
        bytes += mem.read_u32(prev_chunk + 4).map_err(m("mc_Bytes"))?;
        start = prev_chunk;
    } else {
        mem.write_u32(prev_field, start).map_err(m("chunk link"))?;
    }
    mem.write_u32(start, next).map_err(m("mc_Next"))?;
    mem.write_u32(start + 4, bytes).map_err(m("mc_Bytes"))?;
    let free = mem.read_u32(mh + MH_FREE).map_err(m("mh_Free"))?;
    mem.write_u32(mh + MH_FREE, free + len).map_err(m("mh_Free"))
}

pub fn allocate_lvo<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (mh, size) = (cpu.areg(0), cpu.dreg(0));
    let at = allocate(&mut a.mem, mh, size)?.unwrap_or(0);
    cpu.set_d0(at);
    Ok(format!("[exec] Allocate {mh:#x} size={size} -> {at:#x}"))
}

pub fn deallocate_lvo<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (mh, block, size) = (cpu.areg(0), cpu.areg(1), cpu.dreg(0));
    deallocate(&mut a.mem, mh, block, size)?;
    Ok(format!("[exec] Deallocate {mh:#x} {block:#x} size={size}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn header(mem: &mut GuestMem, mh: u32, lo: u32, len: u32) {
        mem.write_u32(mh + MH_FIRST, lo).unwrap(); mem.write_u32(mh + MH_LOWER, lo).unwrap();
        mem.write_u32(mh + MH_UPPER, lo + len).unwrap(); mem.write_u32(mh + MH_FREE, len).unwrap();
        mem.write_u32(lo, 0).unwrap(); mem.write_u32(lo + 4, len).unwrap();
    }
    #[test]
    fn first_fit_split_merge_and_refusals() {
        let mut mem = GuestMem::new(0x10000);
        header(&mut mem, 0x100, 0x1000, 0x100);
        let a = allocate(&mut mem, 0x100, 10).unwrap().unwrap();
        let b = allocate(&mut mem, 0x100, 16).unwrap().unwrap();
        assert_eq!((a, b), (0x1000, 0x1010), "rounded to 16, taken from the front");
        assert_eq!(mem.read_u32(0x100 + MH_FREE).unwrap(), 0x100 - 32);
        assert!(allocate(&mut mem, 0x100, 0x1000).unwrap().is_none(), "too large is NULL");
        assert!(deallocate(&mut mem, 0x100, 0x2000, 8).is_err(), "outside the header refused");
        deallocate(&mut mem, 0x100, a, 10).unwrap();
        assert!(deallocate(&mut mem, 0x100, a, 10).is_err(), "double free refused");
        deallocate(&mut mem, 0x100, b, 16).unwrap();
        assert_eq!((mem.read_u32(0x100 + MH_FIRST).unwrap(), mem.read_u32(0x1004).unwrap()), (0x1000, 0x100), "merged back into one chunk");
        assert_eq!(mem.read_u32(0x100 + MH_FREE).unwrap(), 0x100);
    }
}
