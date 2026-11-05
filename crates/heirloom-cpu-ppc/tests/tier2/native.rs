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
