//! Tier 2 (the x86-64 JIT) against tier 0 (the reference): the same state at every exit, whatever the slicing, the traps,
//! the natives and the code modifications; JitLockstep finds no divergence; a random-program fuzz over every inline form
//! agrees; and each planted JIT mutant goes RED. Host only (the JIT needs executable memory: HostCodeMem).
#![cfg(all(target_arch = "x86_64", target_os = "linux"))]
mod common;
use common::{programs, CODE, EXIT, STACK};
use heirloom_cpu_ppc::{Cpu, Exec, Host, HostCodeMem, RunStats, Tier};
use heirloom_mem_flat32::GuestMem;

const BUF: u32 = 0x4000;
const NATIVE: u32 = 0xF100;
const ADDI_1: u32 = 0x38A5_0001; // addi r5,r5,1
const NOP: u32 = 0x6000_0000;
const RAM: u32 = 1 << 20;

struct M { mem: GuestMem, calls: Vec<u32> }
impl Host for M {
    fn mem(&mut self) -> &mut GuestMem { &mut self.mem }
    fn native(&mut self, cpu: &mut Cpu, addr: u32) -> bool {
        self.calls.push(addr);
        match addr {
            EXIT => { cpu.halt(); true }
            NATIVE if cpu.gpr[4] == 32 => self.mem.zero(cpu.gpr[3], 32).is_ok(),
            NATIVE => {
                let w = self.mem.read_u32(cpu.gpr[3]).unwrap();
                self.mem.write_u32(cpu.gpr[3], if w == NOP { ADDI_1 ^ 3 } else { w ^ 3 }).is_ok()
            }
            _ => true,
        }
    }
    fn syscall(&mut self, cpu: &mut Cpu) -> bool { cpu.gpr[3] = cpu.gpr[0] + 1; true }
}

fn load(prog: &[u32], setup: impl FnOnce(&mut Cpu, &mut M)) -> (Cpu, M) {
    let mut m = M { mem: GuestMem::new(RAM as usize), calls: vec![] };
    for (i, w) in prog.iter().enumerate() { m.mem.write_u32(CODE + 4 * i as u32, *w).unwrap(); }
    m.mem.write_u32(CODE + 4 * prog.len() as u32, 0x4E80_0020).unwrap(); // blr
    let mut c = Cpu::new(CODE, STACK);
    c.lr = EXIT;
    c.bind_native(EXIT);
    c.bind_native(NATIVE);
    c.gpr[6] = NATIVE;
    setup(&mut c, &mut m);
    (c, m)
}

#[derive(Debug, PartialEq)]
struct End { gpr: [u32; 32], fpr: [u64; 32], cr: u32, lr: u32, ctr: u32, xer: u32, fpscr: u32, pc: u32, tb: u64, ram: Vec<u8>, last: RunStats, total: u64, calls: Vec<u32> }

fn finish(exec: &mut Exec, c: &mut Cpu, m: &mut M, slice: u64) -> End {
    let mut total = 0;
    let last = loop {
        let st = exec.resume(m, c, slice);
        total += st.insns;
        if st.halted || st.trap.is_some() || total >= 3_000_000 || exec.divergence().is_some() { break st; }
    };
    let ram = m.mem.bytes(m.mem.span(0, RAM).unwrap()).unwrap().to_vec();
    let last = RunStats { insns: 0, ..last };
    End { gpr: c.gpr, fpr: c.fpr, cr: c.cr, lr: c.lr, ctr: c.ctr, xer: c.xer, fpscr: c.fpscr, pc: c.pc, tb: c.tb, ram, last, total, calls: m.calls.clone() }
}

fn exec(tier: Tier, mutant: u8) -> Exec {
    let mut e = Exec::new(tier);
    e.set_code_mem(Box::new(HostCodeMem::new()));
    e.set_jit_publish_after(0); // short programs: publish every new block at once
    if mutant != 0 { e.plant_jit_mutant(mutant); }
    e
}

fn run_on(tier: Tier, mutant: u8, prog: &[u32], slice: u64, setup: impl Fn(&mut Cpu, &mut M)) -> (End, Exec) {
    let (mut c, mut m) = load(prog, &setup);
    let mut e = exec(tier, mutant);
    let end = finish(&mut e, &mut c, &mut m, slice);
    (end, e)
}

