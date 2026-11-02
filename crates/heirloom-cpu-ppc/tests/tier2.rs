//! Tier 2 (the x86-64 JIT) against tier 0 (the reference): the same state at every exit, whatever the slicing, the traps,
//! the natives and the code modifications; JitLockstep finds no divergence; a random-program fuzz over every inline form
//! agrees; and each planted JIT mutant goes RED. Host only (the JIT needs executable memory: HostCodeMem).
#![cfg(all(target_arch = "x86_64", target_os = "linux"))] // HostCodeMem: the crate's own dev-dependency enables host-codemem
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
        let w = match r.pick(50) {
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
            // the forms jit_inline.rs takes off the slow path (each also has its slow cases: NaNs, denormals, RN, Rc)
            34 => fa(63, fr(&mut r), fr(&mut r), fr(&mut r), fr(&mut r), [20, 21, 25][r.pick(3) as usize], r.pick(8) == 0),
            35 => fa(59, fr(&mut r), fr(&mut r), fr(&mut r), fr(&mut r), [20, 21, 25][r.pick(3) as usize], r.pick(8) == 0),
            36 => 63 << 26 | r.pick(8) << 23 | fr(&mut r) << 16 | fr(&mut r) << 11 | [0, 32][r.pick(2) as usize] << 1, // fcmpu / fcmpo
            37 => 63 << 26 | fr(&mut r) << 21 | fr(&mut r) << 11 | [40, 72, 136, 264][r.pick(4) as usize] << 1 | (r.pick(8) == 0) as u32,
            38 => x(t, a, b, 792, r.pick(2) == 1), // sraw(.)
            39 => x(t, a, 0, [922, 954][r.pick(2) as usize], true), // extsh. / extsb.
            40 => x(t, 20, 23, [279, 343, 407, 534, 790, 662, 918][r.pick(7) as usize], false), // lhzx lhax sthx lwbrx lhbrx stwbrx sthbrx
            41 => x(t, 0, 23, [23, 87][r.pick(2) as usize], false), // lwzx / lbzx with RA = 0 (low memory)
            42 => d([48, 52][r.pick(2) as usize], fr(&mut r), 20, 0xC00 + (off & 0x1F8)), // lfs / stfs over the special singles
            43 => d(14, 23, 0, r.pick(0x80) * 4 + (r.pick(4) == 0) as u32), // r23: sometimes unaligned
            44 => 20 << 26 | t << 21 | a << 16 | r.pick(32) << 11 | r.pick(32) << 6 | r.pick(32) << 1 | r.pick(2), // rlwimi(.)
            45 => x(t, a, b, [10, 138, 202, 8, 136, 200, 26][r.pick(7) as usize], r.pick(2) == 1), // carry chain, cntlzw (+Rc)
            46 => x(t, a, b, [75, 11, 491, 459][r.pick(4) as usize], r.pick(2) == 1), // mulhw(u), divw(u) (+Rc)
            47 => 63 << 26 | fr(&mut r) << 21 | fr(&mut r) << 11 | 12 << 1, // frsp
            48 => x(fr(&mut r), 20, 23, [535, 663][r.pick(2) as usize], false), // lfsx / stfsx (r23 into BUF)
            49 => d(14, [b, 0][r.pick(2) as usize], 0, [0, 0xFFFF, 1][r.pick(3) as usize]), // zero / -1 / 1 divisors and operands
            _ => 0x6000_0000,
        };
        p.push(w);
    }
    p.push(0x6000_0000); // a body-ending skip lands here, never past the trip count
    // the trip count in cr7, so CR0 (every Rc=1 form's) reaches the end state instead of being overwritten each iteration
    p.push(d(12, 31, 31, 0xFFFF)); // addic r31,r31,-1
    p.push(d(11, 7 << 2, 31, 0)); // cmpwi cr7,r31,0
    let back = -(4 * (p.len() - top) as i32);
    p.push(bc(4, 30, back)); // bne cr7,top
    p.push(d(15, 24, 0, EXIT >> 16));
    p.push(d(24, 24, 24, EXIT & 0xFFFF));
    p.push(mtspr(8, 24)); // mtlr r24 (the closing blr goes to EXIT)
    p
}

