//! Tier 2's h86 NATIVE ROUTINES (jit_xnat) against tier 0, each with a planted mutant that must make it RED: the palette blit,
//! the nibble-rotate rows and the bitset scan (jit_xpix). Every case CALLS the routine's exact words (a loop entered at its
//! head, `blr` after its exit) from a driver that sets its inputs from a table row and folds its result registers into
//! accumulators; the whole RAM and the final state are compared: tier 0 answers, Tier::Jit in four slicings and JitLockstep
//! agree. The cases include what each routine must REFUSE (a destination on a code page) and what it must reproduce through
//! memory order (a destination overlapping its own source, the flag word or the table it reads).
use super::*;

const TBL: u32 = 0x3_0000; // 64 KiB aligned: `lis` alone loads it
const XR: u32 = 0x2_4000; // the routine under test (its page also holds a "destination on a code page")

fn bl(from: u32, to: u32) -> u32 { 18 << 26 | (to.wrapping_sub(from) & 0x03FF_FFFC) | 1 }
fn lwz(rt: u32, ra: u32, d: u32) -> u32 { super::d(32, rt, ra, d) }
fn stw(rs: u32, ra: u32, d: u32) -> u32 { super::d(36, rs, ra, d) }
fn add(rt: u32, ra: u32, rb: u32) -> u32 { x(rt, ra, rb, 266, false) }
fn mfcr(rt: u32) -> u32 { 31 << 26 | rt << 21 | 19 << 1 }
fn mfspr(rt: u32, spr: u32) -> u32 { 31 << 26 | rt << 21 | (spr & 31) << 16 | (spr >> 5) << 11 | 339 << 1 }
fn li(rt: u32, v: u32) -> u32 { d(14, rt, 0, v) }

/// The driver's own registers: the round counter, the table row pointer, a scratch, the accumulators.
struct Regs { cnt: u32, tbl: u32, tmp: u32, acc: [u32; 6] }

/// `rounds` rounds over `cases` table rows of `row` bytes at TBL; per row `load` sets the routine's inputs from the row
/// (register `tbl`), `bl` to `f`, `post` (e.g. FPRs logged into the row), then `folds` add result registers into the
/// accumulators (the first rotated per call, so the ORDER of results counts), and CR and XER are folded too.
fn driver(cases: u32, row: u32, load: &[u32], f: u32, post: &[u32], folds: &[u32], r: &Regs, rounds: u32) -> Vec<u32> {
    let mut p = vec![li(r.cnt, rounds), li(r.acc[0], 0)];
    let outer = p.len();
    p.push(d(15, r.tbl, 0, TBL >> 16)); // lis tbl,TBL
    let inner = p.len();
    p.extend_from_slice(load);
    let at = CODE + 4 * p.len() as u32;
    p.push(bl(at, f));
    p.extend_from_slice(post);
    p.push(rlwinm(r.acc[0], r.acc[0], 1, 0, 31, false));
    for (k, &g) in folds.iter().enumerate() { let a = r.acc[k % 6]; p.push(add(a, a, g)); }
    p.push(mfcr(r.tmp));
    p.push(add(r.acc[1], r.acc[1], r.tmp));
    p.push(mfspr(r.tmp, 1)); // mfxer
    p.push(add(r.acc[2], r.acc[2], r.tmp));
    p.push(d(14, r.tbl, r.tbl, row)); // addi tbl,tbl,row
    p.push(d(15, r.tmp, 0, (TBL + row * cases) >> 16));
    p.push(d(24, r.tmp, r.tmp, (TBL + row * cases) & 0xFFFF)); // ori: the end of the table
    p.push(x(0, r.tbl, r.tmp, 0, false)); // cmpw cr0,tbl,tmp
    let b = p.len();
    p.push(bc(4, 2, (inner as i32 - b as i32) * 4)); // bne inner
    p.push(d(13, r.cnt, r.cnt, 0xFFFF)); // addic. cnt,cnt,-1
    let b = p.len();
    p.push(bc(4, 2, (outer as i32 - b as i32) * 4)); // bne outer
    p.extend([d(15, r.tmp, 0, EXIT >> 16), d(24, r.tmp, r.tmp, EXIT & 0xFFFF), mtspr(8, r.tmp)]);
    p
}

