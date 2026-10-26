//! Tier 1 (decoded blocks) against tier 0 (the reference): the same state at every exit, whatever the slicing, the
//! traps, the natives and the code modifications; and the planted stale-code mutant goes RED.
mod common;
use common::{asm, programs, CODE, EXIT, STACK};
use heirloom_mem_flat32::GuestMem;
use heirloom_cpu_ppc::{Cpu, Exec, Host, RunStats, Tier};

const BUF: u32 = 0x4000;
/// The native the self-modifying programs call: BlockZero (r3, r4) for bench_callback, the code patcher for smc_native.
const NATIVE: u32 = 0xF100;
const ADDI_1: u32 = 0x38A5_0001; // addi r5,r5,1
const NOP: u32 = 0x6000_0000;

struct M { mem: GuestMem, calls: Vec<u32> }
impl Host for M {
    fn mem(&mut self) -> &mut GuestMem { &mut self.mem }
    fn native(&mut self, cpu: &mut Cpu, addr: u32) -> bool {
        self.calls.push(addr);
        match addr {
            EXIT => { cpu.halt(); true }
            NATIVE if cpu.gpr[4] == 32 => self.mem.zero(cpu.gpr[3], 32).is_ok(),
            NATIVE => { // the code patcher of smc_native.s: nop -> addi 5,5,2, then toggle 2 <-> 1
                let w = self.mem.read_u32(cpu.gpr[3]).unwrap();
                self.mem.write_u32(cpu.gpr[3], if w == NOP { ADDI_1 ^ 3 } else { w ^ 3 }).is_ok()
            }
            _ => true,
        }
    }
    fn syscall(&mut self, cpu: &mut Cpu) -> bool { cpu.gpr[3] = cpu.gpr[0] + 1; true }
}

fn load(prog: &[u32], setup: impl FnOnce(&mut Cpu, &mut M)) -> (Cpu, M) {
    let mut m = M { mem: GuestMem::new(1 << 20), calls: vec![] };
    for (i, w) in prog.iter().enumerate() { m.mem.write_u32(CODE + 4 * i as u32, *w).unwrap(); }
    m.mem.write_u32(CODE + 4 * prog.len() as u32, asm("blr")).unwrap();
    let mut c = Cpu::new(CODE, STACK);
    c.lr = EXIT;
    c.bind_native(EXIT);
    c.bind_native(NATIVE);
    c.gpr[6] = NATIVE;
    setup(&mut c, &mut m);
    (c, m)
}

#[derive(Debug, PartialEq)]
struct End { gpr: [u32; 32], fpr: [u64; 32], cr: u32, lr: u32, ctr: u32, xer: u32, fpscr: u32, pc: u32, tb: u64, ram: Vec<u8>, last: RunStats, total: u64, calls: Vec<u32>, pcs: Vec<u32> }

/// Run to a stop (halt, trap or 2M instructions) in `slice`-instruction resumes on `exec`.
fn finish(exec: &mut Exec, c: &mut Cpu, m: &mut M, slice: u64) -> End {
    let mut total = 0;
    let last = loop {
        let st = exec.resume(m, c, slice);
        total += st.insns;
        if st.halted || st.trap.is_some() || total >= 2_000_000 || exec.divergence().is_some() { break st; }
    };
    let ram = m.mem.bytes(m.mem.span(0, 1 << 20).unwrap()).unwrap().to_vec();
    let last = RunStats { insns: 0, ..last }; // the size of the last slice depends on the slicing; `total` is compared
    End { gpr: c.gpr, fpr: c.fpr, cr: c.cr, lr: c.lr, ctr: c.ctr, xer: c.xer, fpscr: c.fpscr, pc: c.pc, tb: c.tb, ram, last, total, calls: m.calls.clone(), pcs: c.recent_pcs().collect() }
}

fn run_on(tier: Tier, prog: &[u32], slice: u64, setup: impl Fn(&mut Cpu, &mut M)) -> (End, Exec) {
    let (mut c, mut m) = load(prog, &setup);
    c.trace_pcs(16);
    let mut e = Exec::new(tier);
    let end = finish(&mut e, &mut c, &mut m, slice);
    (end, e)
}

