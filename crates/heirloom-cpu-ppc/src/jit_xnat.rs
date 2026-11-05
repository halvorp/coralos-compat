//! Tier 2's NATIVE ROUTINES (h86 native, 2026-11-04; jit_xlate's family): a hot guest routine - a whole function or a loop at
//! its head - recognised by its exact instruction words and run as RUST code over guest RAM and the Cpu, called from a small
//! JIT block, instead of being translated instruction by instruction. The Heirloom way ("interpret only the app's own code"):
//! the Z83's first real A/B priced generic code trimming at ~nothing and the three translated routines of jit_xlate at ~1 ms
//! a frame, because a routine run natively retires its guest instructions without executing JIT code for each - the hot
//! footprint shrinks with the host instructions.
//!
//! A ROUTINE EARNS ITS PLACE BY MEASURED HOST INSTRUCTIONS (callgrind over the WC3 gameplay census, the routine's own Rust
//! against the JIT code it replaces, /build/patches/h86/native): this JIT runs straight-line scalar guest code at ~3-6 host
//! instructions a guest instruction, so a scalar transcription barely beats it (h86 native measured setupVertexArrays's
//! skinning loop at 1.25x the JIT's host instructions in Rust, updateLightState's unchanged-light loop at break-even, both
//! dropped); a routine pays where its Rust does less work per element - no per-access bounds and store-map tests (one per
//! row), no state written per instruction, a scan over zero words.
//!
//! THE BLOCK (`compile`): the entry checks of any shape (the code page's generation under KNOB_ENTRY_CHECKS, a budget that
//! covers the routine's shortest unit), the global pins stored, `call [ctx.xnat]` =
//! `h_xnat::<H>(ctx, pc, id | logged << 8 | mutant << 16)`, the pins loaded back, and: 0 = BAIL (nothing written, nothing
//! retired: the dispatcher runs tier 1's block at pc), else on to `cpu.pc` through the dispatch table's IBTC probe.
//!
//! THE ROUTINES (`Routine::run`) are exact by construction: they compute the original instructions' effect - every register,
//! CR field, CTR, every byte stored, the retired count (subtracted from the Ctx's `left`) and the pc - and they REFUSE before
//! writing anything what they cannot reproduce: an access outside RAM, a store to a page the store map does not clear
//! (watched code: tier 1's write moves its generation), too little budget. A unit whose stores reach what it reads runs its
//! elements in the original's order on real memory (each read sees every earlier write); one whose stores reach nothing it
//! reads may take a faster path. A LOOP runs unit by unit (a row): each unit is validated before it writes, so a refusal after k
//! units leaves the exact state at the loop head after k trips (pc = the head, k units retired), and a refusal at the first
//! unit is a bail. Units run in program order on real memory, so what one unit stores, the next one reads.
//!
//! LOCKSTEP: every store is journaled first (`h_note`'s journal: the destination's old bytes), as inline stores are; a fast
//! path journals its whole destination range once.
use crate::cpu::{Cpu, XER_SO};
use crate::jit::{Ctx, EXIT_BAIL, EXIT_NORMAL};
use crate::jit_asm::*;
use crate::jit_emit::{cpu, ctx, Env, E, PC};
use crate::Host;
use alloc::vec::Vec;
use core::mem::offset_of;

/// What a routine answers: None = bail (nothing written), Some((pc, n)) = it retired `n` instructions and goes on at `pc`.
pub(crate) type Out = Option<(u32, u64)>;

/// A routine's view of the guest: the Cpu, guest RAM (`len` bytes at `ram`, addresses unmasked: shapes compile only when the
/// RAM mask is the identity), the store map, the budget left, and the lockstep journal.
pub(crate) struct X<'a> {
    pub(crate) cpu: &'a mut Cpu,
    ram: *mut u8,
    len: u64,
    smap: &'a [u8; crate::jit_smap::MAP_PAGES],
    pub(crate) left: u64,
    pub(crate) mutant: u8,
    note: Option<&'a mut dyn FnMut(u32, u32)>,
}