/// The routine's words at XR with `blr` after them (its loop's exit is the word after its last).
fn place(m: &mut M, words: &[u32]) {
    for (i, &w) in words.iter().chain([0x4E80_0020u32].iter()).enumerate() { m.mem.write_u32(XR + 4 * i as u32, w).unwrap(); }
}
fn table(m: &mut M, row: u32, rows: &[Vec<u32>]) {
    for (i, r) in rows.iter().enumerate() { for (j, &v) in r.iter().enumerate() { m.mem.write_u32(TBL + row * i as u32 + 4 * j as u32, v).unwrap(); } }
}
/// A deterministic byte pattern.
fn pat(n: u32, k: u32) -> Vec<u8> { (0..n).map(|i| (i.wrapping_mul(2_654_435_761u32.wrapping_add(k)) >> 13) as u8).collect() }

/// tier 0, Tier::Jit in four slicings, JitLockstep: the same End; the routine ran natively at least `min_translated` times.
fn check(name: &str, prog: &[u32], setup: Setup, min_translated: u64) -> End {
    let (want, _) = run_on(Tier::Interp, 0, prog, 1_000_000, setup);
    assert!(want.last.halted && want.last.trap.is_none(), "{name} tier 0: {:?}", want.last);
    for slice in [1_000_000, 1_000, 97, 13] {
        let (got, e) = run_on(Tier::Jit, 0, prog, slice, setup);
        assert!(got == want, "{name} slice {slice}: gpr {:x?} vs {:x?}, cr {:#x}/{:#x}, ctr {:#x}/{:#x}, fpscr {:#x}/{:#x}, tb {}/{}, ram equal {} {:?}",
            got.gpr, want.gpr, got.cr, want.cr, got.ctr, want.ctr, got.fpscr, want.fpscr, got.tb, want.tb, got.ram == want.ram, e.jit_stats());
        if slice >= 1_000 { assert!(e.jit_stats().translated >= min_translated, "{name} slice {slice}: translated {:?}", e.jit_stats()); }
    }
    let (got, e) = run_on(Tier::JitLockstep, 0, prog, 100_000, setup);
    assert!(got == want && e.divergence().is_none(), "{name} lockstep: {:?}", e.divergence());
    want
}

// ---- the palette blit (PALBLIT) ----

const PAL: u32 = 0x6_0000; // r26: the flag word at + 8604, the table at + 6552
const PSRC: u32 = 0x7_0000;
const PMASK: u32 = 0x7_8000;
const PDST: u32 = 0x8_0000;
const PB_ROW: u32 = 32;

/// Rows: mask (r4), src (r5), dst (r10), rows - 1 (r11), mask stride (r0), src stride (r3), the flag. Cases: the flag clear; set;
/// 0x100 with other strides; the destination ON the source; the destination over the flag word (pixel 3 rewrites it
/// mid-row); over the table; on the routine's own code page (refused: tier 1); 40 rows.
fn pb_rows() -> Vec<Vec<u32>> {
    vec![vec![PMASK, PSRC, PDST, 0, 16, 16, 0], vec![PMASK, PSRC, PDST + 0x100, 2, 16, 16, 1], vec![PMASK + 5, PSRC + 9, PDST + 0x200, 4, 24, 40, 0x100],
         vec![PMASK, PSRC + 0x400, PSRC + 0x400, 1, 16, 32, 1], vec![PMASK, PSRC, PAL + 8604 - 6, 0, 16, 16, 1], vec![PMASK, PSRC, PAL + 6552 + 0x40, 1, 16, 16, 1],
         vec![PMASK, PSRC, XR + 0x800, 1, 16, 16, 1], vec![PMASK + 32, PSRC + 64, PDST + 0x400, 39, 16, 16, 7]]
}
fn pb_setup(c: &mut Cpu, m: &mut M) {
    place(m, &heirloom_cpu_ppc::NAT_PALBLIT);
    c.gpr[26] = PAL;
    for i in 0..512u32 { m.mem.write_bytes(PAL + 6552 + 2 * i, &((i * 40503 % 65521) as u16).to_be_bytes()).unwrap(); }
    m.mem.write_bytes(PSRC, &pat(0x800, 3)).unwrap();
    m.mem.write_bytes(PMASK, &pat(0x800, 4)).unwrap();
    table(m, PB_ROW, &pb_rows());
}
fn pb_program() -> Vec<u32> {
    let mut load: Vec<u32> = [4, 5, 10, 11, 0, 3].iter().enumerate().map(|(k, &r)| lwz(r, 30, 4 * k as u32)).collect();
    load.extend([lwz(28, 30, 24), stw(28, 26, 8604)]);
    let r = Regs { cnt: 29, tbl: 30, tmp: 28, acc: [14, 15, 16, 17, 18, 19] };
    driver(pb_rows().len() as u32, PB_ROW, &load, XR, &[], &[6, 7, 8, 9, 12, 24, 25, 27, 4, 5, 10, 11], &r, 40)
}