type Setup = fn(&mut Cpu, &mut M);
fn programs_under_test() -> Vec<(&'static str, &'static [u32], Setup)> {
    vec![
        ("crc32", programs::CRC32, |c, m| { m.mem.write_bytes(BUF, b"123456789").unwrap(); c.gpr[3] = BUF; c.gpr[4] = 9; }),
        ("fib", programs::FIB, |c, _| c.gpr[3] = 15),
        ("branches", programs::BRANCHES, |_, _| {}),
        ("horner", programs::HORNER, |c, m| { c.gpr[10] = BUF; c.set_fpr_f64(2, 1.5); for (i, k) in [3.0f64, 2.0, 1.0, 5.0].iter().enumerate() { m.mem.write_bytes(BUF + 8 * i as u32, &k.to_be_bytes()).unwrap(); } }),
        ("bench_int", programs::BENCH_INT, |c, _| c.gpr[3] = 300),
        ("bench_branch", programs::BENCH_BRANCH, |c, _| c.gpr[3] = 300),
        ("bench_mem", programs::BENCH_MEM, |c, m| { for i in 0..256u32 { m.mem.write_u32(BUF + 4 * i, i * 7).unwrap(); } c.gpr[3] = 3; c.gpr[4] = BUF; }),
        ("bench_fp", programs::BENCH_FP, |c, m| { for (i, k) in [0.5f64, 0.75, 0.125].iter().enumerate() { m.mem.write_bytes(BUF + 8 * i as u32, &k.to_be_bytes()).unwrap(); } c.gpr[3] = 300; c.gpr[4] = BUF; }),
        ("bench_callback", programs::BENCH_CALLBACK, |c, _| { c.gpr[3] = 100; c.gpr[4] = BUF; }),
        ("bench_smc", programs::BENCH_SMC, |c, _| c.gpr[3] = 100),
        ("smc_native", programs::SMC_NATIVE, |c, _| c.gpr[3] = 100),
    ]
}

#[test]
fn every_program_ends_in_tier_0s_state_on_tier_2_in_any_slicing() {
    for (name, prog, setup) in programs_under_test() {
        let (want, _) = run_on(Tier::Interp, 0, prog, 1_000_000, setup);
        assert!(want.last.halted && want.last.trap.is_none(), "{name} tier 0: {:?}", want.last);
        for slice in [1, 2, 3, 7, 64, 1_000, 1_000_000] {
            let (got, e) = run_on(Tier::Jit, 0, prog, slice, setup);
            assert!(got == want, "{name}: tier 2 (slice {slice}) differs from tier 0: pc {:#x}/{:#x} tb {}/{} gpr {:x?}/{:x?} cr {:#x}/{:#x} xer {:#x}/{:#x} last {:?}/{:?}", got.pc, want.pc, got.tb, want.tb, got.gpr, want.gpr, got.cr, want.cr, got.xer, want.xer, got.last, want.last);
            let s = e.jit_stats();
            assert!(!s.refused, "{name}: {s:?}");
            if slice == 1_000 && want.total > 20_000 { assert!(s.insns * 2 > want.total, "{name} (slice {slice}): most instructions ran in JIT code: {s:?} of {}", want.total); }
        }
    }
}

#[test]
fn jit_lockstep_compares_every_entry_and_finds_no_divergence() {
    for (name, prog, setup) in programs_under_test() {
        let (want, _) = run_on(Tier::Interp, 0, prog, 1_000_000, setup);
        let (got, e) = run_on(Tier::JitLockstep, 0, prog, 1_000, setup);
        assert_eq!(e.divergence(), None, "{name}");
        assert!(got == want, "{name}: lockstep keeps tier 0's state");
        if want.total > 20_000 { assert!(e.lockstep_stats().insns * 2 > want.total, "{name}: most instructions compared: {:?} {:?}", e.lockstep_stats(), e.jit_stats()); }
    }
}

#[test]
fn self_modifying_code_runs_the_new_instruction_with_icbi_and_without() {
    for (name, prog) in [("bench_smc", programs::BENCH_SMC), ("smc_native", programs::SMC_NATIVE)] {
        let (got, _) = run_on(Tier::Jit, 0, prog, 1_000_000, |c, _| c.gpr[3] = 100);
        assert_eq!(got.gpr[5], 150, "{name}");
    }
}

