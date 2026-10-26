//! Run a case on heirloom-cpu-ppc and on ppc-rs in lockstep, one instruction at a time.
use crate::state::{fill_window, State, CODE, RAM};
use ppc::{PpcAlignmentPolicy, PpcCpu, PpcException, PpcMemory, PpcSectionMem, PpcStepResult};
use heirloom_mem_flat32::GuestMem;
use heirloom_cpu_ppc::cpu::{Cpu, Trap};
use heirloom_cpu_ppc::{Exec, Host, Tier};

/// Trap classes both sides are mapped onto. `Illegal` folds heirloom's Unimplemented / InvalidForm / Privileged and
/// ppc-rs's Unimplemented / IllegalInstruction / FloatingPointUnavailable (the detail is kept in `Snap::note`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tr {
    None,
    Illegal,
    Bus { write: bool },
    Align,
    Sys,
    TrapWord,
}

#[derive(Clone, Debug)]
pub struct Snap {
    pub gpr: [u32; 32],
    pub fpr: [u64; 32],
    pub cr: u32,
    pub xer: u32,
    pub lr: u32,
    pub ctr: u32,
    pub fpscr: u32,
    pub pc: u32,
    pub tb: u64,
    pub mem: Vec<u8>,
    pub trap: Tr,
    pub note: String,
    /// heirloom's reason when the trap is an `InvalidForm` (an encoding the architecture marks invalid).
    pub invalid_why: Option<&'static str>,
}

struct H(GuestMem);
impl Host for H {
    fn mem(&mut self) -> &mut GuestMem { &mut self.0 }
}

/// Which alignment policy the oracle runs under for this case: `Trap` for the instructions whose misaligned
/// behaviour is what is being compared (lmw/stmw/lwarx/stwcx. require word alignment), `EmulateData` otherwise
/// (heirloom, like the 6xx in big-endian mode, handles unaligned plain loads/stores).
pub struct Pair {
    /// The heirloom-cpu-ppc execution tier under test (one runner for the whole run, so tier 1's block cache sees every
    /// case's code replaced by the next: invalidation is exercised too).
    exec: Exec,
    heirloom: Cpu,
    h: H,
    ppc: PpcCpu,
    mem: PpcSectionMem,
}

impl Default for Pair { fn default() -> Self { Pair::new() } }

/// The tier `Pair::new` tests: `HEIRLOOM_PPC_TIER` (`0`, `1`, `lockstep`) if set, else heirloom-cpu-ppc's default.
pub fn tier_from_env() -> Tier {
    match std::env::var("HEIRLOOM_PPC_TIER") {
        Ok(t) => Tier::parse(&t).unwrap_or_else(|| panic!("HEIRLOOM_PPC_TIER={t:?}: expected 0, 1 or lockstep")),
        Err(_) => Exec::default().tier,
    }
}

impl Pair {
    pub fn new() -> Pair { Pair::with_tier(tier_from_env()) }
    pub fn with_tier(tier: Tier) -> Pair {
        let mut mem = PpcSectionMem::new();
        mem.add_region(0, vec![0; RAM]);
        Pair { exec: Exec::new(tier), heirloom: Cpu::new(0, 0), h: H(GuestMem::new(RAM)), ppc: PpcCpu::new(), mem }
    }
    pub fn tier(&self) -> Tier { self.exec.tier }
    /// NEGATIVE CONTROL: tier 1 with its code-page invalidation disabled (see heirloom_cpu_ppc::Exec::plant_stale_code_mutant).
    pub fn plant_stale_code_mutant(&mut self) { self.exec.plant_stale_code_mutant(); }

    /// Load `words` at CODE and `st` into both machines.
    pub fn load(&mut self, words: &[u32], st: &State, strict_align: bool) {
        let mut ram = vec![0u8; RAM];
        fill_window(&mut ram, st.memseed);
        for (i, w) in words.iter().enumerate() {
            let a = CODE as usize + 4 * i;
            ram[a..a + 4].copy_from_slice(&w.to_be_bytes());
        }
        self.h.0.write_bytes(0, &ram).unwrap();
        self.mem.write_bytes(0, &ram).unwrap();
        let c = &mut self.heirloom;
        *c = Cpu::new(CODE, 0);
        c.gpr = st.gpr; c.fpr = st.fpr; c.cr = st.cr; c.xer = st.xer; c.lr = st.lr; c.ctr = st.ctr; c.fpscr = st.fpscr;
        c.tb = st.tb;
        let p = &mut self.ppc;
        p.gpr = st.gpr; p.fpr = st.fpr; p.cr = st.cr; p.xer = st.xer; p.lr = st.lr; p.ctr = st.ctr; p.fpscr = st.fpscr;
        p.pc = CODE;
        p.invalidate_reservation();
        p.set_time_base(st.tb);
        p.alignment_policy = if strict_align { PpcAlignmentPolicy::Trap } else { PpcAlignmentPolicy::EmulateData };
    }

