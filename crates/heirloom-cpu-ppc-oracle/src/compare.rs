//! Compare two snapshots; a [`Relax`] says which bits an exclusion (exclude.rs) declares undefined or untracked.
use crate::world::{Snap, Tr};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diff {
    pub field: String,
    pub heirloom: String,
    pub oracle: String,
}

/// Which bits of each register take part in the comparison (1 = compared), and which named exclusions made the mask.
#[derive(Clone, Debug)]
pub struct Relax {
    pub gpr: [u32; 32],
    pub fpr: [u64; 32],
    pub cr: u32,
    pub fpscr: u32,
    pub names: Vec<&'static str>,
    /// Byte ranges of memory not compared (the store whose result a model, not the oracle, judges).
    pub mem_skip: Vec<(usize, usize)>,
    /// A reference model (models.rs) disagrees with heirloom-cpu-ppc: always a discrepancy.
    pub model_fail: Vec<String>,
}

impl Relax {
    pub fn full() -> Relax { Relax { gpr: [!0; 32], fpr: [!0; 32], cr: !0, fpscr: !0, names: Vec::new(), mem_skip: Vec::new(), model_fail: Vec::new() } }
    pub fn note(&mut self, n: &'static str) { if !self.names.contains(&n) { self.names.push(n); } }
}

pub fn diff(a: &Snap, b: &Snap, rx: &Relax) -> Vec<Diff> {
    let mut out = Vec::new();
    let mut chk = |field: &str, x: u64, y: u64, hex: usize| {
        if x != y { out.push(Diff { field: field.into(), heirloom: format!("{x:#0hex$x}"), oracle: format!("{y:#0hex$x}") }); }
    };
    if a.trap != b.trap {
        out.push(Diff { field: "trap".into(), heirloom: format!("{:?} [{}]", a.trap, a.note), oracle: format!("{:?} [{}]", b.trap, b.note) });
        return out;
    }
    chk("pc", a.pc as u64, b.pc as u64, 10);
    // After a bus fault / alignment trap the architecture allows partial completion: compare nothing but the class.
    if matches!(a.trap, Tr::Bus { .. } | Tr::Align) { return out; }
    for i in 0..32 {
        chk(&format!("r{i}"), (a.gpr[i] & rx.gpr[i]) as u64, (b.gpr[i] & rx.gpr[i]) as u64, 10);
        chk(&format!("f{i}"), a.fpr[i] & rx.fpr[i], b.fpr[i] & rx.fpr[i], 18);
    }
    chk("cr", (a.cr & rx.cr) as u64, (b.cr & rx.cr) as u64, 10);
    chk("xer", (a.xer & heirloom_cpu_ppc::cpu::XER_MASK) as u64, (b.xer & heirloom_cpu_ppc::cpu::XER_MASK) as u64, 10);
    chk("lr", a.lr as u64, b.lr as u64, 10);
    chk("ctr", a.ctr as u64, b.ctr as u64, 10);
    chk("fpscr", (a.fpscr & rx.fpscr) as u64, (b.fpscr & rx.fpscr) as u64, 10);
    chk("tb", a.tb, b.tb, 18);
    for m in &rx.model_fail { out.push(Diff { field: "model".into(), heirloom: m.clone(), oracle: "PEM".into() }); }
    let same = |i: usize| a.mem[i] == b.mem[i] || rx.mem_skip.iter().any(|&(lo, hi)| (lo..hi).contains(&i));
    if let Some(i) = (0..a.mem.len()).find(|&i| !same(i)) {
        out.push(Diff { field: format!("mem[{i:#x}]"), heirloom: format!("{:#04x}", a.mem[i]), oracle: format!("{:#04x}", b.mem[i]) });
    }
    out
}