#[test]
fn without_executable_memory_tier_2_is_refused_by_name_and_tier_1_runs() {
    let (mut c, mut m) = load(programs::FIB, |c, _| c.gpr[3] = 15);
    let mut e = Exec::new(Tier::Jit);
    let got = finish(&mut e, &mut c, &mut m, 1_000_000);
    let (want, _) = run_on(Tier::Interp, 0, programs::FIB, 1_000_000, |c, _| c.gpr[3] = 15);
    assert!(got == want);
    assert!(e.jit_stats().refused && e.jit_stats().insns == 0, "{:?}", e.jit_stats());
}

// ---- hand-encoded PowerPC for the loops below (PEM chapter 8 encodings) ----
fn d(op: u32, rt: u32, ra: u32, imm: u32) -> u32 { op << 26 | rt << 21 | ra << 16 | (imm & 0xFFFF) }
fn x(rt: u32, ra: u32, rb: u32, xo: u32, rc: bool) -> u32 { 31 << 26 | rt << 21 | ra << 16 | rb << 11 | xo << 1 | rc as u32 }
fn rlwinm(ra: u32, rs: u32, sh: u32, mb: u32, me: u32, rc: bool) -> u32 { 21 << 26 | rs << 21 | ra << 16 | sh << 11 | mb << 6 | me << 1 | rc as u32 }
fn bc(bo: u32, bi: u32, rel: i32) -> u32 { 16 << 26 | bo << 21 | bi << 16 | (rel as u32 & 0xFFFC) }
fn mtspr(spr: u32, rs: u32) -> u32 { 31 << 26 | rs << 21 | (spr & 31) << 16 | (spr >> 5) << 11 | 467 << 1 }

/// A loop of 3 iterations whose 3rd load reads past the end of RAM: the trap mid-block, after the block went hot.
#[test]
fn a_trap_in_a_hot_block_leaves_tier_0s_pc_time_base_and_trap() {
    let prog = [
        d(15, 4, 0, (RAM - 8) >> 16), d(24, 4, 4, (RAM - 8) & 0xFFFF), // lis/ori r4 = RAM - 8
        d(14, 6, 0, 0), d(14, 31, 0, 6), // r6 = 0, r31 = 6
        x(5, 4, 6, 23, false), d(14, 6, 6, 4), x(7, 0, 0, 19, false), d(13, 31, 31, 0xFFFF), bc(4, 2, -16), // lwzx; addi; mfcr; addic.; bne
    ];
    for slice in [1_000_000, 5, 1] {
        let (want, _) = run_on(Tier::Interp, 0, &prog, slice, |_, _| {});
        assert!(want.last.trap.is_some(), "{:?}", want.last);
        let (got, _) = run_on(Tier::Jit, 0, &prog, slice, |_, _| {});
        assert!(got == want, "slice {slice}: {:?} / {:?} pc {:#x}/{:#x} tb {}/{}", got.last, want.last, got.pc, want.pc, got.tb, want.tb);
        let (got, e) = run_on(Tier::JitLockstep, 0, &prog, slice, |_, _| {});
        assert!(got == want && e.divergence().is_none(), "lockstep slice {slice}");
    }
}

/// A store that patches another page's code with no icbi (the code watch alone must catch it): each iteration toggles the
/// subroutine T (at CODE + 0x1000) between `addi r5,r5,1` and `addi r5,r5,2` and then calls it. The storing block's own page
/// is never written, so it stays compiled and its inline store is what runs.
const T: u32 = CODE + 0x1000;
fn smc_store_program() -> Vec<u32> {
    vec![
        d(14, 5, 0, 0), d(14, 31, 0, 50),
        d(15, 7, 0, ADDI_1 >> 16), d(24, 7, 7, ADDI_1 & 0xFFFF),
        d(15, 8, 0, T >> 16), d(24, 8, 8, T & 0xFFFF),
        d(26, 7, 7, 3), d(36, 7, 8, 0), 18 << 26 | ((T - (CODE + 32)) & 0x03FF_FFFC) | 1, // xori; stw r7,0(r8); bl T
        d(13, 31, 31, 0xFFFF), bc(4, 2, -16), // addic.; bne xori
        d(15, 24, 0, EXIT >> 16), d(24, 24, 24, EXIT & 0xFFFF), mtspr(8, 24),
    ]
}
fn smc_store_setup(_: &mut Cpu, m: &mut M) { m.mem.write_u32(T, NOP).unwrap(); m.mem.write_u32(T + 4, 0x4E80_0020).unwrap(); }