    /// Make the oracle machine's state equal to heirloom-cpu-ppc's (registers, pc, time base, memory). Used after an
    /// excluded instruction, whose difference is already accounted for, so that the rest of a sequence is still
    /// compared instead of dropped. The reservation is cleared on the oracle only; callers do not resync across
    /// lwarx/stwcx. (see `run.rs`).
    pub fn resync_oracle(&mut self) {
        let c = &self.heirloom;
        let p = &mut self.ppc;
        p.gpr = c.gpr; p.fpr = c.fpr; p.cr = c.cr; p.xer = c.xer; p.lr = c.lr; p.ctr = c.ctr; p.fpscr = c.fpscr; p.pc = c.pc;
        p.set_time_base(c.tb);
        p.invalidate_reservation();
        let ram = self.h.0.bytes(self.h.0.span(0, RAM as u32).unwrap()).unwrap();
        self.mem.write_bytes(0, ram).unwrap();
    }

    pub fn pcs(&self) -> (u32, u32) { (self.heirloom.pc, self.ppc.pc) }

    pub fn snap_heirloom(&mut self, trap: Tr, note: String, why: Option<&'static str>) -> Snap {
        let c = &self.heirloom;
        let mem = self.h.0.bytes(self.h.0.span(0, RAM as u32).unwrap()).unwrap().to_vec();
        Snap { gpr: c.gpr, fpr: c.fpr, cr: c.cr, xer: c.xer, lr: c.lr, ctr: c.ctr, fpscr: c.fpscr, pc: c.pc, tb: c.tb, mem, trap, note, invalid_why: why }
    }
    pub fn snap_ppc(&mut self, trap: Tr, note: String) -> Snap {
        let c = &self.ppc;
        let mut mem = vec![0u8; RAM];
        self.mem.read_bytes_into(0, &mut mem).unwrap();
        Snap { gpr: c.gpr, fpr: c.fpr, cr: c.cr, xer: c.xer, lr: c.lr, ctr: c.ctr, fpscr: c.fpscr, pc: c.pc, tb: c.time_base(), mem, trap, note, invalid_why: None }
    }

    /// Both machines' current state, as snapshots with no trap.
    pub fn now(&mut self) -> (Snap, Snap) { (self.snap_heirloom(Tr::None, String::new(), None), self.snap_ppc(Tr::None, String::new())) }

    /// Execute one instruction on each machine and snapshot both.
    pub fn step(&mut self) -> (Snap, Snap) {
        let (mut rt, mut rn, mut why) = (Tr::None, String::new(), None);
        let s = self.exec.resume(&mut self.h, &mut self.heirloom, 1);
        if let Some(d) = self.exec.divergence() { panic!("lockstep: tier 1 and tier 0 diverged at {:#x}: {:?}", d.block, d.what); }
        if let Some(t) = s.trap {
            rn = format!("{t}");
            rt = match t {
                Trap::Unimplemented { .. } | Trap::Privileged { .. } | Trap::NativeUnhandled { .. } => Tr::Illegal,
                Trap::InvalidForm { why: w, .. } => { why = Some(w); Tr::Illegal }
                Trap::BusFault { write, .. } => Tr::Bus { write },
                Trap::Alignment { .. } => Tr::Align,
                Trap::Syscall { .. } => Tr::Sys,
                Trap::TrapWord { .. } => Tr::TrapWord,
            };
        }
        let (mut pt, mut pn) = (Tr::None, String::new());
        let w = self.mem.read_u32_be(self.ppc.pc).unwrap();
        match self.ppc.step(&mut self.mem, w) {
            PpcStepResult::Stepped => {}
            PpcStepResult::Unimplemented(e) => { pt = Tr::Illegal; pn = format!("unimplemented {e:?}"); }
            PpcStepResult::MemoryFault { was_write, addr } => { pt = Tr::Bus { write: was_write }; pn = format!("memory fault at {addr:#x}"); }
            PpcStepResult::Exception(e) => {
                pn = format!("{e:?}");
                pt = match e {
                    PpcException::ProgramTrap { .. } => Tr::TrapWord,
                    PpcException::SystemCall { .. } => Tr::Sys,
                    PpcException::Alignment { .. } => Tr::Align,
                    _ => Tr::Illegal,
                };
            }
        }
        (self.snap_heirloom(rt, rn, why), self.snap_ppc(pt, pn))
    }
}
