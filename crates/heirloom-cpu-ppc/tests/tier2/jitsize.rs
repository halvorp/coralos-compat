//! Tier 2's h86 jitsize code-size changes against tier 0, each with the planted mutant that must make it RED: the store map's
//! entry covering the next page, SO-free compares, load groups, FPR facts, store groups (tier2.rs's harness).
use super::*;

/// An UNALIGNED store that starts on a page no code is decoded from and ends on the code page at T2: each iteration flips
/// the first word there between `addi r5,r5,1` and `addis r5,r5,1` through its upper halfword (the word's other half is the
/// store's first two bytes, on the page before). The store map's entry for the first page must cover the next (jit_smap).
const T2: u32 = CODE + 0x2000;
fn straddle_store_program() -> Vec<u32> {
    vec![
        d(14, 5, 0, 0), d(14, 31, 0, 50),
        d(15, 7, 0, 0x1234), d(24, 7, 7, 0x3CA5),
        d(15, 8, 0, T2 >> 16), d(24, 8, 8, T2 & 0xFFFF),
        d(26, 7, 7, 0x0400), d(36, 7, 8, 0xFFFE), 18 << 26 | ((T2 - (CODE + 32)) & 0x03FF_FFFC) | 1, // xori; stw r7,-2(r8); bl T2
        d(13, 31, 31, 0xFFFF), bc(4, 2, -16), // addic.; bne xori
        d(15, 24, 0, EXIT >> 16), d(24, 24, 24, EXIT & 0xFFFF), mtspr(8, 24),
    ]
}
fn straddle_store_setup(_: &mut Cpu, m: &mut M) { m.mem.write_u32(T2, 0x3CA5_0001).unwrap(); m.mem.write_u32(T2 + 4, 0x4E80_0020).unwrap(); }

#[test]
fn an_unaligned_store_ending_on_a_code_page_runs_the_new_instruction() {
    let prog = straddle_store_program();
    let (want, _) = run_on(Tier::Interp, 0, &prog, 1_000_000, straddle_store_setup);
    assert_eq!(want.gpr[5], 25 + (25 << 16), "tier 0: addi and addis in turn, 50 iterations");
    for slice in [1_000_000, 1_000, 13] {
        let (got, e) = run_on(Tier::Jit, 0, &prog, slice, straddle_store_setup);
        assert!(got == want, "slice {slice}: r5 {} {:?}", got.gpr[5], e.jit_stats());
    }
    let (got, e) = run_on(Tier::JitLockstep, 0, &prog, 13, straddle_store_setup);
    assert!(got == want && e.divergence().is_none(), "{:?}", e.divergence());
    // negative control: an entry that covers its own page only lets the stale word run
    let (got, e) = run_on(Tier::Jit, 31, &prog, 1_000_000, straddle_store_setup);
    assert!(got != want, "mutant 31 (one-page store map) stayed GREEN: r5 {:#x} {:?}", got.gpr[5], e.jit_stats());
}

/// XER[SO] set in the middle of a hot loop (addo overflows on the 16th trip; OE forms run on tier 1's handler), a compare
/// into cr1 after it every trip, its CR read back (mfcr) into r8: SO-free code must leave when SO is set (jit.rs so_aware).
fn so_program() -> Vec<u32> {
    vec![
        d(15, 5, 0, 0x7FFF), d(24, 5, 5, 0xFFF0), d(14, 6, 0, 1), d(14, 31, 0, 64),
        x(5, 5, 6, 778, false), d(11, 4, 5, 0), x(7, 0, 0, 19, false), x(8, 8, 7, 316, false), // addo; cmpwi cr1; mfcr; xor
        d(13, 31, 31, 0xFFFF), bc(4, 2, -20), // addic.; bne addo
        d(15, 24, 0, EXIT >> 16), d(24, 24, 24, EXIT & 0xFFFF), mtspr(8, 24),
    ]
}