/// The palette blit leaves tier 0's pixels and registers for every case - a destination over its own source, the flag word and
/// the table (memory order), one on a code page through tier 1 - and its planted count mutant (44) goes RED.
#[test]
fn the_native_palette_blit_is_exact_and_its_mutant_goes_red() {
    let prog = pb_program();
    let want = check("palblit", &prog, pb_setup, 1);
    let (got, _) = run_on(Tier::Jit, 44, &prog, 1_000_000, pb_setup);
    assert!(got != want, "mutant 44 (a row's count) stayed GREEN");
}

// ---- the nibble-rotate rows (TEXCONV) ----

const TSRC: u32 = 0x7_0000;
const TC_ROW: u32 = 32;

/// Rows: src (r3), dst (r7), width (r5), rows (r6; r4 starts at 0), the extra source stride (r10). Widths 0 / 1 / 7 / 8 / 9 /
/// 16 / 17 / 33 / 100 (the 8-pixel loop and the tail), in place, the destination 2 bytes after / before the source (memory
/// order), a source stride that re-reads the row, a destination on the routine's code page (refused: tier 1).
fn tc_rows() -> Vec<Vec<u32>> {
    let o = |k: u32| 0x8_0000 + 0x400 * k;
    vec![vec![TSRC, o(0), 0, 2, 0], vec![TSRC, o(1), 1, 3, 4], vec![TSRC + 2, o(2), 7, 2, 0], vec![TSRC, o(3), 8, 1, 6],
         vec![TSRC, o(4), 9, 3, 0], vec![TSRC + 6, o(5), 16, 2, 2], vec![TSRC, o(6), 17, 1, 0], vec![TSRC, o(7), 33, 2, 10],
         vec![TSRC, o(8), 100, 1, 0], vec![o(9), o(9), 20, 2, 0], vec![o(10), o(10) + 2, 19, 2, 0], vec![o(11) + 2, o(11), 25, 2, 0],
         vec![TSRC, o(12), 12, 3, (-24i32) as u32], vec![TSRC, XR + 0x800, 9, 2, 0]]
}
fn tc_setup(_: &mut Cpu, m: &mut M) {
    place(m, &heirloom_cpu_ppc::NAT_TEXCONV);
    m.mem.write_bytes(TSRC, &pat(0x1000, 5)).unwrap();
    for k in 9..12 { m.mem.write_bytes(0x8_0000 + 0x400 * k, &pat(0x100, k)).unwrap(); }
    table(m, TC_ROW, &tc_rows());
}
fn tc_program() -> Vec<u32> {
    let mut load: Vec<u32> = [3, 7, 5, 6, 10].iter().enumerate().map(|(k, &r)| lwz(r, 30, 4 * k as u32)).collect();
    load.push(li(4, 0));
    let r = Regs { cnt: 29, tbl: 30, tmp: 28, acc: [14, 15, 16, 17, 18, 19] };
    driver(tc_rows().len() as u32, TC_ROW, &load, XR, &[], &[0, 3, 4, 7, 8, 9], &r, 40)
}

/// The nibble-rotate rows leave tier 0's pixels and registers for every width (an empty row leaves r0 r9 CTR as they were),
/// in place and overlapping either way, and the planted count mutant (45) goes RED.
#[test]
fn the_native_nibble_rotate_rows_are_exact_and_their_mutant_goes_red() {
    let prog = tc_program();
    let want = check("texconv", &prog, tc_setup, 1);
    let (got, _) = run_on(Tier::Jit, 45, &prog, 1_000_000, tc_setup);
    assert!(got != want, "mutant 45 (the 8-pixel loop's count) stayed GREEN");
}