impl X<'_> {
    /// [ea, ea + n) lies inside RAM (n >= 1; a range that wraps past 2^32 never does).
    #[inline(always)]
    pub(crate) fn in_ram(&self, ea: u32, n: u32) -> bool { ea as u64 + n as u64 <= self.len }
    /// [ea, ea + n) lies inside RAM and every page of it is cleared for inline stores by the store map (not watched code).
    #[inline(always)]
    pub(crate) fn storable(&self, ea: u32, n: u32) -> bool {
        if !self.in_ram(ea, n) { return false; }
        let s = crate::jit_smap::PAGE_SHIFT;
        let (p0, p1) = ((ea >> s) as usize, ((ea + (n - 1)) >> s) as usize);
        (p0..=p1).all(|p| self.smap[p] != 0)
    }
    // Loads and stores of ranges a routine VALIDATED (in_ram / storable) before: big-endian, any alignment.
    #[inline(always)]
    pub(crate) fn rd8(&self, ea: u32) -> u32 {
        debug_assert!(self.in_ram(ea, 1));
        // SAFETY: the routine checked ea is inside the `len` bytes at `ram`.
        unsafe { *self.ram.add(ea as usize) as u32 }
    }
    #[inline(always)]
    pub(crate) fn rd16(&self, ea: u32) -> u32 {
        debug_assert!(self.in_ram(ea, 2));
        // SAFETY: the routine checked [ea, ea + 2) is inside the `len` bytes at `ram`.
        u16::from_be_bytes(unsafe { (self.ram.add(ea as usize) as *const [u8; 2]).read_unaligned() }) as u32
    }
    /// The two bytes at `ea` as the host loads them (little-endian: the big-endian halfword's bytes swapped) - for a test
    /// against a mask swapped once instead of every value swapped.
    #[inline(always)]
    pub(crate) fn rd16_raw(&self, ea: u32) -> u16 {
        debug_assert!(self.in_ram(ea, 2));
        // SAFETY: the routine checked [ea, ea + 2) is inside RAM.
        unsafe { (self.ram.add(ea as usize) as *const u16).read_unaligned() }
    }
    #[inline(always)]
    pub(crate) fn rd32(&self, ea: u32) -> u32 {
        debug_assert!(self.in_ram(ea, 4));
        // SAFETY: the routine checked [ea, ea + 4) is inside RAM.
        u32::from_be_bytes(unsafe { (self.ram.add(ea as usize) as *const [u8; 4]).read_unaligned() })
    }
    #[inline(always)]
    pub(crate) fn wr16(&mut self, ea: u32, v: u32) {
        debug_assert!(self.storable(ea, 2));
        if let Some(n) = self.note.as_mut() { n(ea, 2); }
        // SAFETY: the routine checked [ea, ea + 2) is inside RAM and storable.
        unsafe { (self.ram.add(ea as usize) as *mut [u8; 2]).write_unaligned((v as u16).to_be_bytes()) }
    }
    #[inline(always)]
    pub(crate) fn wr32(&mut self, ea: u32, v: u32) {
        debug_assert!(self.storable(ea, 4));
        if let Some(n) = self.note.as_mut() { n(ea, 4); }
        // SAFETY: the routine checked [ea, ea + 4) is inside RAM and storable.
        unsafe { (self.ram.add(ea as usize) as *mut [u8; 4]).write_unaligned(v.to_be_bytes()) }
    }
    // FAST PATHS: a unit whose whole destination range was validated (storable) and journaled ONCE (`note`) stores with the
    // `st*` forms - no per-store journal test; 
    /// Whether stores are journaled (lockstep): a routine skips computing what it would journal otherwise.
    #[inline(always)]
    pub(crate) fn journaling(&self) -> bool { self.note.is_some() }
    /// Journal [ea, ea + n) once before a fast path writes it (lockstep only; nothing otherwise).
    #[inline(always)]
    pub(crate) fn note(&mut self, ea: u32, n: u32) { if let Some(f) = self.note.as_mut() { f(ea, n); } }
    #[inline(always)]
    pub(crate) fn st16(&mut self, ea: u32, v: u32) {
        debug_assert!(self.storable(ea, 2));
        // SAFETY: the routine checked [ea, ea + 2) is inside RAM and storable, and journaled it.
        unsafe { (self.ram.add(ea as usize) as *mut [u8; 2]).write_unaligned((v as u16).to_be_bytes()) }
    }
    /// Four bytes at `ea` in memory order (`v`'s low byte first: the host is little-endian) - validated and journaled.
    #[inline(always)]
    pub(crate) fn st4_bytes(&mut self, ea: u32, v: u32) {
        debug_assert!(self.storable(ea, 4));
        // SAFETY: the routine checked [ea, ea + 4) is inside RAM and storable, and journaled it.
        unsafe { (self.ram.add(ea as usize) as *mut [u8; 4]).write_unaligned(v.to_le_bytes()) }
    }
    #[inline(always)]
    pub(crate) fn st32(&mut self, ea: u32, v: u32) {
        debug_assert!(self.storable(ea, 4));
        // SAFETY: the routine checked [ea, ea + 4) is inside RAM and storable, and journaled it.
        unsafe { (self.ram.add(ea as usize) as *mut [u8; 4]).write_unaligned(v.to_be_bytes()) }
    }
    /// XER[CA] = `ca` (a srawi's carry: the last one a routine reproduces).
    #[inline(always)]
    pub(crate) fn set_ca(&mut self, ca: bool) { self.cpu.xer = (self.cpu.xer & !crate::cpu::XER_CA) | if ca { crate::cpu::XER_CA } else { 0 }; }
    /// CR field 0 from a compare's LT / GT / EQ and XER[SO].
    #[inline(always)]
    pub(crate) fn cr0(&mut self, lt: bool, gt: bool, eq: bool) {
        let so = (self.cpu.xer & XER_SO != 0) as u32;
        self.cpu.cr = (self.cpu.cr & 0x0FFF_FFFF) | (lt as u32) << 31 | (gt as u32) << 30 | (eq as u32) << 29 | so << 28;
    }
    /// cmpw / cmpwi into CR0 (signed), cmplw / cmplwi (unsigned).
    #[inline(always)]
    pub(crate) fn cmpw0(&mut self, a: u32, b: u32) { let (a, b) = (a as i32, b as i32); self.cr0(a < b, a > b, a == b) }
    #[inline(always)]
    pub(crate) fn cmplw0(&mut self, a: u32, b: u32) { self.cr0(a < b, a > b, a == b) }
}

