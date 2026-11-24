//! Replay and lockstep tests for Dolphin-inspired Slice 1:
//! - Region FPR register cache (`xmm8..xmm15`, dirty-tracked, SSE-domain `fabs`/`fneg`/`fnabs`/`fmr`)
//! - Inline double-precision FMA (`fmadd`/`fmsub`/`fnmadd`/`fnmsub`, CPUID FMA3, bit-exact single rounding + non-finite slow path)
//! - Backward CR-liveness dead-CR0 elimination & `Rc=1` (`andi.`/`or.`/`addic.`) + `bc` fusion (`jit_rflow::CrOpt`)
//! - Negative control: mutant 52 (`MUT_FP_LOCAL_EXIT`) must go RED.
use super::*;

fn fma(xo5: u32, d: u32, a: u32, c: u32, b: u32) -> u32 {
    63 << 26 | d << 21 | a << 16 | b << 11 | c << 6 | xo5 << 1
}
fn fp63_xo(xo10: u32, d: u32, b: u32) -> u32 {
    63 << 26 | d << 21 | b << 11 | xo10 << 1
}
fn andi_rc(a: u32, s: u32, imm: u32) -> u32 {
    28 << 26 | s << 21 | a << 16 | (imm & 0xFFFF)
}

/// Hot FP + Rc=1 loop exercising cached FPRs (read-only constants f1..f3 + dirty accumulators f4..f8),
/// all four double FMA forms, SSE sign ops (fabs/fneg/fnabs/fmr), single-precision fadds/lfs/stfs,
/// fcmpu, a dead `andi.` (CR0 overwritten before read), and a fused `andi. + beq`.
fn fp_cache_replay_program() -> Vec<u32> {
    vec![
        d(50, 1, 4, 0),            // 0: lfd f1,0(r4)
        d(50, 2, 4, 8),            // 1: lfd f2,8(r4)
        d(50, 3, 4, 16),           // 2: lfd f3,16(r4)
        fp63_xo(72, 4, 1),         // 3: fmr f4,f1
        d(14, 20, 0, 200),         // 4: li r20,200
        // loop (pc = 5):
        fma(29, 4, 4, 2, 3),       // 5: fmadd f4,f4,f2,f3
        fma(28, 5, 4, 1, 3),       // 6: fmsub f5,f4,f1,f3
        fma(31, 6, 5, 2, 1),       // 7: fnmadd f6,f5,f2,f1
        fma(30, 7, 6, 3, 2),       // 8: fnmsub f7,f6,f3,f2
        fp63_xo(264, 7, 7),        // 9: fabs f7,f7
        fp63_xo(40, 6, 7),         // 10: fneg f6,f7
        fp63_xo(136, 8, 5),        // 11: fnabs f8,f5
        fa(59, 9, 7, 2, 0, 21, false), // 12: fadds f9,f7,f2
        d(52, 9, 4, 24),           // 13: stfs f9,24(r4)
        d(48, 10, 4, 24),          // 14: lfs f10,24(r4)
        fa(63, 4, 10, 1, 0, 21, false), // 15: fadd f4,f10,f1
        fa(63, 4, 4, 0, 2, 25, false),  // 16: fmul f4,f4,f2
        andi_rc(21, 20, 0xFF),     // 17: dead andi. (CR0 overwritten at 18)
        andi_rc(22, 20, 1),        // 18: fused andi. with beq at 19
        bc(12, 2, 8),              // 19: beq +8 (skip fadd at 20)
        fa(63, 4, 4, 8, 0, 20, false), // 20: fsub f4,f4,f8 (even/odd branch)
        d(54, 4, 4, 32),           // 21: stfd f4,32(r4)
        d(13, 20, 20, 0xFFFF),     // 22: addic. r20,r20,-1
        bc(4, 2, -18 * 4),         // 23: bne loop (5)
        d(54, 6, 4, 40),           // 24: stfd f6,40(r4) (post-loop read of cached dirty FPR!)
        d(54, 7, 4, 48),           // 25: stfd f7,48(r4)
        d(15, 24, 0, EXIT >> 16),
        d(24, 24, 24, EXIT & 0xFFFF),
        mtspr(8, 24),
    ]
}

fn fp_cache_setup(c: &mut Cpu, m: &mut M) {
    c.gpr[4] = BUF;
    for (i, v) in [0.5f64, 0.75, 0.125].iter().enumerate() {
        m.mem.write_bytes(BUF + 8 * i as u32, &v.to_be_bytes()).unwrap();
    }
}