fn fa(op: u32, d: u32, a: u32, b: u32, c: u32, xo: u32, rc: bool) -> u32 { op << 26 | d << 21 | a << 16 | b << 11 | c << 6 | xo << 1 | rc as u32 }
fn fr(r: &mut Rng) -> u32 { 1 + r.pick(6) }
/// FP values that take every path: specials (NaNs incl. signalling, infinities, zeros, denormals of both ranges, the
/// single range's edges) and ordinary numbers of both precisions.
fn special_fp(r: &mut Rng) -> u64 {
    const S: [u64; 14] = [0x7FF8_0000_0000_0001, 0x7FF0_0000_0000_0001, 0xFFF0_0000_0000_0000, 0x7FF0_0000_0000_0000, 0, 1 << 63, 1, 0x8000_0000_0000_0010,
        0x3810_0000_0000_0000 /* 2^-126 */, 0x380F_FFFF_FFFF_FFFF, 0x47EF_FFFF_E000_0000 /* f32::MAX */, 0x47EF_FFFF_F000_0000, 0x3FF0_0000_0000_0001, 0xC059_0000_0000_0000];
    match r.pick(3) {
        0 => S[r.pick(S.len() as u32) as usize],
        1 => (((r.next() as i32) as f64) / (1 + r.pick(1000)) as f64).to_bits(),
        _ => ((((r.next() as i32) as f32) / (1 + r.pick(1000)) as f32) as f64).to_bits(),
    }
}

fn fuzz_setup(seed: u64) -> impl Fn(&mut Cpu, &mut M) {
    move |c, m| {
        let mut r = Rng(seed ^ 0x5DEE_CE66);
        for i in 3..13 { c.gpr[i] = r.next() ^ r.next() << 16; }
        c.gpr[20] = BUF;
        c.gpr[23] = 16;
        for i in 1..7 { c.fpr[i] = special_fp(&mut r); }
        if r.pick(16) == 0 { c.fpscr = r.pick(4); } // a directed rounding mode: every arithmetic form is tier 0's refusal
        c.xer = r.next() & 0xE000_0000;
        c.cr = r.next();
        let bytes: Vec<u8> = (0..0x1000).map(|_| r.next() as u8).collect();
        m.mem.write_bytes(BUF, &bytes).unwrap();
        // singles for lfs at BUF + 0xC00: NaNs (quiet / signalling), infinities, zeros, denormals, ordinary values
        for k in 0..64u32 {
            let v = [0x7FC0_0001u32, 0x7F80_0001, 0xFF80_0000, 0x8000_0000, 0x0000_0001, 0x007F_FFFF, 0x3F80_0000, r.next()][(k % 8) as usize];
            m.mem.write_u32(BUF + 0xC00 + 4 * k, v).unwrap();
        }
    }
}