/// Two half-open ranges overlap.
#[inline(always)]
pub(crate) fn overlap(a: u32, an: u32, b: u32, bn: u32) -> bool { (a as u64) < b as u64 + bn as u64 && (b as u64) < a as u64 + an as u64 }

/// The routines (each one's knob: heirloom_cpu_abi::ppc::KNOB_NO_NAT_*).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Routine {
    /// jit_xpix::PALBLIT: resizePaletteAndBlit's rows of 16 palette-indexed pixels to 16-bit.
    PalBlit,
    /// jit_xpix::TEXCONV: texture_format_convert's rows of 16-bit pixels rotated by a nibble (ARGB4444 -> RGBA4444).
    TexConv,
    /// jit_xpix::BITSCAN: clearBitset's skip over zero words.
    BitScan,
    /// jit_xpix::ALPHASET: alphaBlendMaskRows's unmasked rows (each 16-bit half's top bit from its low 15 bits).
    AlphaSet,
    /// jit_xpix::ALPHAMASK: alphaBlendMaskRows's masked rows.
    AlphaMask,
    /// jit_xscan::GRIDSCAN: checkCollisionDetection's cell loop.
    GridScan,
    /// jit_xpix::TILEEXPAND: terrainTileBlend's 4 x 4 corner expansion of a dirty tile.
    TileExpand,
}