#[test]
fn a_store_into_code_with_no_icbi_runs_the_new_instruction() {
    let prog = smc_store_program();
    let (want, _) = run_on(Tier::Interp, 0, &prog, 1_000_000, smc_store_setup);
    assert_eq!(want.gpr[5], 75, "tier 0: 2+1+2+1... over 50 iterations");
    for slice in [1_000_000, 1_000, 13] {
        let (got, e) = run_on(Tier::Jit, 0, &prog, slice, smc_store_setup);
        assert!(got == want, "slice {slice}: r5 {} {:?}", got.gpr[5], e.jit_stats());
    }
    let (got, e) = run_on(Tier::JitLockstep, 0, &prog, 13, smc_store_setup);
    assert!(got == want && e.divergence().is_none(), "{:?}", e.divergence());
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u32 { self.0 ^= self.0 << 13; self.0 ^= self.0 >> 7; self.0 ^= self.0 << 17; (self.0 >> 16) as u32 }
    fn pick(&mut self, n: u32) -> u32 { self.next() % n }
    fn reg(&mut self) -> u32 { 3 + self.pick(10) }
}

/// A random loop body over every form tier 2 compiles inline (and some it hands to tier 1): r3-r12 data, r20 = BUF (never
/// written), r22 a moving base reset each iteration, r31 the trip count, f1-f4 FP data.
fn random_program(seed: u64) -> Vec<u32> {
    let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut p = vec![d(14, 31, 0, 25)];
    let top = p.len();
    p.push(d(14, 22, 20, 0x100)); // addi r22,r20,0x100
    let n = 6 + r.pick(18);
    for _ in 0..n {
        let (t, a, b) = (r.reg(), r.reg(), r.reg());
        let imm = r.next() & 0xFFFF;
        let off = r.pick(0x3F0) & !3;
        let w = match r.pick(34) {
            0 => d(14, t, a, imm), 1 => d(15, t, a, imm), 2 => d(24, a, t, imm), 3 => d(25, a, t, imm), 4 => d(26, a, t, imm),
            5 => d(28, a, t, imm), 6 => d(29, a, t, imm), 7 => d(7, t, a, imm), 8 => d(12, t, a, imm), 9 => d(13, t, a, imm),
            10 => d(8, t, a, imm),
            11 => rlwinm(a, t, r.pick(32), r.pick(32), r.pick(32), r.pick(2) == 1),
            12 => d(11, r.pick(8) << 2, a, imm), 13 => d(10, r.pick(8) << 2, a, imm),
            14 => x(r.pick(8) << 2, a, b, 0, false), 15 => x(r.pick(8) << 2, a, b, 32, false),
            16 => x(t, a, b, [444, 28, 316, 60, 124][r.pick(5) as usize], r.pick(2) == 1 && r.pick(2) == 0),
            17 => x(t, a, b, [266, 40, 235][r.pick(3) as usize], r.pick(2) == 1),
            18 => x(t, a, 0, 104, false), 19 => x(t, a, 0, [922, 954][r.pick(2) as usize], false),
            20 => x(t, a, r.pick(32), 824, r.pick(2) == 1), 21 => x(t, a, b, [24, 536][r.pick(2) as usize], false),
            22 => x(t, a, b, [792, 26, 491, 459, 10, 138, 19][r.pick(7) as usize], false), // sraw cntlzw divw divwu addc adde mfcr (tier 1)
            23 => d([32, 34, 40, 42][r.pick(4) as usize], t, 20, off | r.pick(4)),
            24 => d([33, 35][r.pick(2) as usize], t, 22, off & 0xFC), // lwzu / lbzu
            25 => d([36, 38, 44][r.pick(3) as usize], t, 20, 0x400 + off + r.pick(4)),
            26 => d([37, 39][r.pick(2) as usize], t, 22, 0x400 + (off & 0xFC)), // stwu / stbu
            27 => x(t, 20, 23, [23, 87, 151, 215][r.pick(4) as usize], false), // lwzx lbzx stwx stbx (r23 = a small offset)
            28 => d([50, 54, 48, 52][r.pick(4) as usize], 1 + r.pick(4), 20, 0x800 + (off & !7)), // lfd stfd lfs stfs
            29 => bc([4, 12, 20, 16, 18, 0, 2][r.pick(7) as usize], r.pick(32), 8), // skip the next instruction
            30 => x(t, 0, 0, [339 /* mflr */, 467][r.pick(2) as usize], false) | if r.pick(2) == 0 { 8 << 16 } else { 9 << 16 },
            31 => d(14, 23, 0, r.pick(0x80) * 4), // r23 = a small aligned offset
            32 => mtspr(1, t), // mtxer (tier 1): random SO/OV/CA for the next compares and carries
            _ => 0x6000_0000,
        };
        p.push(w);
    }
    p.push(0x6000_0000); // a body-ending skip lands here, never past the trip count
    p.push(d(13, 31, 31, 0xFFFF)); // addic. r31,r31,-1
    let back = -(4 * (p.len() - top) as i32);
    p.push(bc(4, 2, back)); // bne top
    p.push(d(15, 24, 0, EXIT >> 16));
    p.push(d(24, 24, 24, EXIT & 0xFFFF));
    p.push(mtspr(8, 24)); // mtlr r24 (the closing blr goes to EXIT)
    p
}

