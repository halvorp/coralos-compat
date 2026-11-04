//! Tier 2's h86 fnreg translated shapes against tier 0, each with a planted mutant that must make it RED: the matrix
//! compare's vector fast path (jit_xlate), the unsigned byte compare and the saturating byte-add loop (jit_xbytes). Every
//! case is a CALL of the shape's exact words from a driver loop that folds each result register into accumulators; tier 0
//! answers, Tier::Jit in four slicings and JitLockstep (a slice large enough that its entries run the translation) agree.
use super::*;

const DATA: u32 = 0x1_0000;
const TBL: u32 = 0x3_0000; // 64 KiB aligned: `lis` alone loads it
const XMC: u32 = 0x2_2000;
const XBL: u32 = 0x2_3000;

fn bl(from: u32, to: u32) -> u32 { 18 << 26 | (to.wrapping_sub(from) & 0x03FF_FFFC) | 1 }
fn lwz(rt: u32, ra: u32, d: u32) -> u32 { super::d(32, rt, ra, d) }
fn add(rt: u32, ra: u32, rb: u32) -> u32 { x(rt, ra, rb, 266, false) }
fn mfcr(rt: u32) -> u32 { 31 << 26 | rt << 21 | 19 << 1 }
fn mfspr(rt: u32, spr: u32) -> u32 { 31 << 26 | rt << 21 | (spr & 31) << 16 | (spr >> 5) << 11 | 339 << 1 }

/// A driver: 150 rounds over `cases` table rows of `row` bytes at TBL; per row `load` fills the argument registers from r30,
/// `bl` to `f`, then `folds` adds result registers into r20.. (r20 rotated first, so the ORDER of results counts too).
fn driver(cases: u32, row: u32, load: &[u32], f: u32, folds: &[u32]) -> Vec<u32> {
    let mut p = vec![d(14, 31, 0, 150), d(14, 20, 0, 0)]; // li r31,150; li r20,0
    let outer = p.len();
    p.push(d(15, 30, 0, TBL >> 16)); // lis r30,TBL
    let inner = p.len();
    p.extend_from_slice(load);
    let at = CODE + 4 * p.len() as u32;
    p.push(bl(at, f));
    p.push(rlwinm(20, 20, 1, 0, 31, false)); // rotlwi r20,r20,1
    for (k, &r) in folds.iter().enumerate() { p.push(add(20 + k as u32 % 6, 20 + k as u32 % 6, r)); }
    p.push(mfcr(26));
    p.push(add(21, 21, 26));
    p.push(mfspr(26, 1)); // mfxer
    p.push(add(22, 22, 26));
    p.push(d(14, 30, 30, row)); // addi r30,r30,row
    p.push(d(15, 26, 0, (TBL + row * cases) >> 16));
    p.push(d(24, 26, 26, (TBL + row * cases) & 0xFFFF)); // r26 = end of the table
    p.push(x(0, 30, 26, 0, false)); // cmpw cr0,r30,r26
    let b = p.len();
    p.push(bc(4, 2, (inner as i32 - b as i32) * 4)); // bne inner
    p.push(d(13, 31, 31, 0xFFFF)); // addic. r31,r31,-1
    let b = p.len();
    p.push(bc(4, 2, (outer as i32 - b as i32) * 4)); // bne outer
    p.extend([d(15, 24, 0, EXIT >> 16), d(24, 24, 24, EXIT & 0xFFFF), mtspr(8, 24)]);
    p
}

// ---- the unsigned byte compare (MEMCMPB) ----