impl Routine {
    pub(crate) const ALL: [Routine; 7] = [Routine::PalBlit, Routine::TexConv, Routine::BitScan, Routine::AlphaSet, Routine::AlphaMask, Routine::GridScan, Routine::TileExpand];
    /// The routine's exact words (recognised from its first word: one read for every other block).
    pub(crate) fn words(self) -> &'static [u32] {
        match self {
            Routine::PalBlit => &crate::jit_xpix::PALBLIT,
            Routine::TexConv => &crate::jit_xpix::TEXCONV,
            Routine::BitScan => &crate::jit_xpix::BITSCAN,
            Routine::AlphaSet => &crate::jit_xpix::ALPHASET,
            Routine::AlphaMask => &crate::jit_xpix::ALPHAMASK,
            Routine::GridScan => &crate::jit_xscan::GRIDSCAN,
            Routine::TileExpand => &crate::jit_xpix::TILEEXPAND,
        }
    }
    /// Its knob: set = the routine is not recognised (compiled block by block like any code).
    pub(crate) fn knob(self) -> u32 {
        use heirloom_cpu_abi::ppc::*;
        match self {
            Routine::PalBlit => KNOB_NO_NAT_PALBLIT,
            Routine::TexConv => KNOB_NO_NAT_TEXCONV,
            Routine::BitScan => KNOB_NO_NAT_BITSCAN,
            Routine::AlphaSet | Routine::AlphaMask => KNOB_NO_NAT_ALPHA,
            Routine::GridScan => KNOB_NO_NAT_GRIDSCAN,
            Routine::TileExpand => KNOB_NO_NAT_TILE,
        }
    }
    /// The fewest instructions one unit retires (the block's entry budget test; each unit tests its own exact need).
    pub(crate) fn min_insns(self) -> u32 {
        match self {
            Routine::PalBlit => crate::jit_xpix::PALBLIT_ROW_MIN,
            Routine::TexConv => crate::jit_xpix::TEXCONV_ROW_MIN,
            Routine::BitScan => 4,
            Routine::AlphaSet => crate::jit_xpix::ALPHASET_TRIP,
            Routine::AlphaMask => crate::jit_xpix::ALPHAMASK_TRIP,
            Routine::GridScan => crate::jit_xscan::GRIDSCAN_TRIP_MAX,
            Routine::TileExpand => crate::jit_xpix::TILE_PREFIX + crate::jit_xpix::TILE_ROW,
        }
    }
    /// The budget the dispatcher wants before entering it (Slot::len): a few units, so a lockstep entry (budget = this + up to
    /// 63) runs the routine instead of bailing.
    pub(crate) fn slot_len(self) -> u32 {
        match self {
            Routine::PalBlit => 2 * crate::jit_xpix::PALBLIT_ROW_MAX,
            Routine::TexConv => 4 * 14 + 4 * 64,
            Routine::BitScan => 64 * crate::jit_xpix::BITSCAN_ZERO,
            Routine::AlphaSet => 4 * crate::jit_xpix::ALPHASET_TRIP,
            Routine::AlphaMask => 4 * crate::jit_xpix::ALPHAMASK_TRIP,
            Routine::GridScan => 64 * crate::jit_xscan::GRIDSCAN_TRIP_MAX,
            Routine::TileExpand => crate::jit_xpix::TILE_PREFIX + 4 * crate::jit_xpix::TILE_ROW,
        }
    }
    fn id(self) -> u32 { Routine::ALL.iter().position(|&r| r == self).unwrap() as u32 }
    fn run(self, x: &mut X, pc: u32) -> Out {
        match self {
            Routine::PalBlit => crate::jit_xpix::palblit(x, pc),
            Routine::TexConv => crate::jit_xpix::texconv(x, pc),
            Routine::BitScan => crate::jit_xpix::bitscan(x, pc),
            Routine::AlphaSet => crate::jit_xpix::alphaset(x, pc),
            Routine::AlphaMask => crate::jit_xpix::alphamask(x, pc),
            Routine::GridScan => crate::jit_xscan::gridscan(x, pc),
            Routine::TileExpand => crate::jit_xpix::tileexpand(x, pc),
        }
    }
}