#[test]
fn xer_so_set_under_so_free_code_makes_it_leave_and_compares_see_so() {
    let prog = so_program();
    for (name, setup) in [("SO clear at the start", (|_: &mut Cpu, _: &mut M| {}) as Setup), ("SO set at the start", |c, _| c.xer |= 1 << 31)] {
        let (want, _) = run_on(Tier::Interp, 0, &prog, 1_000_000, setup);
        assert!(want.xer & 1 << 31 != 0 && want.cr & 0x0100_0000 != 0, "{name}: tier 0 ends with SO in XER and cr1");
        for slice in [1_000_000, 1_000, 13] {
            let (got, e) = run_on(Tier::Jit, 0, &prog, slice, setup);
            assert!(got == want, "{name}, slice {slice}: r8 {:#x} want {:#x} {:?}", got.gpr[8], want.gpr[8], e.jit_stats());
        }
        let (got, e) = run_on(Tier::JitLockstep, 0, &prog, 13, setup);
        assert!(got == want && e.divergence().is_none(), "{name}: {:?}", e.divergence());
    }
    // negative control: a handler that sets SO without making SO-free code leave
    let (want, _) = run_on(Tier::Interp, 0, &prog, 1_000_000, |_, _| {});
    let (got, _) = run_on(Tier::Jit, 33, &prog, 1_000_000, |_, _| {});
    assert!(got != want, "mutant 33 (SO set, SO-free code stays) stayed GREEN");
}

/// A subroutine of two loads off r3 (one load group: jit_lgroup) called 20 times on a buffer, then once with r3 eight bytes
/// before the end of RAM: its second load ends two bytes past RAM - tier 0's trap there, with the first load done.
fn load_group_program() -> Vec<u32> {
    let bl = |from: u32, to: u32| 18 << 26 | ((4 * (to - from)) & 0x03FF_FFFC) | 1;
    vec![
        d(14, 31, 0, 20), d(15, 3, 0, BUF >> 16), d(24, 3, 3, BUF & 0xFFFF), bl(3, 13), d(13, 31, 31, 0xFFFF), bc(4, 2, -8),
        d(15, 3, 0, (RAM - 8) >> 16), d(24, 3, 3, (RAM - 8) & 0xFFFF), bl(8, 13),
        d(15, 24, 0, EXIT >> 16), d(24, 24, 24, EXIT & 0xFFFF), mtspr(8, 24), 0x4E80_0020,
        d(32, 5, 3, 0), d(32, 6, 3, 6), 0x4E80_0020, // F: lwz r5,0(r3); lwz r6,6(r3); blr
    ]
}

#[test]
fn a_load_group_whose_span_leaves_ram_runs_its_head_and_traps_at_the_member_like_tier_0() {
    let prog = load_group_program();
    let setup = |_: &mut Cpu, m: &mut M| { m.mem.write_u32(RAM - 8, 0x1234_5678).unwrap(); };
    let (want, _) = run_on(Tier::Interp, 0, &prog, 1_000_000, setup);
    assert!(want.last.trap.is_some() && want.gpr[5] == 0x1234_5678 && want.pc == CODE + 4 * 14, "tier 0 traps at the second load: {:?}", want.last);
    for slice in [1_000_000, 1_000, 13] {
        let (got, e) = run_on(Tier::Jit, 0, &prog, slice, setup);
        assert!(got == want, "slice {slice}: {:?} {:?}", got.last, e.jit_stats());
    }
    let (got, e) = run_on(Tier::JitLockstep, 0, &prog, 13, setup);
    assert!(got == want && e.divergence().is_none(), "{:?}", e.divergence());
    // negative control: a span short of the last member's bytes lets the member read past RAM and run on
    let (got, _) = run_on(Tier::Jit, 35, &prog, 1_000_000, setup);
    assert!(got != want, "mutant 35 (a load group's short span) stayed GREEN");
}

/// fadds of a signalling NaN with FPSCR[VE] set leaves frD as it was (tier 0, an enabled invalid operation) - a double no
/// single equals - and stfs then stores the PowerPC's bit selection of it (truncation, not cvtsd2ss's rounding). The fadds'
/// slow path must not come back to code that took its result for a single (jit_ffacts).
fn facts_program() -> Vec<u32> {
    vec![
        d(14, 31, 0, 20), d(15, 20, 0, BUF >> 16), d(24, 20, 20, BUF & 0xFFFF),
        fa(59, 3, 1, 2, 0, 21, false), d(52, 3, 20, 0), d(13, 31, 31, 0xFFFF), bc(4, 2, -12), // fadds f3,f1,f2; stfs f3,0(r20)
        d(15, 24, 0, EXIT >> 16), d(24, 24, 24, EXIT & 0xFFFF), mtspr(8, 24),
    ]
}
fn facts_setup(c: &mut Cpu, _: &mut M) {
    c.fpr[1] = 0x7FF0_0000_0000_0001; // SNaN
    c.set_fpr_f64(2, 1.0);
    c.fpr[3] = 0x3FF0_0000_1FFF_FFFF; // its single rounding 0x3F800001, its bit selection 0x3F800000
    c.fpscr = 0x80; // VE
}