fn fuzz_setup(seed: u64) -> impl Fn(&mut Cpu, &mut M) {
    move |c, m| {
        let mut r = Rng(seed ^ 0x5DEE_CE66);
        for i in 3..13 { c.gpr[i] = r.next() ^ r.next() << 16; }
        c.gpr[20] = BUF;
        c.gpr[23] = 16;
        for i in 1..5 { c.fpr[i] = (r.next() as u64) << 32 | 0x3FF0_0000 << 32 >> 32 | r.next() as u64; }
        c.xer = r.next() & 0xE000_0000;
        c.cr = r.next();
        let bytes: Vec<u8> = (0..0x1000).map(|_| r.next() as u8).collect();
        m.mem.write_bytes(BUF, &bytes).unwrap();
    }
}

#[test]
fn a_random_program_fuzz_agrees_with_tier_0_on_tier_2_and_jit_lockstep() {
    let mut compared = 0u64;
    for seed in 0..400u64 {
        let prog = random_program(seed);
        let (want, _) = run_on(Tier::Interp, 0, &prog, 1_000_000, fuzz_setup(seed));
        assert!(want.last.halted, "seed {seed}: the generated loop ends ({:?})", want.last);
        // a run slice publishes what it compiled: short slices make the fuzz's short loops run in JIT code
        for slice in [37, 1_000, 1_000_000] {
            let (got, e) = run_on(Tier::Jit, 0, &prog, slice, fuzz_setup(seed));
            assert!(got == want, "seed {seed} slice {slice}: gpr {:x?}/{:x?} cr {:#x}/{:#x} xer {:#x}/{:#x} tb {}/{} last {:?}/{:?} ram equal {} {:?}",
                got.gpr, want.gpr, got.cr, want.cr, got.xer, want.xer, got.tb, want.tb, got.last, want.last, got.ram == want.ram, e.jit_stats());
        }
        let (got, e) = run_on(Tier::JitLockstep, 0, &prog, 50, fuzz_setup(seed));
        assert!(e.divergence().is_none(), "seed {seed}: {:?}", e.divergence());
        assert!(got == want, "seed {seed}: lockstep end state");
        compared += e.lockstep_stats().insns;
    }
    assert!(compared > 100_000, "the fuzz really compared JIT code: {compared}");
}

/// Each planted JIT mutant makes the gates RED: the fuzz (end state or lockstep) for the arithmetic ones, the self-modifying
/// programs for the store-watch and exit-count ones. A gate that stays green with a mutant proves nothing.
#[test]
fn negative_control_every_planted_jit_mutant_goes_red() {
    for mutant in 1..=4u8 {
        let mut red = false;
        for seed in 0..400u64 {
            let prog = random_program(seed);
            let (want, _) = run_on(Tier::Interp, 0, &prog, 1_000_000, fuzz_setup(seed));
            let (got, _) = run_on(Tier::Jit, mutant, &prog, 37, fuzz_setup(seed));
            if got != want { red = true; break; }
        }
        let smc = smc_store_program();
        for (prog, setup) in [(&smc[..], smc_store_setup as Setup), (programs::BENCH_SMC, |c, _| c.gpr[3] = 100)] {
            if red { break; }
            let (want, _) = run_on(Tier::Interp, 0, prog, 1_000_000, setup);
            let (got, _) = run_on(Tier::Jit, mutant, prog, 13, setup);
            red = got != want;
        }
        assert!(red, "mutant {mutant} stayed GREEN");
    }
}