/// Rows (a, b, n): equal for n = 0 / 1 / 27 / 100 / 300 (past the dispatcher's typical budget), the same pointer twice,
/// differences at byte 0 (a > b), 5 (a < b), 13 of 40, the last of 40, and a high-bit pair (unsigned: 0x90 > 0x10).
fn memcmp_rows() -> Vec<(u32, u32, u32)> {
    let (s, s2) = (DATA, DATA + 0x400);
    let d = |k: u32| DATA + 0x800 + 0x200 * k;
    vec![(s, s2, 0), (s, s2, 1), (s, s2, 27), (s, s2, 100), (s, d(0), 30), (s, d(1), 30), (s, d(2), 40), (s, d(3), 40),
         (s, s2, 300), (s, s, 50), (d(4), d(5), 16)]
}
fn memcmp_setup(_: &mut Cpu, m: &mut M) {
    for (i, &w) in heirloom_cpu_ppc::XLATE_MEMCMPB.iter().enumerate() { m.mem.write_u32(XMC + 4 * i as u32, w).unwrap(); }
    let base: Vec<u8> = (0..0x200u32).map(|i| (i * 7 + 3) as u8).collect();
    m.mem.write_bytes(DATA, &base).unwrap();
    m.mem.write_bytes(DATA + 0x400, &base).unwrap();
    for (k, (at, f)) in [(0usize, 0xFFu8), (5, 1), (13, 0x40), (39, 1)].into_iter().enumerate() {
        let mut v = base.clone();
        v[at] = if k == 0 { v[at].wrapping_sub(1) } else if k == 1 { v[at].wrapping_add(f) } else { v[at] ^ f };
        m.mem.write_bytes(DATA + 0x800 + 0x200 * k as u32, &v).unwrap();
    }
    let mut h = base[..16].to_vec();
    h[9] = 0x90;
    m.mem.write_bytes(DATA + 0x800 + 0x200 * 4, &h).unwrap();
    h[9] = 0x10;
    m.mem.write_bytes(DATA + 0x800 + 0x200 * 5, &h).unwrap();
    for (i, (a, b, n)) in memcmp_rows().into_iter().enumerate() {
        for (j, v) in [a, b, n].into_iter().enumerate() { m.mem.write_u32(TBL + 12 * i as u32 + 4 * j as u32, v).unwrap(); }
    }
}
fn memcmp_program() -> Vec<u32> {
    driver(memcmp_rows().len() as u32, 12, &[lwz(3, 30, 0), lwz(4, 30, 4), lwz(5, 30, 8)], XMC, &[3, 4, 0, 6, 7])
}

fn check(name: &str, prog: &[u32], setup: Setup, min_translated: u64) -> End {
    let (want, _) = run_on(Tier::Interp, 0, prog, 1_000_000, setup);
    for slice in [1_000_000, 1_000, 61, 13] {
        let (got, e) = run_on(Tier::Jit, 0, prog, slice, setup);
        assert!(got == want, "{name} slice {slice}: gpr {:x?} vs {:x?}, cr {:#x}/{:#x}, xer {:#x}/{:#x}, tb {}/{} {:?}",
            got.gpr, want.gpr, got.cr, want.cr, got.xer, want.xer, got.tb, want.tb, e.jit_stats());
        // (a slice below the shape's own block length never compiles it: tier 1 runs that block)
        if slice >= 61 { assert!(e.jit_stats().translated >= min_translated, "{name} slice {slice}: translated {:?}", e.jit_stats()); }
    }
    let (got, e) = run_on(Tier::JitLockstep, 0, prog, 100_000, setup);
    assert!(got == want && e.divergence().is_none(), "{name} lockstep: {:?}", e.divergence());
    want
}

/// The byte compare's translation ends every call in tier 0's exact state (r0 r3 r4 r6 r7, CR0, XER[CA], the retired count)
/// - equal, every kind of difference, n = 0 - and the planted count mutant (the a < b path's two extra instructions dropped)
/// goes RED.
#[test]
fn the_translated_byte_compare_is_exact_and_its_mutant_goes_red() {
    let prog = memcmp_program();
    let want = check("memcmp", &prog, memcmp_setup, 1);
    let (got, _) = run_on(Tier::Jit, 40, &prog, 1_000_000, memcmp_setup);
    assert!(got != want, "mutant 40 (the a < b count) stayed GREEN");
}

// ---- the saturating byte-add loop (BLENDADD) ----

