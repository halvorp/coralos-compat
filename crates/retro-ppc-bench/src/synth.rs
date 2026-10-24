//! Synthetic PowerPC workloads: small programs (crates/retro-ppc/tests/asm/bench_*.s, assembled by LLVM through
//! tools/ppc-regen.py) that each stress one thing an interpreter can be slow at.
use retro_guest_mem::GuestMem;
use retro_ppc::{Cpu, Exec, Host};

#[allow(dead_code)]
mod programs { include!("../../retro-ppc/tests/common/programs.rs"); }

pub const CODE: u32 = 0x1000;
pub const EXIT: u32 = 0xF000;
/// The native the callback workload calls: BlockZero(r3 = ptr, r4 = bytes).
pub const BLOCKZERO: u32 = 0xF100;
pub const BUF: u32 = 0x4000;
pub const STACK: u32 = 0x8000;
const BLR: u32 = 0x4E80_0020;

/// The machine a synthetic workload runs on: 1 MiB of RAM, EXIT halts, BLOCKZERO zeroes guest memory natively.
pub struct Vm { pub mem: GuestMem, pub natives: u64 }

impl Host for Vm {
    fn mem(&mut self) -> &mut GuestMem { &mut self.mem }
    fn native(&mut self, cpu: &mut Cpu, addr: u32) -> bool {
        self.natives += 1;
        match addr {
            EXIT => { cpu.halt(); true }
            BLOCKZERO => self.mem.zero(cpu.gpr[3], cpu.gpr[4]).is_ok(),
            _ => false,
        }
    }
}

pub struct Synth {
    pub name: &'static str,
    pub what: &'static str,
    pub prog: &'static [u32],
    /// r3 (iterations / passes).
    pub n: u32,
    /// Extra setup (buffers, registers) after the program is loaded.
    pub setup: fn(&mut Cpu, &mut Vm),
    /// The program's own result check (an independent model, or an invariant).
    pub check: fn(&Cpu, &Vm, u32) -> Result<(), String>,
}

fn none(_: &mut Cpu, _: &mut Vm) {}
fn ok(_: &Cpu, _: &Vm, _: u32) -> Result<(), String> { Ok(()) }

/// bench_int.s modelled in Rust.
fn int_model(n: u32) -> (u32, u32) {
    let (mut r5, mut r6) = (0x1234_5678u32, 0u32);
    for _ in 0..n {
        r5 ^= r5 << 13; r5 ^= r5 >> 17; r5 ^= r5 << 5;
        r6 = r6.wrapping_add(r5.wrapping_mul(r6));
        let r9 = r6.rotate_left(3) & 0xFFFF_FFF8;
        r6 = r6.wrapping_sub(r9).wrapping_add(7) | (r5 & 0xFF);
    }
    (r5, r6)
}

pub fn all() -> Vec<Synth> {
    vec![
        Synth { name: "synth-int", what: "integer ALU: shifts, xor, mullw, rlwinm, record forms", prog: programs::BENCH_INT, n: 1_000_000, setup: none,
            check: |c, _, n| { let m = int_model(n); if (c.gpr[5], c.gpr[6]) == m { Ok(()) } else { Err(format!("r5,r6 = {:#x},{:#x}, model {m:#x?}", c.gpr[5], c.gpr[6])) } } },
        Synth { name: "synth-branch", what: "conditional branches, bl/blr, bctrl", prog: programs::BENCH_BRANCH, n: 1_500_000, setup: none,
            check: |c, _, n| { let w = (1..=n).map(|i| if i & 3 < 2 { 3 } else { 1 }).sum::<u32>(); // i&3 = 0: bl leaf, 1: bctrl leaf (+3 each), else +1
                if c.gpr[5] == w { Ok(()) } else { Err(format!("r5 = {}, want {w}", c.gpr[5])) } } },
        Synth { name: "synth-mem", what: "lwzu/stwu copy, lwz/lbz/lhz/stw walk", prog: programs::BENCH_MEM, n: 8_000,
            setup: |c, v| { for i in 0..256u32 { v.mem.write_u32(BUF + 4 * i, i.wrapping_mul(0x9E37_79B9)).unwrap(); } c.gpr[4] = BUF; }, check: ok },
        Synth { name: "synth-fp", what: "fmadd/fmul/fsub/fadds/fabs, lfd/stfd", prog: programs::BENCH_FP, n: 1_000_000,
            setup: |c, v| { for (i, k) in [0.5f64, 0.75, 0.125].iter().enumerate() { v.mem.write_bytes(BUF + 8 * i as u32, &k.to_be_bytes()).unwrap(); } c.gpr[4] = BUF; },
            check: |c, _, _| { let x = c.fpr_f64(4); if x.is_finite() && x != 0.0 { Ok(()) } else { Err(format!("f4 = {x}")) } } },
        Synth { name: "synth-callback", what: "a native BlockZero callout every 11 instructions", prog: programs::BENCH_CALLBACK, n: 500_000,
            setup: |c, _| { c.gpr[4] = BUF; c.gpr[6] = BLOCKZERO; c.bind_native(BLOCKZERO); },
            check: |c, v, n| { if c.gpr[5] == n && v.natives == n as u64 + 1 { Ok(()) } else { Err(format!("r5 = {}, natives {}", c.gpr[5], v.natives)) } } },
        Synth { name: "synth-smc", what: "self-modifying code: stw + icbi patches the next instruction every iteration", prog: programs::BENCH_SMC, n: 1_000_000, setup: none,
            check: |c, _, n| { if c.gpr[5] == 3 * n / 2 { Ok(()) } else { Err(format!("r5 = {}, want {} (a stale instruction ran)", c.gpr[5], 3 * n / 2)) } } },
    ]
}

/// Load `s` (program, trailing blr, EXIT bound, LR = EXIT), ready to run.
pub fn load(s: &Synth) -> (Cpu, Vm) {
    let mut v = Vm { mem: GuestMem::new(1 << 20), natives: 0 };
    for (i, w) in s.prog.iter().enumerate() { v.mem.write_u32(CODE + 4 * i as u32, *w).unwrap(); }
    v.mem.write_u32(CODE + 4 * s.prog.len() as u32, BLR).unwrap();
    let mut c = Cpu::new(CODE, STACK);
    c.lr = EXIT;
    c.bind_native(EXIT);
    c.gpr[3] = s.n;
    (s.setup)(&mut c, &mut v);
    (c, v)
}

/// Run a loaded workload to its exit on `exec`; the RunStats of the last slice. `slice` is the budget per resume
/// (the cooperative scheduler's quantum: a host gets control back that often).
pub fn run(exec: &mut Exec, c: &mut Cpu, v: &mut Vm, slice: u64) -> retro_ppc::RunStats {
    loop {
        let st = exec.resume(v, c, slice);
        if st.halted || st.trap.is_some() || st.blocked { return st; }
    }
}