#[test]
fn an_fp_slow_path_that_leaves_frd_unchanged_is_not_followed_by_code_trusting_frd() {
    let prog = facts_program();
    let (want, _) = run_on(Tier::Interp, 0, &prog, 1_000_000, facts_setup);
    assert_eq!(&want.ram[BUF as usize..BUF as usize + 4], &[0x3F, 0x80, 0, 0], "tier 0 stores the bit selection");
    for slice in [1_000_000, 1_000, 13] {
        let (got, e) = run_on(Tier::Jit, 0, &prog, slice, facts_setup);
        assert!(got == want, "slice {slice}: {:x?} {:?}", &got.ram[BUF as usize..BUF as usize + 4], e.jit_stats());
    }
    let (got, e) = run_on(Tier::JitLockstep, 0, &prog, 13, facts_setup);
    assert!(got == want && e.divergence().is_none(), "{:?}", e.divergence());
    let (got, _) = run_on(Tier::Jit, 36, &prog, 1_000_000, facts_setup);
    assert!(got != want, "mutant 36 (fact-making slow paths come back) stayed GREEN");
}

/// Two stores off r8 in one block: one on a free page, one TWO pages above it on code (T3, called every trip): more than a page
/// apart, so they form no store group (jit_lgroup) and the second takes its own store-map test.
const T3: u32 = 0xB000;
fn store_group_program() -> Vec<u32> {
    vec![
        d(14, 5, 0, 0), d(14, 31, 0, 50), d(15, 7, 0, ADDI_1 >> 16), d(24, 7, 7, ADDI_1 & 0xFFFF), d(14, 8, 0, 0), d(24, 8, 8, T3 - 0x2000),
        d(26, 7, 7, 3), d(36, 9, 8, 0), d(36, 7, 8, 0x2000), 18 << 26 | ((T3 - (CODE + 36)) & 0x03FF_FFFC) | 1, // xori; stw r9,0(r8); stw r7,0x2000(r8); bl T3
        d(13, 31, 31, 0xFFFF), bc(4, 2, -20),
        d(15, 24, 0, EXIT >> 16), d(24, 24, 24, EXIT & 0xFFFF), mtspr(8, 24),
    ]
}
fn store_group_setup(_: &mut Cpu, m: &mut M) { m.mem.write_u32(T3, NOP).unwrap(); m.mem.write_u32(T3 + 4, 0x4E80_0020).unwrap(); }

#[test]
fn stores_more_than_a_page_apart_take_their_own_store_map_tests() {
    let prog = store_group_program();
    let (want, _) = run_on(Tier::Interp, 0, &prog, 1_000_000, store_group_setup);
    for slice in [1_000_000, 1_000, 13] {
        let (got, e) = run_on(Tier::Jit, 0, &prog, slice, store_group_setup);
        assert!(got == want, "slice {slice}: r5 {} want {} {:?}", got.gpr[5], want.gpr[5], e.jit_stats());
    }
    let (got, e) = run_on(Tier::JitLockstep, 0, &prog, 13, store_group_setup);
    assert!(got == want && e.divergence().is_none(), "{:?}", e.divergence());
    let (got, _) = run_on(Tier::Jit, 37, &prog, 1_000_000, store_group_setup);
    assert!(got != want, "mutant 37 (a store group over more than a page) stayed GREEN");
    // a store GROUP (0x800 apart) whose second store lands on the code page right above its head's page: the head's map entry
    // covers that page too, so the head's one test fails, and the block leaves after the head (jit_lgroup)
    let mut near = prog.clone();
    (near[5], near[8]) = (d(24, 8, 8, T3 - 0x800), d(36, 7, 8, 0x800));
    let (want, _) = run_on(Tier::Interp, 0, &near, 1_000_000, store_group_setup);
    for slice in [1_000_000, 13] {
        let (got, e) = run_on(Tier::Jit, 0, &near, slice, store_group_setup);
        assert!(got == want, "group, slice {slice}: r5 {} want {} {:?}", got.gpr[5], want.gpr[5], e.jit_stats());
    }
    let (got, _) = run_on(Tier::Jit, 31, &near, 1_000_000, store_group_setup);
    assert!(got != want, "mutant 31 (one-page map entries) under a store group stayed GREEN");
}