#[test]
fn a_random_program_fuzz_agrees_with_tier_0_on_tier_2_and_jit_lockstep() {
    let (mut compared, mut pinned, mut compiled) = (0u64, 0u64, 0u64);
    for seed in 0..400u64 {
        let prog = random_program(seed);
        let (want, _) = run_on(Tier::Interp, 0, &prog, 1_000_000, fuzz_setup(seed));
        assert!(want.last.halted || want.last.trap.is_some(), "seed {seed}: the generated loop ends, halted or trapped ({:?})", want.last);
        // a run slice publishes what it compiled: short slices make the fuzz's short loops run in JIT code
        for slice in [37, 1_000, 1_000_000] {
            let (got, e) = run_on(Tier::Jit, 0, &prog, slice, fuzz_setup(seed));
            (pinned, compiled) = (pinned + e.jit_stats().pinned, compiled + e.jit_stats().compiled);
            assert!(got == want, "seed {seed} slice {slice}: gpr {:x?}/{:x?} cr {:#x}/{:#x} xer {:#x}/{:#x} tb {}/{} last {:?}/{:?} ram equal {} {:?}",
                got.gpr, want.gpr, got.cr, want.cr, got.xer, want.xer, got.tb, want.tb, got.last, want.last, got.ram == want.ram, e.jit_stats());
        }
        // the BSWAP forms too (a CPU without MOVBE): the same state
        let (mut c, mut m) = load(&prog, fuzz_setup(seed));
        let mut e = exec(Tier::Jit, 0);
        e.set_jit_movbe(false);
        let got = finish(&mut e, &mut c, &mut m, 37);
        assert!(got == want && !e.jit_stats().movbe, "seed {seed}: without MOVBE");
        let (got, e) = run_on(Tier::JitLockstep, 0, &prog, 50, fuzz_setup(seed));
        assert!(e.divergence().is_none(), "seed {seed}: {:?}", e.divergence());
        assert!(got == want, "seed {seed}: lockstep end state");
        compared += e.lockstep_stats().insns;
    }
    // a floor that proves JIT code ran (~100k with this generator: programs that trap early compare less)
    assert!(compared > 50_000, "the fuzz really compared JIT code: {compared}");
    // and the register cache really held guest fields in host registers (blocks pin ~2-5 of them)
    assert!(pinned > compiled, "the register cache engaged: {pinned} fields pinned over {compiled} blocks");
}

/// A tiny code arena (the stub page and 16 segments of one page; the tests publish after every run, a page each time):
/// segments fill and are reused all the time.
const SMALL_ARENA: usize = 4096 + 16 * 4096;

fn run_small_arena(tier: Tier, mutant: u8, prog: &[u32], slice: u64, setup: impl Fn(&mut Cpu, &mut M)) -> (End, Exec) {
    let (mut c, mut m) = load(prog, &setup);
    let mut e = exec(tier, 0);
    e.set_jit_arena(SMALL_ARENA);
    if mutant != 0 { e.plant_jit_mutant(mutant); }
    let end = finish(&mut e, &mut c, &mut m, slice);
    (end, e)
}

/// Every program under test and 120 fuzz seeds with the small arena, at two slicings and under JitLockstep, against tier 0:
/// (evictions, blocks evicted) or the first difference.
fn small_arena_workload(mutant: u8) -> Result<(u64, u64), String> {
    let (mut evictions, mut evicted) = (0u64, 0u64);
    let mut progs: Vec<(String, Vec<u32>, Box<dyn Fn(&mut Cpu, &mut M)>)> =
        programs_under_test().into_iter().map(|(n, p, s)| (n.to_string(), p.to_vec(), Box::new(s) as Box<dyn Fn(&mut Cpu, &mut M)>)).collect();
    for seed in 0..120u64 { progs.push((format!("fuzz {seed}"), random_program(seed), Box::new(fuzz_setup(seed)))); }
    for (name, prog, setup) in &progs {
        let (want, _) = run_on(Tier::Interp, 0, prog, 1_000_000, setup);
        for slice in [37, 1_000] {
            let (got, e) = run_small_arena(Tier::Jit, mutant, prog, slice, setup);
            let s = e.jit_stats();
            if got != want { return Err(format!("{name} slice {slice}: differs from tier 0 with a small arena: {s:?}")); }
            if s.arena_full != 0 { return Err(format!("{name}: a whole-cache drop for want of space: {s:?}")); }
            (evictions, evicted) = (evictions + s.evictions, evicted + s.evicted);
        }
        let (got, e) = run_small_arena(Tier::JitLockstep, mutant, prog, 50, setup);
        if e.divergence().is_some() || got != want { return Err(format!("{name}: lockstep with a small arena: {:?}", e.divergence())); }
    }
    Ok((evictions, evicted))
}

/// A full arena drops its OLDEST segment's blocks, never all of them (the p99 hitch of a whole-arena flush): with a tiny
/// arena every program and fuzz seed reuses segments over and over, still ends in tier 0's state, JitLockstep finds no
/// divergence, and no whole-cache flush happens for want of space.
#[test]
fn a_full_arena_reuses_its_oldest_segment_and_stays_exact() {
    let (evictions, evicted) = small_arena_workload(0).unwrap();
    assert!(evictions > 100 && evicted > 100, "segments really were reused: {evictions} evictions, {evicted} blocks dropped");
}