#[test]
fn fp_region_cache_fma_and_rc_fusion_replay_bit_exact_and_mutant_goes_red() {
    let prog = fp_cache_replay_program();
    let (want, _) = run_on(Tier::Interp, 0, &prog, 1_000_000, fp_cache_setup);
    assert!(want.last.halted && want.last.trap.is_none());

    let mut fp_cached_total = 0u64;
    let mut fused_total = 0u64;
    for fma_on in [true, false] {
        for slice in [1_000_000, 1_000, 61, 13, 3] {
            let (mut c, mut m) = load(&prog, fp_cache_setup);
            let mut e = exec(Tier::Jit, 0);
            e.set_jit_region_after(REGION_TEST);
            e.set_jit_fma(fma_on);
            let got = finish(&mut e, &mut c, &mut m, slice);
            assert!(
                got == want,
                "fma={fma_on} slice={slice}: fpr4 {:016x}/{:016x} fpscr {:#x}/{:#x} cr {:#x}/{:#x}",
                got.fpr[4], want.fpr[4], got.fpscr, want.fpscr, got.cr, want.cr
            );
            fp_cached_total += e.jit_stats().region_fp_locals;
            fused_total += e.jit_stats().region_fused;
        }
        let (mut c, mut m) = load(&prog, fp_cache_setup);
        let mut e = exec(Tier::JitLockstep, 0);
        e.set_jit_region_after(REGION_TEST);
        e.set_jit_fma(fma_on);
        let got = finish(&mut e, &mut c, &mut m, 37);
        assert!(got == want && e.divergence().is_none(), "fma={fma_on} lockstep: {:?}", e.divergence());
    }
    assert!(fp_cached_total > 0, "expected region FP locals to be allocated in xmm8..xmm15");
    assert!(fused_total > 0, "expected Rc=1 CR0 dead/fuse optimisations in region");

    // NEGATIVE CONTROL: mutant 52 skips storing dirty FPR locals at region exits.
    let (got, _) = run_region(Tier::Jit, 52, &prog, 1_000_000, fp_cache_setup, Some(REGION_TEST));
    assert!(got != want, "mutant 52 (MUT_FP_LOCAL_EXIT) stayed GREEN");
}

#[test]
fn double_fma_exact_single_rounding_and_non_finite_edges_match_tier_0() {
    // Tests exact single-rounding FMA tie: (1 + 2^-52)*(1 - 2^-52) - 1 = -2^-104 (double-rounding yields 0.0!),
    // plus Inf*0 (VXIMZ), NaN propagation, overflow (OX), and -0.0 sign preservation across fmadd/fmsub/fnmadd/fnmsub.
    let prog = vec![
        d(14, 20, 0, 10),           // 0: li r20,10 (hot enough to compile into JIT & region)
        d(50, 1, 4, 0),             // 1: loop: lfd f1,0(r4)
        d(50, 2, 4, 8),             // 2: lfd f2,8(r4)
        d(50, 3, 4, 16),            // 3: lfd f3,16(r4)
        fma(29, 4, 1, 2, 3),        // 4: fmadd f4,f1,f2,f3
        fma(28, 5, 1, 2, 3),        // 5: fmsub f5,f1,f2,f3
        fma(31, 6, 1, 2, 3),        // 6: fnmadd f6,f1,f2,f3
        fma(30, 7, 1, 2, 3),        // 7: fnmsub f7,f1,f2,f3
        d(13, 20, 20, 0xFFFF),      // 8: addic. r20,r20,-1
        bc(4, 2, -8 * 4),           // 9: bne loop
        d(15, 24, 0, EXIT >> 16),
        d(24, 24, 24, EXIT & 0xFFFF),
        mtspr(8, 24),
    ];
    let cases: [(&str, u64, u64, u64); 6] = [
        ("exact_single_rounding_tie", (1.0f64 + f64::EPSILON).to_bits(), (1.0f64 - f64::EPSILON).to_bits(), (-1.0f64).to_bits()),
        ("neg_zero", (-0.0f64).to_bits(), (0.0f64).to_bits(), (-0.0f64).to_bits()),
        ("inf_times_zero_vximz", f64::INFINITY.to_bits(), 0.0f64.to_bits(), 1.0f64.to_bits()),
        ("overflow_ox", f64::MAX.to_bits(), 2.0f64.to_bits(), 1.0f64.to_bits()),
        ("snan_prop", 0x7FF0_0000_0000_0001, 2.0f64.to_bits(), 3.0f64.to_bits()),
        ("subnormal", f64::MIN_POSITIVE.to_bits(), 0.5f64.to_bits(), 0.0f64.to_bits()),
    ];
    for (name, a, c_op, b) in cases {
        let setup = move |cpu: &mut Cpu, m: &mut M| {
            cpu.gpr[4] = BUF;
            m.mem.write_bytes(BUF, &a.to_be_bytes()).unwrap();
            m.mem.write_bytes(BUF + 8, &c_op.to_be_bytes()).unwrap();
            m.mem.write_bytes(BUF + 16, &b.to_be_bytes()).unwrap();
        };
        let (want, _) = run_on(Tier::Interp, 0, &prog, 1_000_000, setup);
        let (got, e) = run_region(Tier::JitLockstep, 0, &prog, 29, setup, Some(REGION_TEST));
        assert!(got == want && e.divergence().is_none(), "case {name}: {:?}", e.divergence());
        if name == "exact_single_rounding_tie" {
            assert_ne!(got.fpr[4], 0, "exact FMA must yield -2^-104, not 0.0");
        }
    }
}
