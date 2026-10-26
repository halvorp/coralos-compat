//! Test support: a sparse little-endian memory and the vector runner shared by the vector tests.
//!
//! A vector line is `name;iset;code;init;expect` (see tools/uc_oracle.py). The expectation is Unicorn's
//! ARM1176 state after one instruction; the runner executes the same instruction with `Cpu::step` and
//! compares every register, the CPSR flags, VFP state where the instruction is VFP, and every byte written.
#![allow(dead_code)]

use heirloom_cpu_arm::{Access, Cpu, Exit, Fault, Memory, TrapKind};
use std::collections::{BTreeMap, HashMap};

pub const MEM_SIZE: u32 = 0x1000_0000;
pub const CPSR_MASK: u32 = 0xF80F_0220;
pub const FPSCR_MASK: u32 = 0xF3F7_9F9F;

/// 256 MiB of zero-filled address space, paged on write; above it every access faults.
#[derive(Default)]
pub struct SparseMem {
    pages: HashMap<u32, Box<[u8; 4096]>>,
    pub writes: BTreeMap<u32, u8>,
}

impl SparseMem {
    pub fn poke(&mut self, addr: u32, bytes: &[u8]) {
        for (i, &b) in bytes.iter().enumerate() {
            let a = addr + i as u32;
            self.pages.entry(a >> 12).or_insert_with(|| Box::new([0; 4096]))[(a & 0xFFF) as usize] = b;
        }
    }

    pub fn peek(&self, addr: u32) -> u8 {
        self.pages.get(&(addr >> 12)).map_or(0, |p| p[(addr & 0xFFF) as usize])
    }

    fn check(&self, addr: u32, len: u32, access: Access) -> Result<(), Fault> {
        if addr.checked_add(len).is_none_or(|end| end > MEM_SIZE) {
            Err(Fault { addr, access })
        } else {
            Ok(())
        }
    }

    fn read(&self, addr: u32, len: u32, access: Access) -> Result<u32, Fault> {
        self.check(addr, len, access)?;
        Ok((0..len).fold(0, |v, i| v | (self.peek(addr + i) as u32) << (8 * i)))
    }

    fn write(&mut self, addr: u32, len: u32, value: u32) -> Result<(), Fault> {
        self.check(addr, len, Access::Write)?;
        for i in 0..len {
            let b = (value >> (8 * i)) as u8;
            self.poke(addr + i, &[b]);
            self.writes.insert(addr + i, b);
        }
        Ok(())
    }
}

impl Memory for SparseMem {
    fn read_u8(&mut self, addr: u32) -> Result<u8, Fault> {
        self.read(addr, 1, Access::Read).map(|v| v as u8)
    }
    fn read_u16(&mut self, addr: u32) -> Result<u16, Fault> {
        assert_eq!(addr & 1, 0, "the CPU promised aligned halfword accesses");
        self.read(addr, 2, Access::Read).map(|v| v as u16)
    }
    fn read_u32(&mut self, addr: u32) -> Result<u32, Fault> {
        assert_eq!(addr & 3, 0, "the CPU promised aligned word accesses");
        self.read(addr, 4, Access::Read)
    }
    fn write_u8(&mut self, addr: u32, value: u8) -> Result<(), Fault> {
        self.write(addr, 1, value as u32)
    }
    fn write_u16(&mut self, addr: u32, value: u16) -> Result<(), Fault> {
        assert_eq!(addr & 1, 0, "the CPU promised aligned halfword accesses");
        self.write(addr, 2, value as u32)
    }
    fn write_u32(&mut self, addr: u32, value: u32) -> Result<(), Fault> {
        assert_eq!(addr & 3, 0, "the CPU promised aligned word accesses");
        self.write(addr, 4, value)
    }
    fn fetch_u16(&mut self, addr: u32) -> Result<u16, Fault> {
        self.read(addr, 2, Access::Fetch).map(|v| v as u16)
    }
    fn fetch_u32(&mut self, addr: u32) -> Result<u32, Fault> {
        self.read(addr, 4, Access::Fetch)
    }
}

fn hex(s: &str) -> u32 {
    u32::from_str_radix(s, 16).unwrap_or_else(|_| panic!("bad hex {s}"))
}