/// With a routine's knob set it is not recognised: the same program runs block by block (nothing translated) and still ends in
/// tier 0's state.
#[test]
fn a_native_routines_knob_turns_it_off() {
    let prog = tc_program();
    let (want, _) = run_on(Tier::Interp, 0, &prog, 1_000_000, tc_setup);
    let (mut c, mut m) = load(&prog, tc_setup);
    let mut e = exec(Tier::Jit, 0);
    e.set_jit_knobs(heirloom_cpu_abi::ppc::KNOB_NO_NAT_TEXCONV);
    let got = finish(&mut e, &mut c, &mut m, 1_000_000);
    assert!(got == want, "knob off: differs from tier 0");
    assert_eq!(e.jit_stats().translated, 0, "knob off: {:?}", e.jit_stats());
}

// ---- the bitset scan (BITSCAN) ----

const BSET: u32 = 0x7_0000; // the bitset (r3 = [r24 + 152])
const BOBJ: u32 = 0x6_0000; // r24
const B_ROW: u32 = 16;

/// The scan's words at XR (entered by a branch, LR kept in r12), the bit loop's callee (`bl` from word 10, 0x9C0 on: a
/// `blr`), `blr` after the loop. Rows: the first
/// word index (r27 = 4 i, r25 = i, r26 = 32 i) and the end (r28). Cases: all zero; a set word first; set words after runs of
/// zeros; one word; the end in the middle of the zeros.
fn bs_rows() -> Vec<Vec<u32>> { vec![vec![0, 40], vec![40, 70], vec![0, 200], vec![120, 121], vec![70, 90]] }
fn bs_setup(c: &mut Cpu, m: &mut M) {
    // `mflr r12; b XR` before it (the bit loop's call takes LR), `mtlr r12; blr` after it
    m.mem.write_u32(XR - 8, 0x7D88_02A6).unwrap();
    m.mem.write_u32(XR - 4, 0x4800_0004).unwrap();
    place(m, &heirloom_cpu_ppc::NAT_BITSCAN);
    m.mem.write_u32(XR + 4 * 21, 0x7D88_03A6).unwrap();
    m.mem.write_u32(XR + 4 * 22, 0x4E80_0020).unwrap();
    m.mem.write_u32(XR + 40 + 0x9C0, 0x4E80_0020).unwrap(); // the per-bit callee: blr
    c.gpr[24] = BOBJ;
    m.mem.write_u32(BOBJ + 152, BSET).unwrap();
    for (i, v) in [(40u32, 1u32), (41, 0x8000_0005), (77, 0x10), (150, 3), (199, 0xFFFF_FFFF)] { m.mem.write_u32(BSET + 4 * i, v).unwrap(); }
    table(m, B_ROW, &bs_rows());
}
fn bs_program() -> Vec<u32> {
    // r27 = 4 i, r25 = i, r26 = 32 i, r28 = end; r31 = the word the bit loop stores back (here: the word itself, so the set bits
    // stay set and every round sees them)
    let load = vec![lwz(25, 23, 0), lwz(28, 23, 4), rlwinm(27, 25, 2, 0, 29, false), rlwinm(26, 25, 5, 0, 26, false), li(31, 0)];
    let r = Regs { cnt: 22, tbl: 23, tmp: 21, acc: [14, 15, 16, 17, 18, 19] };
    driver(bs_rows().len() as u32, B_ROW, &load, XR - 8, &[], &[3, 25, 26, 27, 29, 30], &r, 40)
}

/// The bitset scan leaves tier 0's state over runs of zero words, at a set word (handed to the bit loop) and at the end, and
/// its planted count mutant (46) goes RED.
#[test]
fn the_native_bitset_scan_is_exact_and_its_mutant_goes_red() {
    let prog = bs_program();
    let want = check("bitscan", &prog, bs_setup, 1);
    let (got, _) = run_on(Tier::Jit, 46, &prog, 1_000_000, bs_setup);
    assert!(got != want, "mutant 46 (a set word's count) stayed GREEN");
}

// ---- the alpha-mask rows (ALPHASET, ALPHAMASK: h86 heavy) ----

const ABUF: u32 = 0x7_0000;
const A_ROW: u32 = 16;
fn mfctr(rt: u32) -> u32 { mfspr(rt, 9) }
fn mtctr(rs: u32) -> u32 { mtspr(9, rs) }

