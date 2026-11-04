//! Tier 2's h86 fnreg region register binding against tier 0: a hot loop whose region uses ten GPRs that are not global pins
//! and none of the pins r0 r1 r3 r4 r6, so its binding re-binds the pins' host registers to region locals (jit_regs
//! pick_locals) - with a helper call (divw: tier 1's handler) and a side exit inside - in every slicing, under JitLockstep and
//! with the A/B knob; the planted mutant that leaves a displaced pin's register holding a local at an exit goes RED.
use super::*;

fn add(d: u32, a: u32, b: u32) -> u32 { x(d, a, b, 266, false) }
fn xor(d: u32, a: u32, b: u32) -> u32 { 31 << 26 | a << 21 | d << 16 | b << 11 | 316 << 1 }
fn divw(d: u32, a: u32, b: u32) -> u32 { x(d, a, b, 491, false) }
fn subf(d: u32, a: u32, b: u32) -> u32 { x(d, a, b, 40, false) }

/// r20..r29 live in the loop (each used >= 4 times a trip), r21 grows by one on trips where r20 < 1000 (the bge is a side
/// exit of the loop's first node), r29 counts 300 trips down; the pins r0 r1 r3 r4 r6 are read only after the loop.
fn displace_program() -> Vec<u32> {
    let mut p = vec![d(14, 20, 0, 0), d(14, 21, 0, 1), d(14, 22, 0, 2), d(14, 23, 0, 3), d(14, 24, 0, 5), d(14, 25, 0, 7),
                     d(14, 26, 0, 11), d(14, 27, 0, 13), d(14, 28, 0, 0), d(14, 29, 0, 300)];
    let top = p.len();
    for _ in 0..2 {
        p.extend([add(20, 20, 21), add(22, 22, 20), xor(23, 23, 22), add(24, 24, 23), divw(25, 24, 26), add(26, 26, 21),
                  subf(27, 25, 27), add(28, 28, 27), xor(22, 22, 28), add(23, 23, 26)]);
    }
    p.push(d(11, 0, 20, 1000)); // cmpwi r20,1000
    p.push(bc(4, 0, 8)); // bge +8 (skip the addi)
    p.push(d(14, 21, 21, 1)); // addi r21,r21,1
    p.push(d(13, 29, 29, 0xFFFF)); // addic. r29,r29,-1
    let b = p.len();
    p.push(bc(4, 2, (top as i32 - b as i32) * 4)); // bne top
    // the pins after the loop: whatever they hold must be what they held before it
    p.extend([add(20, 20, 0), add(20, 20, 1), add(20, 20, 3), add(20, 20, 4), add(20, 20, 6)]);
    p.extend([d(15, 24, 0, EXIT >> 16), d(24, 24, 24, EXIT & 0xFFFF), mtspr(8, 24)]);
    p
}
fn displace_setup(c: &mut Cpu, _: &mut M) { c.gpr[0] = 0x1111; c.gpr[3] = 0x3333; c.gpr[4] = 0x4444; c.gpr[6] = 0x6666; }

#[test]
fn a_region_rebinding_the_pins_ends_in_tier_0s_state_and_its_mutant_goes_red() {
    let prog = displace_program();
    let (want, _) = run_on(Tier::Interp, 0, &prog, 1_000_000, displace_setup);
    let mut displaced = 0;
    for slice in [1_000_000, 1_000, 61, 13, 3] {
        let (got, e) = run_region(Tier::Jit, 0, &prog, slice, displace_setup, Some(REGION_TEST));
        assert!(got == want, "slice {slice}: gpr {:x?} vs {:x?} {:?}", got.gpr, want.gpr, e.jit_stats());
        displaced += e.jit_stats().region_displaced;
    }
    assert!(displaced > 0, "the binding displaced no pin");
    let (got, e) = run_region(Tier::JitLockstep, 0, &prog, 50, displace_setup, Some(REGION_TEST));
    assert!(got == want && e.divergence().is_none(), "lockstep: {:?}", e.divergence());
    // the A/B knob: the same state, and no pin displaced
    let (got, e) = run_knobs(heirloom_cpu_abi::ppc::KNOB_NO_DISPLACE, &prog, 1_000_000, displace_setup);
    assert!(got == want && e.jit_stats().region_displaced == 0, "{:?}", e.jit_stats());
    // negative control: an exit that leaves a local in a pin's register
    let (got, _) = run_region(Tier::Jit, 41, &prog, 1_000_000, displace_setup, Some(REGION_TEST));
    assert!(got != want, "mutant 41 (displaced pins not restored at exits) stayed GREEN");
}

