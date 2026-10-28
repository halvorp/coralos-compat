//! exec cache control: CacheClearU / CacheClearE / CacheControl on a machine whose 68k core (Musashi) has no cache.
//!
//! The 68020 has an instruction cache only (no data cache, no copyback), so the cache bits that can ever be set are
//! CACRF_EnableI (1<<0) and CACRF_FreezeI (1<<1); the machine starts with EnableI set, as Kickstart leaves a 68020.
//! CacheClearU and CacheClearE do nothing (nothing is cached, so nothing can be stale: self-modifying code and a patched
//! library vector just work), CacheControl keeps the two bits and returns the OLD bits; the clear and 030/040-only bits
//! (ClearI 1<<3, IBE 1<<4, EnableD 1<<8, ...) are accepted and ignored. exec/execbase.h offsets are in the LVO names.
use alloc::format;
use alloc::string::String;
use crate::libtable::LibraryBuilder;
use crate::machine::Amiga;
use crate::Refusal;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

pub const CACRF_ENABLE_I: u32 = 1 << 0;
pub const CACRF_FREEZE_I: u32 = 1 << 1;
const KEPT: u32 = CACRF_ENABLE_I | CACRF_FREEZE_I;

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.named("CacheClearU", clear_u::<U>).named("CacheClearE", clear_e::<U>).named("CacheControl", control::<U>)
}

fn clear_u<U: GuestUi>(_: &mut Amiga<U>, _: &mut Cpu) -> Result<String, Refusal> { Ok("[exec] CacheClearU (no cache to clear)".into()) }

fn clear_e<U: GuestUi>(_: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    Ok(format!("[exec] CacheClearE {:#x}+{} caches {:#x} (no cache to clear)", cpu.areg(0), cpu.dreg(0), cpu.dreg(1)))
}

/// CacheControl(d0 = cacheBits, d1 = cacheMask) -> the old bits.
fn control<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (bits, mask) = (cpu.dreg(0), cpu.dreg(1));
    let old = a.cache_bits;
    a.cache_bits = (old & !(mask & KEPT)) | (bits & mask & KEPT);
    cpu.set_d0(old);
    Ok(format!("[exec] CacheControl bits={bits:#x} mask={mask:#x} -> old {old:#x}, now {:#x} (68020: instruction cache bits only)", a.cache_bits))
}