fn bytes(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

/// The outcome classes the runner counts.
#[derive(Default, Debug)]
pub struct Report {
    pub passed: usize,
    pub smoke: usize,
    /// Encodings Unicorn executed and this CPU refused: UNPREDICTABLE per llvm-mc, or reviewed by hand.
    pub refused_unpredictable: usize,
    pub failures: Vec<String>,
}

impl Report {
    pub fn total(&self) -> usize {
        self.passed + self.smoke + self.refused_unpredictable + self.failures.len()
    }

    pub fn assert_clean(&self, what: &str) {
        eprintln!(
            "{what}: {} vectors: {} passed full-state, {} smoke (no panic), {} refused (UNPREDICTABLE/reviewed), {} failed",
            self.total(),
            self.passed,
            self.smoke,
            self.refused_unpredictable,
            self.failures.len()
        );
        for f in self.failures.iter().take(40) {
            eprintln!("  FAIL {f}");
        }
        assert!(self.failures.is_empty(), "{what}: {} of {} vectors failed", self.failures.len(), self.total());
    }
}

/// Full architectural state the vectors compare.
#[derive(Clone, PartialEq, Eq, Debug)]
struct State {
    r: [u32; 16],
    cpsr: u32,
    fpscr: u32,
    s: [u32; 32],
}

fn state(cpu: &Cpu) -> State {
    State { r: cpu.regs, cpsr: cpu.cpsr() & CPSR_MASK, fpscr: cpu.vfp.fpscr & FPSCR_MASK, s: cpu.vfp.s }
}

fn apply(st: &mut State, k: &str, v: &str) -> bool {
    match k {
        "cpsr" => st.cpsr = hex(v),
        "fpscr" => st.fpscr = hex(v),
        _ if k.starts_with('r') => st.r[k[1..].parse::<usize>().unwrap()] = hex(v),
        _ if k.starts_with('s') && k[1..].chars().all(|c| c.is_ascii_digit()) => {
            st.s[k[1..].parse::<usize>().unwrap()] = hex(v)
        }
        _ => return false,
    }
    true
}

/// Run every vector in `text`. `reviewed` lists the (iset, encoding) pairs checked by hand as refusals
/// (A32 words without the condition nibble).
pub fn run_vectors(text: &str, reviewed: &[Reviewed]) -> Report {
    let mut report = Report::default();
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split(';').collect();
        assert_eq!(fields.len(), 5, "malformed vector {line}");
        let (name, thumb, code) = (fields[0], fields[1] == "t16", bytes(fields[2]));
        let mut cpu = Cpu::default();
        let mut mem = SparseMem::default();
        mem.poke(0, &code);
        let mut budget = 1;
        for kv in fields[3].split_whitespace() {
            let (k, v) = kv.split_once('=').unwrap();
            match k {
                "cpsr" => cpu.set_cpsr(hex(v)),
                "fpscr" => cpu.vfp.fpscr = hex(v),
                "n" => budget = v.parse().unwrap(),
                _ if k.starts_with('m') => mem.poke(hex(&k[1..]), &bytes(v)),
                _ if k.starts_with('s') => cpu.vfp.s[k[1..].parse::<usize>().unwrap()] = hex(v),
                _ => cpu.regs[k[1..].parse::<usize>().unwrap()] = hex(v),
            }
        }
        cpu.thumb = thumb;
        let mut want = state(&cpu);
        mem.writes.clear();
        let insn = u32::from_le_bytes([code[0], code[1], code[2], code[3]]);
        let exit = cpu.run(&mut mem, budget);
        let expect: Vec<(&str, &str)> = fields[4].split_whitespace().map(|kv| kv.split_once('=').unwrap()).collect();
        let get = |key: &str| expect.iter().find(|(k, _)| *k == key).map(|(_, v)| *v);
        if get("smoke").is_some() {
            report.smoke += 1;
            continue;
        }
        let want_exit = get("exit").unwrap();
        let exit_ok = match (want_exit, exit) {
            ("ok", Exit::BudgetSpent) | ("svc", Exit::Svc(_)) | ("bkpt", Exit::Bkpt(_)) => true,
            ("undef", Exit::Trap(t)) => !matches!(t.kind, TrapKind::Unaligned { .. }),
            ("fault", Exit::Fault { .. }) => true,
            ("ok", Exit::Trap(t)) if budget == 1 => {
                let key = if thumb { (true, insn & 0xFFFF) } else { (false, insn & 0x0FFF_FFFF) };
                let unpredictable = matches!(t.kind, TrapKind::Unpredictable | TrapKind::Privileged);
                if (unpredictable && get("unp").is_some()) || reviewed_hit(reviewed, key.0, key.1) {
                    report.refused_unpredictable += 1;
                    continue;
                }
                false
            }
            _ => false,
        };
        let mut diffs = Vec::new();
        if !exit_ok {
            diffs.push(format!("exit {exit:?}, want {want_exit}"));
        }
        for (k, v) in &expect {
            if *k == "exit" || *k == "unp" || apply(&mut want, k, v) {
                continue;
            }
            let (addr, want_bytes) = (hex(&k[1..]), bytes(v));
            for (i, b) in want_bytes.iter().enumerate() {
                let a = addr + i as u32;
                if mem.writes.remove(&a) != Some(*b) {
                    diffs.push(format!("mem[{a:#x}] = {:#x}, want {b:#x}", mem.peek(a)));
                }
            }
        }
        let got = state(&cpu);
        for i in 0..16 {
            if got.r[i] != want.r[i] {
                diffs.push(format!("r{i} = {:#x}, want {:#x}", got.r[i], want.r[i]));
            }
        }
        if got.cpsr != want.cpsr {
            diffs.push(format!("cpsr = {:#x}, want {:#x}", got.cpsr, want.cpsr));
        }
        if got.fpscr != want.fpscr {
            diffs.push(format!("fpscr = {:#x}, want {:#x}", got.fpscr, want.fpscr));
        }
        for i in 0..32 {
            if got.s[i] != want.s[i] {
                diffs.push(format!("s{i} = {:#x}, want {:#x}", got.s[i], want.s[i]));
            }
        }
        if !mem.writes.is_empty() {
            diffs.push(format!("unexpected writes at {:x?}", mem.writes.keys().collect::<Vec<_>>()));
        }
        if diffs.is_empty() {
            report.passed += 1;
        } else {
            report.failures.push(format!("{name} [{} {} {}] {}", fields[1], fields[2], fields[3], diffs.join(", ")));
        }
    }
    report
}