// ---- the lazy XER[CA] (jit_lazyca) ----

fn srawi(a: u32, s: u32, sh: u32) -> u32 { 31 << 26 | s << 21 | a << 16 | sh << 11 | 824 << 1 }
fn addze(d: u32, a: u32) -> u32 { 31 << 26 | d << 21 | a << 16 | 202 << 1 }
fn b(rel: i32) -> u32 { 18 << 26 | (rel as u32 & 0x03FF_FFFC) }

/// A loop whose srawi leaves its carry pending into a superblock SIDE EXIT (bge, once r20 >= 100) to code that reads it
/// (addze r30), then a second srawi overwriting it before a store and an addze in the loop's tail, addic. counting down.
fn lazy_ca_program() -> Vec<u32> {
    vec![
        d(14, 29, 0, 200), d(14, 20, 0, 0xFFF9), d(14, 30, 0, 0), d(14, 22, 0, 0), d(14, 24, 0, BUF),
        srawi(21, 20, 1),       // 5 loop: CA = r20 negative and odd
        d(14, 20, 20, 3),       // 6
        d(11, 0, 20, 100),      // 7 cmpwi r20,100
        bc(4, 0, 4 * 7),        // 8 bge side (15)
        srawi(23, 20, 2),       // 9
        d(36, 23, 24, 0),       // 10 stw r23,0(r24)
        addze(22, 22),          // 11 tail
        d(13, 29, 29, 0xFFFF),  // 12 addic. r29,r29,-1
        bc(4, 2, -4 * 8),       // 13 bne loop (5)
        b(4 * 3),               // 14 b out (17)
        addze(30, 30),          // 15 side: the srawi's carry
        b(-4 * 5),              // 16 b tail (11)
        d(15, 24, 0, EXIT >> 16), d(24, 24, 24, EXIT & 0xFFFF), mtspr(8, 24),
    ]
}

#[test]
fn a_lazy_carry_is_settled_at_side_exits_helpers_and_exits_and_its_mutant_goes_red() {
    let prog = lazy_ca_program();
    let (want, _) = run_on(Tier::Interp, 0, &prog, 1_000_000, |_, _| {});
    assert!(want.gpr[30] == 0 && want.gpr[22] > 0, "tier 0: the side path sees CA = 0 (r20 >= 100 is positive), the tail counts some: r30 {} r22 {}", want.gpr[30], want.gpr[22]);
    for slice in [1_000_000, 1_000, 61, 13, 3] {
        for region in [None, Some(REGION_TEST)] {
            let (got, e) = run_region(Tier::Jit, 0, &prog, slice, |_, _| {}, region);
            assert!(got == want, "slice {slice} region {region:?}: r22 {} / {}, r30 {} / {}, xer {:#x} / {:#x} {:?}", got.gpr[22], want.gpr[22], got.gpr[30], want.gpr[30], got.xer, want.xer, e.jit_stats());
        }
    }
    let (got, e) = run_region(Tier::JitLockstep, 0, &prog, 50, |_, _| {}, Some(REGION_TEST));
    assert!(got == want && e.divergence().is_none(), "lockstep: {:?}", e.divergence());
    let (got, _) = run_knobs(heirloom_cpu_abi::ppc::KNOB_NO_LAZY_CA, &prog, 1_000_000, |_, _| {});
    assert!(got == want, "the A/B knob: tier 0's state");
    // negative control: the side exit leaves with the addic.'s older carry (1) where the srawi's (0) belongs
    let (got, _) = run_on(Tier::Jit, 42, &prog, 1_000_000, |_, _| {});
    assert!(got != want, "mutant 42 (side exits do not settle the carry) stayed GREEN");
}