/// JIT code's call into routine `arg & 0xFF` at `pc` (`arg >> 8 & 1`: lockstep journaling; `arg >> 16`: the planted mutant).
/// The Cpu holds every global pin (stored before the call). Answers 0 = bail (nothing written), 1 = go on at cpu.pc (the
/// routine's instructions subtracted from the Ctx's `left`).
pub(crate) extern "C" fn h_xnat<H: Host>(ctx: *mut Ctx, pc: u32, arg: u32) -> u32 {
    // SAFETY: as in jit::h_one - JIT code passes the Ctx run_code filled for this entry; `host` and `cpu` point at the live
    // `&mut H` and `&mut Cpu` of that call, which Rust does not use while JIT code runs.
    let c = unsafe { &mut *ctx };
    let (host, cpu) = unsafe { (&mut *(c.host as *mut H), &mut *c.cpu) };
    let r = Routine::ALL[(arg & 0xFF) as usize];
    let mut journal = |ea: u32, n: u32| host.mem().journal_note(ea, n);
    let mut x = X { cpu, ram: c.ram, len: c.len, smap: &c.smap, left: c.left, mutant: (arg >> 16) as u8, note: if arg & 0x100 != 0 { Some(&mut journal) } else { None } };
    match r.run(&mut x, pc) {
        None => 0,
        Some((next, n)) => {
            debug_assert!(n <= c.left);
            x.cpu.pc = next;
            c.left -= n;
            1
        }
    }
}

/// The JIT block of routine `r` at `pc` (code page `page` at generation `gen`): the code and the offset of its inner entry.
pub(crate) fn compile(r: Routine, pc: u32, gen: u64, page: usize, env: &mut Env) -> (Vec<u8>, usize) {
    let mut e = E::new(&[], pc, env);
    e.cat(crate::jit_census::XLATE);
    let mut bails = Vec::new();
    let check_gen = crate::jit_inval::entry_checks(e.env.knobs);
    let arg = r.id() | (e.env.logged as u32) << 8 | (e.env.mutant as u32) << 16;
    let (chain, table_mask) = (e.env.chain, e.env.table_mask);
    let a = &mut e.a;
    if check_gen {
        a.ld64(RAX, ctx(offset_of!(Ctx, gen)));
        let at = m(RAX, (8 * page) as i32);
        if gen <= i32::MAX as u64 { a.alu_mi64(Alu::Cmp, at, gen as u32); } else { a.mov_ri64(RCX, gen); a.alu_rm64(Alu::Cmp, RCX, at); }
        bails.push(a.jcc(Cc::Ne));
    }
    let inner = a.pos();
    a.alu_mi64(Alu::Cmp, ctx(offset_of!(Ctx, left)), r.min_insns());
    bails.push(a.jcc(Cc::B));
    // the routine reads and writes the Cpu's GPRs, CR and CTR (never FPSCR: a pending FPRF may stay pending): the global pins
    a.mov_rr64(RDI, R13);
    a.mov_ri(RSI, pc);
    a.mov_ri(RDX, arg);
    crate::jit_regs::store_globals(a);
    a.call_m(ctx(offset_of!(Ctx, xnat)));
    crate::jit_regs::load_globals(a);
    a.test_rr(RAX, RAX);
    bails.push(a.jcc(Cc::E));
    if chain {
        a.ld32(R8, cpu(PC));
        crate::jit_emit::probe(a, table_mask);
    }
    a.mov_ri(RAX, EXIT_NORMAL);
    a.jmp_m(ctx(offset_of!(Ctx, exit)));
    for b in bails { a.here(b); }
    a.st32i(cpu(PC), pc);
    a.mov_ri(RAX, EXIT_BAIL);
    a.jmp_m(ctx(offset_of!(Ctx, exit)));
    e.env.tag_runs = e.a.tags.take().unwrap_or_default();
    (e.a.b, inner)
}