/// Negative control for eviction, in a child process (the mutant makes the dispatcher enter pages that are no longer
/// executable or hold other code: the process may die, which a test cannot survive in-process). The child runs the
/// small-arena workload; it must pass without the mutant and fail with it.
#[test]
fn negative_control_eviction_that_leaves_blocks_reachable_goes_red() {
    let child = |mutant: u8| std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "evict_child", "--ignored", "--test-threads=1"]).env("TIER2_EVICT_MUTANT", mutant.to_string())
        .output().unwrap();
    let ok = child(0);
    assert!(ok.status.success(), "the child is green without the mutant: {}", String::from_utf8_lossy(&ok.stdout));
    assert!(!child(11).status.success(), "mutant 11 (evicted blocks left reachable) stayed GREEN");
}

#[test]
#[ignore = "run by negative_control_eviction_that_leaves_blocks_reachable_goes_red in a child process"]
fn evict_child() {
    let mutant: u8 = std::env::var("TIER2_EVICT_MUTANT").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    small_arena_workload(mutant).unwrap();
}

/// CR0 of Rc=1 forms whose x86 flags are NOT the result's sign test (an add or subtract that overflows: OF = 1), left in the
/// end state: `add.` / `subf.` of 0x7FFFFFFF and 1 in a hot loop (the trip count in cr7).
fn overflow_flags_program() -> Vec<u32> {
    let mut p = vec![d(14, 31, 0, 20)];
    p.push(x(5, 3, 4, 40, true)); // subf. r5,r3,r4 (1 - 0x7FFFFFFF: no overflow)
    p.push(x(6, 3, 4, 266, true)); // add. r6,r3,r4 (0x80000000: overflow, LT)
    p.push(d(12, 31, 31, 0xFFFF));
    p.push(d(11, 7 << 2, 31, 0));
    p.push(bc(4, 30, -16));
    p
}
fn overflow_flags_setup(c: &mut Cpu, _: &mut M) { c.gpr[3] = 0x7FFF_FFFF; c.gpr[4] = 1; }

#[test]
fn rc_forms_set_cr0_from_the_result_even_when_the_add_overflows() {
    let prog = overflow_flags_program();
    let (want, _) = run_on(Tier::Interp, 0, &prog, 1_000_000, overflow_flags_setup);
    assert_eq!(want.cr >> 28, 0b1000, "tier 0: 0x80000000 is negative");
    for slice in [1_000_000, 7] {
        let (got, e) = run_on(Tier::Jit, 0, &prog, slice, overflow_flags_setup);
        assert!(got == want && e.jit_stats().insns > 0, "slice {slice}: cr {:#x}/{:#x} {:?}", got.cr, want.cr, e.jit_stats());
    }
}

/// Each planted JIT mutant makes the gates RED: the fuzz (end state or lockstep) for the arithmetic ones, the self-modifying
/// programs for the store-watch and exit-count ones. A gate that stays green with a mutant proves nothing.
#[test]
fn negative_control_every_planted_jit_mutant_goes_red() {
    for mutant in 1..=10u8 {
        let mut red = false;
        for seed in 0..400u64 {
            let prog = random_program(seed);
            let (want, _) = run_on(Tier::Interp, 0, &prog, 1_000_000, fuzz_setup(seed));
            let (got, _) = run_on(Tier::Jit, mutant, &prog, 37, fuzz_setup(seed));
            if got != want { red = true; break; }
        }
        let (smc, flags) = (smc_store_program(), overflow_flags_program());
        for (prog, setup) in [(&smc[..], smc_store_setup as Setup), (programs::BENCH_SMC, |c, _| c.gpr[3] = 100), (&flags[..], overflow_flags_setup)] {
            if red { break; }
            let (want, _) = run_on(Tier::Interp, 0, prog, 1_000_000, setup);
            let (got, _) = run_on(Tier::Jit, mutant, prog, 13, setup);
            red = got != want;
        }
        assert!(red, "mutant {mutant} stayed GREEN");
    }
}