/// Rows: the destination (r21), the trips (CTR). Cases: 1, 7 and 40 trips; one ending in a word whose low half is zero and
/// high half not (the two srawi carries differ); unaligned; overlapping the row before (in
/// place, rows re-read what the last rewrote); on the routine's code page (refused: tier 1). The buffer holds every half kind
/// (low 15 bits zero or not, top bit set or not) through `pat` and four fixed words.
fn as_rows() -> Vec<Vec<u32>> {
    vec![vec![ABUF, 1], vec![ABUF + 0x40, 1], vec![ABUF + 0x101, 7], vec![ABUF + 0x200, 40], vec![ABUF + 0x208, 3], vec![XR + 0x800, 2]]
}
fn as_setup(c: &mut Cpu, m: &mut M) {
    place(m, &heirloom_cpu_ppc::NAT_ALPHASET);
    (c.gpr[5], c.gpr[12], c.gpr[6]) = (0x7FFF_7FFF, 0x8000, 0x8000_0000);
    m.mem.write_bytes(ABUF, &pat(0x800, 7)).unwrap();
    for (i, w) in [0u32, 0x8000_8000, 0x0001_0000, 0x7FFF_0000].iter().enumerate() { m.mem.write_u32(ABUF + 0x40 + 4 * i as u32, *w).unwrap(); }
    table(m, A_ROW, &as_rows());
}
fn as_program() -> Vec<u32> {
    let load = vec![lwz(21, 23, 0), lwz(24, 23, 4), mtctr(24)];
    let r = Regs { cnt: 22, tbl: 23, tmp: 24, acc: [14, 15, 16, 17, 18, 19] };
    driver(as_rows().len() as u32, A_ROW, &load, XR, &[mfctr(20)], &[0, 7, 11, 29, 21, 20], &r, 40)
}

/// The unmasked alpha-set rows leave tier 0's words and registers (XER[CA] of the last word's second srawi), in place and
/// overlapping, through tier 1 on a code page; mutant 49 (the first srawi's carry) goes RED.
#[test]
fn the_native_alpha_set_rows_are_exact_and_their_mutant_goes_red() {
    let prog = as_program();
    let want = check("alphaset", &prog, as_setup, 1);
    let (got, _) = run_on(Tier::Jit, 49, &prog, 1_000_000, as_setup);
    assert!(got != want, "mutant 49 (XER[CA] from the first srawi) stayed GREEN");
}

const MSRC: u32 = 0x7_8000;
/// Rows: the destination (r4), the mask words (r5), the trips (CTR), the mask's low half (r11; r12 = it << 16). Cases: 1, 3
/// and 30 trips; a mask with bit 15 (r12's bit 31: a srawi of 0x80000000 carries nothing); the mask 4 bytes after the
/// destination and on it (memory order); on the code page (refused).
fn am_rows() -> Vec<Vec<u32>> {
    vec![vec![ABUF, MSRC, 1, 0x7FFF], vec![ABUF + 0x40, MSRC + 8, 3, 0x8001], vec![ABUF + 0x100, MSRC + 0x20, 30, 0xFFFF],
         vec![ABUF + 0x300, ABUF + 0x304, 5, 0x00F0], vec![ABUF + 0x400, ABUF + 0x400, 4, 0x8000], vec![XR + 0x800, MSRC, 2, 0x7FFF]]
}
fn am_setup(c: &mut Cpu, m: &mut M) {
    place(m, &heirloom_cpu_ppc::NAT_ALPHAMASK);
    (c.gpr[0], c.gpr[24]) = (0x8000, 0x8000_0000);
    m.mem.write_bytes(ABUF, &pat(0x800, 8)).unwrap();
    m.mem.write_bytes(MSRC, &pat(0x400, 9)).unwrap();
    for (i, w) in [0u32, 0x8000_8000, 0xFFFF_0000, 0x0000_FFFF, 0x8000_0001].iter().enumerate() { m.mem.write_u32(MSRC + 8 + 4 * i as u32, *w).unwrap(); }
    table(m, A_ROW, &am_rows());
}
fn am_program() -> Vec<u32> {
    let load = vec![lwz(4, 7, 0), lwz(5, 7, 4), lwz(8, 7, 8), mtctr(8), lwz(11, 7, 12), rlwinm(12, 11, 16, 0, 15, false)];
    let r = Regs { cnt: 6, tbl: 7, tmp: 8, acc: [9, 10, 13, 14, 15, 16] };
    driver(am_rows().len() as u32, A_ROW, &load, XR, &[mfctr(17)], &[21, 22, 23, 25, 26, 27, 28, 30, 4, 5, 17], &r, 40)
}