fn programs_under_test() -> Vec<(&'static str, &'static [u32], fn(&mut Cpu, &mut M))> {
    vec![
        ("crc32", programs::CRC32, |c, m| { m.mem.write_bytes(BUF, b"123456789").unwrap(); c.gpr[3] = BUF; c.gpr[4] = 9; }),
        ("fib", programs::FIB, |c, _| c.gpr[3] = 15),
        ("branches", programs::BRANCHES, |_, _| {}),
        ("horner", programs::HORNER, |c, m| { c.gpr[10] = BUF; c.set_fpr_f64(2, 1.5); for (i, k) in [3.0f64, 2.0, 1.0, 5.0].iter().enumerate() { m.mem.write_bytes(BUF + 8 * i as u32, &k.to_be_bytes()).unwrap(); } }),
        ("int2dbl", programs::INT2DBL, |c, m| { c.gpr[3] = (-123456i32) as u32; c.gpr[10] = BUF; m.mem.write_bytes(BUF + 8, &0x4330_0000_8000_0000u64.to_be_bytes()).unwrap(); m.mem.write_bytes(BUF + 24, &1.5f64.to_be_bytes()).unwrap(); }),
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
fn every_program_ends_in_tier_0s_state_on_tier_1_in_any_slicing() {
    for (name, prog, setup) in programs_under_test() {
        let (want, _) = run_on(Tier::Interp, prog, 1_000_000, setup);
        assert!(want.last.halted && want.last.trap.is_none(), "{name} tier 0: {:?}", want.last);
        for slice in [1, 2, 3, 7, 64, 1_000_000] {
            let (got, e) = run_on(Tier::Blocks, prog, slice, setup);
            assert!(got == want, "{name}: tier 1 (slice {slice}) differs from tier 0: pc {:#x}/{:#x} tb {}/{} gpr {:x?}/{:x?} last {:?}/{:?}", got.pc, want.pc, got.tb, want.tb, got.gpr, want.gpr, got.last, want.last);
            if slice == 1_000_000 { assert!(e.prof.blocks > 0 && e.cache_stats().decodes > 0, "{name}: tier 1 really ran blocks"); }
        }
    }
}

#[test]
fn lockstep_compares_every_block_and_finds_no_divergence() {
    for (name, prog, setup) in programs_under_test() {
        let (want, _) = run_on(Tier::Interp, prog, 1_000_000, setup);
        let (got, e) = run_on(Tier::Lockstep, prog, 1_000, setup);
        assert_eq!(e.divergence(), None, "{name}");
        assert!(got == want, "{name}: lockstep keeps tier 0's state");
        let ls = e.lockstep_stats();
        // every instruction was compared, except native hooks and what ran on tier 0 itself (a volatile page)
        assert!(ls.blocks > 0 && ls.insns + e.prof.natives + e.cache_stats().interp_insns == want.total, "{name}: {ls:?} {:?}", e.cache_stats());
    }
}

#[test]
fn self_modifying_code_runs_the_new_instruction_with_icbi_and_without() {
    for (name, prog) in [("bench_smc (stw + icbi)", programs::BENCH_SMC), ("smc_native (a native's write, no icbi)", programs::SMC_NATIVE)] {
        let (got, e) = run_on(Tier::Blocks, prog, 1_000_000, |c, _| c.gpr[3] = 100);
        assert_eq!(got.gpr[5], 150, "{name}: 2+1+2+1... over 100 iterations");
        assert!(e.cache_stats().stale + e.cache_stats().decodes > 100, "{name}: blocks were re-decoded ({:?})", e.cache_stats());
    }
}

#[test]
fn negative_control_the_stale_code_mutant_goes_red() {
    for (name, prog) in [("bench_smc", programs::BENCH_SMC), ("smc_native", programs::SMC_NATIVE)] {
        // plain tier 1 with the mutant: the result is wrong (the old instruction ran)
        let (mut c, mut m) = load(prog, |c, _| c.gpr[3] = 100);
        let mut e = Exec::new(Tier::Blocks);
        e.plant_stale_code_mutant();
        let got = finish(&mut e, &mut c, &mut m, 1_000_000);
        assert_ne!(got.gpr[5], 150, "{name}: the mutant must run stale code, or this control proves nothing");
        // lockstep with the mutant: a divergence, named at the block, in r5
        let (mut c, mut m) = load(prog, |c, _| c.gpr[3] = 100);
        let mut e = Exec::new(Tier::Lockstep);
        e.plant_stale_code_mutant();
        finish(&mut e, &mut c, &mut m, 1_000_000);
        let d = e.divergence().unwrap_or_else(|| panic!("{name}: lockstep did not see the stale block"));
        assert!(d.what.iter().any(|w| w.starts_with("gpr[5]")), "{name}: {d:?}");
        let st = e.resume(&mut m, &mut c, 1000);
        assert!(st.halted && st.insns == 0, "after a divergence the run is over: {st:?}");
    }
}

#[test]
fn a_trap_mid_block_leaves_the_same_pc_time_base_and_trace() {
    // lwz from outside RAM in the middle of a straight run, then an unimplemented opcode, then an sc
    for prog in [
        vec![asm("li 3,1"), asm("lis 4,0x7FFF"), asm("lwz 5,0(4)"), asm("li 6,2")],
        vec![asm("li 3,1"), asm("li 4,2"), 0x0000_0000, asm("li 6,2")],
        vec![asm("li 0,41"), asm("sc"), asm("addi 3,3,1"), asm("li 6,2")],
        vec![asm("li 3,0x1000"), asm("lmw 29,1(3)")],
    ] {
        let (want, _) = run_on(Tier::Interp, &prog, 1_000_000, |_, _| {});
        let (got, _) = run_on(Tier::Blocks, &prog, 1_000_000, |_, _| {});
        assert!(got == want, "{prog:x?}: {:?} / {:?} pcs {:x?} / {:x?}", got.last, want.last, got.pcs, want.pcs);
    }
}

#[test]
fn a_native_bound_inside_an_already_decoded_block_is_called() {
    // decode the CRC loop on tier 1, then bind a native in the middle of its hot block: tier 1 must stop there
    let (mut c, mut m) = load(programs::CRC32, |c, m| { m.mem.write_bytes(BUF, b"abc").unwrap(); c.gpr[3] = BUF; c.gpr[4] = 3; });
    let mut e = Exec::new(Tier::Blocks);
    e.resume(&mut m, &mut c, 40);
    assert!(e.cache_stats().decodes > 0);
    let inner = CODE + 4 * 12; // `srwi 5,5,1`, inside the bit loop's block (andi. srwi beq)
    c.bind_native(inner);
    let mut cpu0 = c.clone();
    e.resume(&mut m, &mut c, 1_000);
    assert!(m.calls.contains(&inner), "the native bound after decoding was entered: {:x?}", m.calls);
    // and a context with the same natives (a clone) does not drop the cache again
    let flushes = e.cache_stats().flushes;
    cpu0.pc = CODE;
    e.resume(&mut m, &mut cpu0, 10);
    assert_eq!(e.cache_stats().flushes, flushes, "an equal natives set (a cloned context) keeps the blocks");
}

#[test]
fn a_page_rewritten_again_and_again_goes_to_tier_0_and_stays_correct() {
    let (want, _) = run_on(Tier::Interp, programs::BENCH_SMC, 1_000_000, |c, _| c.gpr[3] = 2_000);
    let (got, e) = run_on(Tier::Blocks, programs::BENCH_SMC, 1_000_000, |c, _| c.gpr[3] = 2_000);
    assert!(got == want);
    assert_eq!(got.gpr[5], 3_000);
    let s = e.cache_stats();
    assert_eq!(s.volatile_pages, 1, "{s:?}");
    assert!(s.interp_insns > 10_000 && s.decodes < 1_000, "most of the loop ran on tier 0 after the page went volatile: {s:?}");
    // NEGATIVE CONTROL: code that is never rewritten never goes volatile
    let (_, e) = run_on(Tier::Blocks, programs::BENCH_INT, 1_000_000, |c, _| c.gpr[3] = 2_000);
    assert_eq!(e.cache_stats().volatile_pages, 0);
}