/// The loop's wrapper at XBL: `mtctr r9; cmpwi r9,0; beq out` (the not-taken path into the head, as in the game), the 36
/// words, `out: blr`.
fn blend_setup_words(m: &mut M) {
    let mut w = vec![mtspr(9, 9), d(11, 0, 9, 0), bc(12, 2, 4 * (3 + 36 - 2))];
    w.extend_from_slice(&heirloom_cpu_ppc::XLATE_BLENDADD);
    w.push(0x4E80_0020);
    for (i, &v) in w.iter().enumerate() { m.mem.write_u32(XBL + 4 * i as u32, v).unwrap(); }
}
/// Rows (a, b, dst, groups): disjoint 1 / 5 / 32 groups (the 16-byte and 4-byte passes), in place on a and on b, a destination
/// one byte past a (a partial overlap: tier 1's), sums that saturate everywhere, and a destination on the routine's own code
/// page (the store map refuses it: tier 1's write moves the page's generation).
fn blend_rows() -> Vec<(u32, u32, u32, u32)> {
    let (a, b, o) = (DATA, DATA + 0x400, DATA + 0x800);
    vec![(a, b, o, 1), (a, b, o + 0x100, 5), (a, b, o + 0x200, 32), (o + 0x400, b, o + 0x400, 9), (a, o + 0x500, o + 0x500, 9),
         (o + 0x600, b, o + 0x601, 8), (DATA + 0xC00, DATA + 0xD00, o + 0x700, 6), (a, b, XBL + 0x800, 4)]
}
fn blend_setup(_: &mut Cpu, m: &mut M) {
    blend_setup_words(m);
    let pat = |k: u32| -> Vec<u8> { (0..0x200u32).map(|i| (i * 37 + k * 11) as u8).collect() };
    m.mem.write_bytes(DATA, &pat(1)).unwrap();
    m.mem.write_bytes(DATA + 0x400, &pat(2)).unwrap();
    m.mem.write_bytes(DATA + 0x800 + 0x400, &pat(3)).unwrap();
    m.mem.write_bytes(DATA + 0x800 + 0x500, &pat(4)).unwrap();
    m.mem.write_bytes(DATA + 0x800 + 0x600, &pat(5)).unwrap();
    m.mem.write_bytes(DATA + 0xC00, &[0xF0; 0x100]).unwrap();
    m.mem.write_bytes(DATA + 0xD00, &[0x81; 0x100]).unwrap();
    for (i, (a, b, d, c)) in blend_rows().into_iter().enumerate() {
        for (j, v) in [a, b, d, c].into_iter().enumerate() { m.mem.write_u32(TBL + 16 * i as u32 + 4 * j as u32, v).unwrap(); }
    }
}
fn blend_program() -> Vec<u32> {
    driver(blend_rows().len() as u32, 16, &[lwz(7, 30, 0), lwz(6, 30, 4), lwz(5, 30, 8), lwz(9, 30, 12)], XBL, &[8, 27, 0, 5, 7, 6])
}

/// The byte-add loop's translation leaves tier 0's bytes and registers (r0 r5 r6 r7 r8 r27, CTR = 0, the retired count) for
/// every row - in place, saturating, and the refused ones (a partial overlap, a destination on a code page) through tier 1 -
/// and with the overlap test planted out (mutant 39) the partial overlap goes RED.
#[test]
fn the_translated_byte_add_loop_is_exact_and_its_mutant_goes_red() {
    let prog = blend_program();
    let want = check("blend", &prog, blend_setup, 1);
    let (got, _) = run_on(Tier::Jit, 39, &prog, 1_000_000, blend_setup);
    assert!(got != want, "mutant 39 (no overlap test) stayed GREEN");
}

// ---- the matrix compare's vector fast path (CMP16F) ----

/// xlate_program's seven 128-byte cases, rewritten: all equal (the fast path), a bit-identical quiet NaN at pair 3 (fcmpu:
/// unordered, the routine answers 1; the fast path must refuse it), bit-identical infinities, +0 against -0 at pair 9, a
/// difference at pair 15, a bit-identical signalling NaN at pair 0, all equal again.
fn cmp16f_nan_setup(c: &mut Cpu, m: &mut M) {
    xlate_setup(c, m);
    let f = |v: f32| v.to_bits();
    for case in 0..XCASES {
        let (a, b) = (BUF + 128 * case, BUF + 128 * case + 64);
        for k in 0..16u32 {
            let base = f(2.5 - k as f32);
            let (va, vb) = match case {
                1 if k == 3 => (0x7FC0_0001, 0x7FC0_0001),
                2 if k == 4 => (f(f32::INFINITY), f(f32::INFINITY)),
                3 if k == 9 => (f(0.0), f(-0.0)),
                4 if k == 15 => (base, f(99.0)),
                5 if k == 0 => (0x7F80_0001, 0x7F80_0001),
                _ => (base, base),
            };
            m.mem.write_u32(a + 4 * k, va).unwrap();
            m.mem.write_u32(b + 4 * k, vb).unwrap();
        }
    }
}

/// The matrix compare with its vector fast path: tier 0's state for the NaN / infinity / signed-zero cases the fast path
/// must hand to the walk, and the planted mutant that lets NaN lanes through the fast path goes RED.
#[test]
fn the_matrix_compare_fast_path_refuses_nans_and_its_mutant_goes_red() {
    let prog = xlate_program();
    let want = check("cmp16f-nan", &prog, cmp16f_nan_setup, 1);
    assert_ne!(want.gpr[20], 0, "some calls answered 1");
    let (got, _) = run_on(Tier::Jit, 38, &prog, 1_000_000, cmp16f_nan_setup);
    assert!(got != want, "mutant 38 (NaN lanes through the fast path) stayed GREEN");
}