/// The masked alpha rows leave tier 0's words and registers for every mask kind, with the mask on or just after the
/// destination (word by word in the original's order), through tier 1 on a code page.
#[test]
fn the_native_alpha_mask_rows_are_exact() {
    check("alphamask", &am_program(), am_setup, 1);
}

// ---- the collision cell scan (GRIDSCAN: h86 heavy) ----

const GGRID: u32 = 0x7_0000; // the 16-bit flag grid (r21 / r16 / r30 + r21 + 2)
const GREC: u32 = 0x8_0000; // the 28-byte cell records (r19; the halfword at + 12)
const GOBJ: u32 = 0x6_0000; // r17: width at + 164, height at + 168, the records at + 212
const G_ROW: u32 = 32;

/// Rows: the grid pointer (r21), the next row (r16), the records (r19), the first column (r25), the end (r23), the row stride in
/// bytes (r30), the mask (r31). Cases: a whole row; one cell; the mask 0 (every cell's first test fails); a mask with bit 15;
/// records whose bits mismatch (the update path, with its calls); an end far past RAM (the row is not validated at once: the
/// load-by-load path), which the update callee cuts short (r23 >= 0x1000: r23 = 0) at the row's first mismatch.
fn gs_rows() -> Vec<Vec<u32>> {
    vec![vec![GGRID, GGRID + 0x100, GREC, 0, 60, 0x100, 0x00FF], vec![GGRID + 0x20, GGRID + 0x120, GREC + 0x400, 3, 4, 0x100, 0x0F0F],
         vec![GGRID + 0x40, GGRID + 0x140, GREC + 0x800, 0, 20, 0x100, 0], vec![GGRID + 0x60, GGRID + 0x160, GREC + 0xC00, 2, 40, 0x100, 0x8001],
         vec![GGRID + 0x200, GGRID + 0x300, GREC + 0x1000, 0, 30, 0x100, 0xFFFF], vec![GGRID + 0x400, GGRID + 0x500, GREC + 0x2000, 5, 25, 0x80, 0x0001],
         vec![GGRID + 0x600, GGRID + 0x700, GREC + 0x2400, 1, 0x8000, 0x100, 0xFFFF]]
}
fn gs_setup(c: &mut Cpu, m: &mut M) {
    // `mflr r12; b XR` before it (the update path's calls take LR), `mtlr r12; blr` after it; the two callees: `li r3,1; blr`
    // (FUN_108ece40: the cell may change) and `cmplwi r23,0x1000; blt +8; li r23,0; blr` (FUN_108ecb50: the update; it ends a
    // row whose end is past RAM)
    m.mem.write_u32(XR - 8, 0x7D88_02A6).unwrap();
    m.mem.write_u32(XR - 4, 0x4800_0004).unwrap();
    place(m, &heirloom_cpu_ppc::NAT_GRIDSCAN);
    m.mem.write_u32(XR + 4 * 67, 0x7D88_03A6).unwrap();
    m.mem.write_u32(XR + 4 * 68, 0x4E80_0020).unwrap();
    m.mem.write_u32(XR + 4 * 52 + 0x358, li(3, 1)).unwrap();
    m.mem.write_u32(XR + 4 * 52 + 0x35C, 0x4E80_0020).unwrap();
    for (k, w) in [0x2817_1000u32, 0x4180_0008, 0x3AE0_0000, 0x4E80_0020].iter().enumerate() { m.mem.write_u32(XR + 4 * 60 + 0x48 + 4 * k as u32, *w).unwrap(); }
    c.gpr[17] = GOBJ;
    for (o, v) in [(164u32, 40u32), (168, 30), (212, GREC)] { m.mem.write_u32(GOBJ + o, v).unwrap(); }
    (c.gpr[27], c.gpr[26]) = (5, 0);
    // flags: mostly set (all four tests pass), some zero; records: bit 2 equal to the cell's u, except every 7th
    let grid = pat(0x1000, 11);
    m.mem.write_bytes(GGRID, &grid).unwrap();
    for k in 0..0x200u32 {
        let off = GGRID + 2 * k;
        let v = if k % 5 == 3 { 0 } else { 0xFFFF };
        m.mem.write_bytes(off, &(v as u16 | (grid[k as usize] as u16)).to_be_bytes()).unwrap();
    }
    m.mem.write_bytes(GREC, &pat(0x3000, 12)).unwrap();
    for k in 0..0x180u32 { let at = GREC + 28 * k + 12; let h = if k % 7 == 0 { 0x0004 } else { 0x0000 }; m.mem.write_bytes(at, &(h as u16).to_be_bytes()).unwrap(); }
    table(m, G_ROW, &gs_rows());
}
fn gs_program() -> Vec<u32> {
    let load: Vec<u32> = [21, 16, 19, 25, 23, 30, 31].iter().enumerate().map(|(k, &r)| lwz(r, 20, 4 * k as u32)).collect();
    let r = Regs { cnt: 22, tbl: 20, tmp: 8, acc: [9, 10, 11, 13, 14, 15] };
    driver(gs_rows().len() as u32, G_ROW, &load, XR - 8, &[], &[0, 3, 4, 18, 21, 16, 19, 25, 26], &r, 30)
}