pub fn reviewed_unpredictable() -> Vec<Reviewed> {
    include_str!("../data/unpredictable_reviewed.txt")
        .lines()
        .filter_map(|l| l.split('#').next().map(str::trim).filter(|w| !w.is_empty()))
        .map(|entry| {
            let (iset, word) = entry.split_once(' ').expect("`a32 <hex>[/<mask>]` or `t16 <hex>[/<mask>]`");
            let (value, mask) = match word.trim().split_once('/') {
                Some((v, m)) => (hex(v), hex(m)),
                None => (hex(word.trim()), u32::MAX),
            };
            Reviewed { thumb: iset == "t16", value, mask }
        })
        .collect()
}

/// A reviewed refusal: matches when `insn & mask == value` (A32 words without the cond nibble).
pub struct Reviewed {
    thumb: bool,
    value: u32,
    mask: u32,
}

fn reviewed_hit(list: &[Reviewed], thumb: bool, insn: u32) -> bool {
    list.iter().any(|r| r.thumb == thumb && insn & r.mask == r.value)
}

/// Code base address for the hand tests.
pub const CODE: u32 = 0x1000;

/// A CPU at `CODE` in A32 state with `words` there.
pub fn a32(words: &[u32]) -> (Cpu, SparseMem) {
    let mut mem = SparseMem::default();
    for (i, w) in words.iter().enumerate() {
        mem.poke(CODE + 4 * i as u32, &w.to_le_bytes());
    }
    let mut cpu = Cpu::default();
    cpu.regs[15] = CODE;
    (cpu, mem)
}

/// A CPU at `CODE` in Thumb state with `halfwords` there.
pub fn t16(halfwords: &[u16]) -> (Cpu, SparseMem) {
    let mut mem = SparseMem::default();
    for (i, h) in halfwords.iter().enumerate() {
        mem.poke(CODE + 2 * i as u32, &h.to_le_bytes());
    }
    let mut cpu = Cpu::default();
    cpu.regs[15] = CODE;
    cpu.thumb = true;
    (cpu, mem)
}

pub fn peek32(mem: &SparseMem, addr: u32) -> u32 {
    u32::from_le_bytes([mem.peek(addr), mem.peek(addr + 1), mem.peek(addr + 2), mem.peek(addr + 3)])
}

/// The trap kind of an exit, panicking with the exit otherwise.
pub fn trap_kind(exit: Exit) -> TrapKind {
    match exit {
        Exit::Trap(t) => t.kind,
        other => panic!("expected a trap, got {other:?}"),
    }
}
