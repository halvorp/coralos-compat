//! Tier 2's Ctx: what JIT code reads through r13 and what the helpers it calls find (jit.rs fills it before every entry).
use crate::cpu::{Cpu, Trap};
use crate::jit::{Slot, EMPTY_PC, TABLE_BITS};
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use core::mem::offset_of;

/// What JIT code reads through r13, and what its helpers find. Rebuilt before every entry.
#[repr(C)]
/// FIELD ORDER (h86 ppc-jit6): what JIT code reads on its hot paths comes first - the page generations, the budget, the common
/// exit, the pending FPRF source (jitsize: the store map's base pointer was here until the map moved into the Ctx), the
/// four RAM limits and the CR-field constants fill the first 128 bytes, so each is `[r13 + disp8]`
/// (one byte) instead of a 4-byte displacement; WC3's hot JIT code read crk at 0x1011c past the 64K-entry counters.
pub(crate) struct Ctx {
    /// The code watch's generation array (block entries compare their page's element).
    pub(crate) gen: *const u64,
    pub(crate) budget: u64,
    pub(crate) exit: u64,
    /// The pending FPSCR[FPRF] source (see `fprf_kind`), here for a one-byte displacement.
    pub(crate) fprf_src: u64,
    /// `len - n` for the n-byte accesses: an access at a (masked) EA is inside RAM iff EA <= lim_n.
    pub(crate) lim1: u64,
    pub(crate) lim2: u64,
    pub(crate) lim4: u64,
    pub(crate) lim8: u64,
    /// Per CR field f: the field's LT and GT nibbles already in place (8 << s, 4 << s; s = 28 - 4f) - the CMOV sources of
    /// jit_emit's cr_set.
    pub(crate) crk: [[u32; 2]; 8],
    // ---- 128 bytes: below, a 4-byte displacement ----
    /// The IBTC (Jit::ibtc) and the arena base its entry offsets are relative to.
    pub(crate) ibtc: *const u64,
    pub(crate) abase: u64,
    /// Indirect branches (blr / bctr / a native's return) that found their target in the dispatch table, and that did not.
    pub(crate) ibtc_hits: u64,
    pub(crate) ibtc_misses: u64,
    /// The shared code stubs JIT code calls (jit_inline): FPSCR[FPRF] of a single / double result, stfs's bits.
    pub(crate) fprf_s: u64,
    pub(crate) fprf_d: u64,
    pub(crate) stfs: u64,
    /// LAZY FPSCR[FPRF] (h86 jitsize): an FP result's class is not computed where it is made - JIT code stores the result's
    /// bits in `fprf_src` and `fprf_kind` (1 = single, 2 = double; 0 = FPSCR is exact) and FPSCR[FPRF] is brought up to date only
    /// where something can read it: the common exit, the slow stub (before tier 1's handler), a native block (before the
    /// native), and an inline FPSCR[FPCC] writer (fcmpu / fcmpo, the translated matrix compare) - each through `fprf_lazy`,
    /// the stub that classifies the pending result and clears the kind. A later FP result overwrites a pending one: its
    /// class is all FPRF ever showed.
    /// Its low byte is the kind (JIT code writes and tests one byte).
    pub(crate) fprf_kind: u64,
    pub(crate) fprf_lazy: u64,
    /// fmadds's exact-path stub (jit_fp emit_fma_odd), and the f64 bit patterns shifted up one (sign dropped) at which the
    /// exponent is 0x7FF (k_inf2: not finite) and below which it is under 897 (k_tiny2: under 2^-126, the single normal range).
    pub(crate) fma_odd: u64,
    pub(crate) k_inf2: u64,
    pub(crate) k_tiny2: u64,
    /// The shared slow stub, and `h_one::<H>` it calls.
    pub(crate) slow: u64,
    pub(crate) helper: u64,
    /// `h_native::<H>` (native blocks call it), the natives it ran this entry, and whether one changed what the dispatcher
    /// must resync (the natives set, guest RAM).
    pub(crate) native: u64,
    pub(crate) natives: u64,
    /// h_one calls this entry.
    pub(crate) helpers: u64,
    pub(crate) cpu: *mut Cpu,
    pub(crate) ram: *mut u8,
    pub(crate) len: u64,
    pub(crate) mask: u32,
    pad: u32,
    pub(crate) marked: *const u64,
    pub(crate) retired: u64,
    pub(crate) tb0: u64,
    pub(crate) host: *mut u8,
    pub(crate) table: *const Slot,
    pub(crate) trap: Option<Trap>,
    pub(crate) resync: bool,
    /// Planted mutant MUT_NO_RESYNC: h_native never asks for the resync.
    pub(crate) no_resync: bool,
    /// The code running is SO-free (Jit::so_aware false): h_one / h_native make it leave once XER[SO] is set.
    pub(crate) so_free: bool,
    /// Opt-in census of the instructions JIT code hands to h_one (key: primary opcode and extended opcode bits).
    pub(crate) census: Option<BTreeMap<u32, u64>>,
    /// The head of a block whose entry counter ran out (it left with nothing of it retired): the dispatcher forms a region
    /// there (jit_region). EMPTY_PC = none.
    pub(crate) hot: u32,
    /// Block entry counters, one per dispatch-table index (a colliding block shares one: it only miscounts), counting DOWN
    /// from the region threshold; a block's counter is reset when it is compiled. Inline: JIT code reaches one with a
    /// single `sub [r13 + disp32], 1`.
    pub(crate) counters: [u32; 1 << TABLE_BITS],
    /// Opt-in region census (Exec::set_jit_region_census): region entries and internal edges taken.
    pub(crate) rentries: u64,
    pub(crate) redges: u64,
    /// The STORE MAP (jit_smap): one byte per guest page, 1 = an inline store starting there may write; inside the Ctx so an
    /// inline store's test is `cmp byte [r13 + page + offset], 0` (no load of the map's base).
    pub(crate) smap: [u8; crate::jit_smap::MAP_PAGES],
}

impl Ctx {
    /// A Ctx on the heap (it holds the 1 MiB store map: never built on a stack), all zero but the constants.
    pub(crate) fn boxed() -> Box<Ctx> {
        let mut c = Box::<Ctx>::new_zeroed();
        // SAFETY: every field of Ctx is an integer, a bool, a raw pointer or an array of them - all-zero is a value of each -
        // except `trap` and `census` (Options of types with niches), which are written before the value is assumed made.
        let mut c = unsafe {
            let p = c.as_mut_ptr();
            core::ptr::addr_of_mut!((*p).trap).write(None);
            core::ptr::addr_of_mut!((*p).census).write(None);
            c.assume_init()
        };
        c.hot = EMPTY_PC;
        c.counters.fill(crate::jit_region::REGION_AFTER);
        c.crk = core::array::from_fn(|f| [8 << (28 - 4 * f), 4 << (28 - 4 * f)]);
        (c.k_inf2, c.k_tiny2) = (0xFFE0_0000_0000_0000, 897 << 53);
        c
    }
}


const _: () = assert!(offset_of!(Ctx, crk) + core::mem::size_of::<[[u32; 2]; 8]>() <= 128, "Ctx's hot fields must stay within a one-byte displacement of r13");