// ---- the cold-tail rule (jit_cold) ----

/// A block compiles only once it ran `min` times within one window of guest instructions: with a minimum no block reaches,
/// nothing is compiled and tier 1 ends in tier 0's state; with 4 the hot loop compiles; every program under test ends in tier 0's
/// state under the rule in any slicing.
#[test]
fn the_cold_tail_rule_keeps_rarely_run_blocks_on_tier_1_and_changes_nothing_else() {
    let run = |prog: &[u32], setup: Setup, slice: u64, cold: (u64, u32)| {
        let (mut c, mut m) = load(prog, &setup);
        let mut e = exec(Tier::Jit, 0);
        e.set_jit_cold(cold.0, cold.1);
        let end = finish(&mut e, &mut c, &mut m, slice);
        (end, e)
    };
    for (name, prog, setup) in programs_under_test() {
        let (want, _) = run_on(Tier::Interp, 0, prog, 1_000_000, setup);
        for slice in [1_000_000, 1_000, 13] {
            for cold in [(10_000, 4), (1_000_000, 2), (64, 3)] {
                let (got, e) = run(prog, setup, slice, cold);
                assert!(got == want, "{name} slice {slice} cold {cold:?}: {:?}", e.jit_stats());
            }
        }
    }
    let prog = displace_program();
    let (want, _) = run_on(Tier::Interp, 0, &prog, 1_000_000, displace_setup);
    let (got, e) = run(&prog, displace_setup, 1_000_000, (u64::MAX, u32::MAX));
    assert!(got == want && e.jit_stats().compiled == 0, "no block reaches the minimum: {:?}", e.jit_stats());
    let (got, e) = run(&prog, displace_setup, 1_000_000, (u64::MAX, 4));
    assert!(got == want && e.jit_stats().compiled > 0, "the hot loop compiles: {:?}", e.jit_stats());
}

// ---- the eager invalidation (jit_inval) ----

/// With no entry tests (the default), the self-modifying programs - a store into a later block's page, the loop that rewrites the
/// second page of its own region, a native that rewrites code - end in tier 0's state in every slicing and in lockstep, because
/// the written page's code is dropped before anything is entered; the same with KNOB_ENTRY_CHECKS; and with the planted mutant
/// (the written page's code kept, a native that wrote code not leaving) stale code runs: RED.
#[test]
fn a_written_code_page_loses_its_code_before_anything_enters_it() {
    use heirloom_cpu_abi::ppc::KNOB_ENTRY_CHECKS;
    let (smc, xp) = (smc_store_program(), cross_page_program());
    let progs: [(&str, &[u32], Setup); 4] = [("smc_store", &smc, smc_store_setup), ("cross_page", &xp, cross_page_setup),
        ("bench_smc", programs::BENCH_SMC, |c, _| c.gpr[3] = 100), ("smc_native", programs::SMC_NATIVE, |c, _| c.gpr[3] = 100)];
    let mut red = false;
    for (name, prog, setup) in progs {
        let (want, _) = run_on(Tier::Interp, 0, prog, 1_000_000, setup);
        for slice in [1_000_000, 1_000, 13] {
            for region in [None, Some(REGION_TEST)] {
                let (got, e) = run_region(Tier::Jit, 0, prog, slice, setup, region);
                assert!(got == want, "{name} slice {slice} region {region:?}: {:?}", e.jit_stats());
            }
            let (got, _) = run_knobs(KNOB_ENTRY_CHECKS, prog, slice, setup);
            assert!(got == want, "{name} slice {slice}: with the entry tests too");
            let (got, _) = run_region(Tier::Jit, 43, prog, slice, setup, Some(REGION_TEST));
            red |= got != want;
        }
        let (got, e) = run_region(Tier::JitLockstep, 0, prog, 13, setup, Some(REGION_TEST));
        assert!(got == want && e.divergence().is_none(), "{name} lockstep: {:?}", e.divergence());
    }
    assert!(red, "mutant 43 (written code kept) stayed GREEN");
}