/// The cell scan leaves tier 0's registers and CR0 over matching rows, leaves at the update path where a record's bit
/// differs (the original runs it, with its calls), handles a zero mask and a mask with bit 15, and its planted count
/// mutant (50) goes RED.
#[test]
fn the_native_grid_scan_is_exact_and_its_mutant_goes_red() {
    let prog = gs_program();
    let want = check("gridscan", &prog, gs_setup, 1);
    let (got, _) = run_on(Tier::Jit, 50, &prog, 1_000_000, gs_setup);
    assert!(got != want, "mutant 50 (a failed flags test's count) stayed GREEN");
}

// ---- the terrain tile expansion (TILEEXPAND) ----

const TSTK: u32 = 0x7_0000; // r1: the tile's column / row at + 56 / + 60, the 5 x 8 averages at + 120
const TOBJ: u32 = 0x6_0000; // r28: the map's origin at + 580 / + 584, its base at + 604
const TSTEP: u32 = 0x6_1000; // r27: the tile step
const TDST: u32 = 0x8_0000;
const T_ROW: u32 = 32;

/// Rows: the row stride (r31), the tile step ([r27]), the column and row ([r1 + 56 / 60]), the base ([r28 + 604]). Cases: the
/// game's shape (step 4); a wide stride; step 2 (trips overlap the next's columns: later stores win, in order); a destination
/// ON the averages (each trip reads what the last wrote); on the routine's code page (refused: tier 1 runs the rows).
fn te_rows() -> Vec<Vec<u32>> {
    vec![vec![64, 4, 1, 1, TDST], vec![300, 4, 2, 0, TDST + 0x2000], vec![64, 2, 0, 3, TDST + 0x8000], vec![16, 4, 0, 0, TSTK + 116],
         vec![64, 4, 0, 0, XR + 0x400]]
}
fn te_setup(c: &mut Cpu, m: &mut M) {
    place(m, &heirloom_cpu_ppc::NAT_TILEEXPAND);
    (c.gpr[1], c.gpr[27], c.gpr[28]) = (TSTK, TSTEP, TOBJ);
    m.mem.write_bytes(TSTK + 120, &pat(64, 15)).unwrap();
    for (o, v) in [(580u32, 0u32), (584, 0)] { m.mem.write_u32(TOBJ + o, v).unwrap(); }
    table(m, T_ROW, &te_rows());
}
fn te_program() -> Vec<u32> {
    let load = vec![lwz(31, 21, 0), lwz(22, 21, 4), stw(22, 27, 0), lwz(22, 21, 8), stw(22, 1, 56), lwz(22, 21, 12), stw(22, 1, 60),
                    lwz(22, 21, 16), stw(22, 28, 604)];
    let r = Regs { cnt: 20, tbl: 21, tmp: 22, acc: [23, 24, 25, 26, 29, 30] };
    driver(te_rows().len() as u32, T_ROW, &load, XR, &[mfctr(14)], &[0, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 15, 16, 17, 18, 19, 14], &r, 30)
}

/// The tile expansion leaves tier 0's bytes and registers (XER[CA] of the last srawi, CR0, CTR) for every case, stores over its
/// own averages and overlapping trips included, through tier 1 on a code page; mutant 51 (a row's count) goes RED. (Every
/// operand of the last srawi is a non-negative 8.8 value - bytes and their weighted means - so its carry is always 0.)
#[test]
fn the_native_tile_expansion_is_exact_and_its_mutant_goes_red() {
    let prog = te_program();
    let want = check("tileexpand", &prog, te_setup, 1);
    let (got, _) = run_on(Tier::Jit, 51, &prog, 1_000_000, te_setup);
    assert!(got != want, "mutant 51 (a row's count) stayed GREEN");
}
