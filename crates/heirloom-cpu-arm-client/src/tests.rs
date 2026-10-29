//! The client against the core it stands in for: the same programs, run on heirloom-cpu-arm linked in and through its table,
//! give the same registers, exits and instruction counts.
use super::*;
use std::vec;
use std::vec::Vec;

const CODE: u32 = 0x8000;
const DATA: u32 = 0x9000;

/// 64 KiB of little-endian RAM; `no_exec_above` refuses FETCHES (not reads) at and above it, `alias_fault` reports every
/// refused access at that address (a memory whose fault address differs from the one asked for).
struct Ram {
    bytes: Vec<u8>,
    no_exec_above: u32,
    alias_fault: Option<u32>,
}

impl Ram {
    fn with_code(code: &[u32]) -> Ram {
        let mut r = Ram { bytes: vec![0; 0x10000], no_exec_above: u32::MAX, alias_fault: None };
        for (i, w) in code.iter().enumerate() {
            r.put32(CODE + 4 * i as u32, *w);
        }
        r
    }
    fn with_thumb(at: u32, halves: &[u16], ram: &mut Ram) {
        for (i, h) in halves.iter().enumerate() {
            let a = (at + 2 * i as u32) as usize;
            ram.bytes[a..a + 2].copy_from_slice(&h.to_le_bytes());
        }
    }
    fn put32(&mut self, a: u32, v: u32) {
        self.bytes[a as usize..a as usize + 4].copy_from_slice(&v.to_le_bytes());
    }
    fn fault(&self, addr: u32, access: Access) -> Fault {
        Fault { addr: self.alias_fault.unwrap_or(addr), access }
    }
    fn at(&self, addr: u32, n: u32, access: Access) -> Result<usize, Fault> {
        match addr.checked_add(n) {
            Some(end) if end as usize <= self.bytes.len() => Ok(addr as usize),
            _ => Err(self.fault(addr, access)),
        }
    }
}

impl Memory for Ram {
    fn read_u8(&mut self, a: u32) -> Result<u8, Fault> {
        Ok(self.bytes[self.at(a, 1, Access::Read)?])
    }
    fn read_u16(&mut self, a: u32) -> Result<u16, Fault> {
        let i = self.at(a, 2, Access::Read)?;
        Ok(u16::from_le_bytes([self.bytes[i], self.bytes[i + 1]]))
    }
    fn read_u32(&mut self, a: u32) -> Result<u32, Fault> {
        let i = self.at(a, 4, Access::Read)?;
        Ok(u32::from_le_bytes(self.bytes[i..i + 4].try_into().unwrap()))
    }
    fn write_u8(&mut self, a: u32, v: u8) -> Result<(), Fault> {
        let i = self.at(a, 1, Access::Write)?;
        self.bytes[i] = v;
        Ok(())
    }
    fn write_u16(&mut self, a: u32, v: u16) -> Result<(), Fault> {
        let i = self.at(a, 2, Access::Write)?;
        self.bytes[i..i + 2].copy_from_slice(&v.to_le_bytes());
        Ok(())
    }
    fn write_u32(&mut self, a: u32, v: u32) -> Result<(), Fault> {
        let i = self.at(a, 4, Access::Write)?;
        self.bytes[i..i + 4].copy_from_slice(&v.to_le_bytes());
        Ok(())
    }
    fn fetch_u16(&mut self, a: u32) -> Result<u16, Fault> {
        if a >= self.no_exec_above {
            return Err(self.fault(a, Access::Fetch));
        }
        let i = self.at(a, 2, Access::Fetch)?;
        Ok(u16::from_le_bytes([self.bytes[i], self.bytes[i + 1]]))
    }
    fn fetch_u32(&mut self, a: u32) -> Result<u32, Fault> {
        if a >= self.no_exec_above {
            return Err(self.fault(a, Access::Fetch));
        }
        let i = self.at(a, 4, Access::Fetch)?;
        Ok(u32::from_le_bytes(self.bytes[i..i + 4].try_into().unwrap()))
    }
}

fn bound() {
    bind(heirloom_cpu_arm::vtable()).expect("the core's own table binds");
}

/// The state both backends must agree on after each run.
fn seen_core(c: &heirloom_cpu_arm::Cpu) -> ([u32; 16], u32, u32) {
    (c.regs, c.cpsr(), c.vfp.fpscr)
}
fn seen_client(c: &Cpu) -> ([u32; 16], u32, u32) {
    (c.regs, c.cpsr(), c.vfp.fpscr)
}

/// Run `ram`'s program from CODE on both backends, slice by slice (`budget` each, the SVC immediate returned in r0 + 1 as an
/// "exec call result"), until a non-SVC, non-budget exit; every slice's exit, count and state must be equal.
fn lockstep(code: &[u32], thumb_at: Option<(u32, &[u16])>, budget: u64) -> Vec<(Exit, u64)> {
    bound();
    let mut ram_a = Ram::with_code(code);
    if let Some((at, h)) = thumb_at {
        Ram::with_thumb(at, h, &mut ram_a);
    }
    let mut ram_b = Ram { bytes: ram_a.bytes.clone(), no_exec_above: u32::MAX, alias_fault: None };
    let mut a = heirloom_cpu_arm::Cpu::new(Config::default());
    let mut b = Cpu::new(Config::default());
    a.regs[2] = DATA;
    b.regs[2] = DATA;
    a.regs[13] = 0xF000;
    b.regs[13] = 0xF000;
    a.set_pc_interworking(CODE);
    b.set_pc_interworking(CODE);
    let mut slices = Vec::new();
    for _ in 0..64 {
        let ra = a.run_counted(&mut ram_a, budget);
        let rb = b.run_counted(&mut ram_b, budget);
        assert_eq!(ra, rb, "the exit and the retired count");
        assert_eq!(seen_core(&a), seen_client(&b), "registers, CPSR and FPSCR after {ra:?}");
        assert_eq!(ram_a.bytes, ram_b.bytes, "memory after {ra:?}");
        slices.push(rb);
        match rb.0 {
            Exit::Svc(n) => {
                a.regs[0] = n + 1;
                b.regs[0] = n + 1;
            }
            Exit::BudgetSpent => {}
            _ => break,
        }
    }
    slices
}

#[test]
fn the_table_is_the_arm_core_at_the_frozen_version() {
    let t = heirloom_cpu_arm::vtable();
    assert_eq!(t.version, CPU_ABI_VERSION);
    assert_eq!(t.size, core::mem::size_of::<CpuVTable>() as u32);
    assert_eq!(t.arch_id, ARCH_ARM);
    assert_eq!((t.context_size)(ptr::null_mut()) as usize, core::mem::size_of::<ArmContext>());
}

#[test]
fn a_program_runs_the_same_through_the_table_as_linked_in() {
    // MOV r0,#1; loop: SUBS... - a counted loop, loads/stores, an SVC, a switch to Thumb and a Thumb SVC, then an UDF trap.
    let code = [
        0xE3A0_0005, // MOV r0, #5
        0xE250_0001, // loop: SUBS r0, r0, #1
        0x1AFF_FFFD, //       BNE loop
        0xE592_1000, // LDR r1, [r2]
        0xE281_1007, // ADD r1, r1, #7
        0xE582_1004, // STR r1, [r2, #4]
        0xEF00_0042, // SVC #0x42
        0xE59F_4004, // LDR r4, [pc, #4]  (the literal below)
        0xE12F_FF14, // BX r4 -> Thumb
        0xE7F0_00F0, // (not reached)
        CODE + 0x101, // literal: the Thumb entry | 1
    ];
    let thumb: &[u16] = &[0x2007, 0xDF05, 0xDE01]; // MOVS r0,#7; SVC #5; UDF #1
    let slices = lockstep(&code, Some((CODE + 0x100, thumb)), 3);
    assert!(slices.iter().any(|s| s.0 == Exit::Svc(0x42)), "{slices:?}");
    assert!(slices.iter().any(|s| s.0 == Exit::Svc(5)), "{slices:?}");
    assert!(matches!(slices.last().unwrap().0, Exit::Trap(Trap { thumb: true, kind: TrapKind::Undefined, .. })), "{slices:?}");
}

#[test]
fn run_counted_equals_one_step_per_instruction() {
    bound();
    let code = [0xE3A0_0003, 0xE250_0001, 0x1AFF_FFFD, 0xEF00_0001];
    let mut ram = Ram::with_code(&code);
    let mut stepped = Cpu::new(Config::default());
    stepped.set_pc_interworking(CODE);
    let mut n = 0;
    let exit = loop {
        n += 1;
        match stepped.step(&mut ram) {
            Exit::BudgetSpent => continue,
            e => break e,
        }
    };
    let mut ram = Ram::with_code(&code);
    let mut counted = Cpu::new(Config::default());
    counted.set_pc_interworking(CODE);
    assert_eq!(counted.run_counted(&mut ram, 1000), (exit, n));
    assert_eq!(counted.regs, stepped.regs);
}

#[test]
fn a_fault_is_the_memorys_own_and_a_fetch_is_a_fetch() {
    bound();
    // LDR r1, [r3] with r3 out of range; the memory names its fault at its own address.
    let mut ram = Ram::with_code(&[0xE593_1000]);
    ram.alias_fault = Some(0xDEAD_0000);
    let mut cpu = Cpu::new(Config::default());
    cpu.regs[3] = 0x0010_0000;
    cpu.set_pc_interworking(CODE);
    assert_eq!(cpu.run(&mut ram, 10), Exit::Fault { pc: CODE, fault: Fault { addr: 0xDEAD_0000, access: Access::Read } });
    assert_eq!(cpu.pc(), CODE, "the PC stays on the refused instruction");

    // A branch into memory that may be read but not executed faults as a FETCH (the engine's exec permission).
    let mut ram = Ram::with_code(&[0xEA00_003E]); // B CODE + 0x100
    ram.no_exec_above = CODE + 0x100;
    let mut cpu = Cpu::new(Config::default());
    cpu.set_pc_interworking(CODE);
    assert_eq!(cpu.run(&mut ram, 10), Exit::Fault { pc: CODE + 0x100, fault: Fault { addr: CODE + 0x100, access: Access::Fetch } });
}

#[test]
fn the_exclusive_monitor_travels_with_the_thread() {
    bound();
    // LDREX r1, [r2]; SVC #0; STREX r3, r1, [r2]; SVC #1
    let code = [0xE192_1F9F, 0xEF00_0000, 0xE182_3F91, 0xEF00_0001];
    let run = |clear: bool| {
        let mut ram = Ram::with_code(&code);
        let mut cpu = Cpu::new(Config::default());
        cpu.regs[2] = DATA;
        cpu.set_pc_interworking(CODE);
        assert_eq!(cpu.run(&mut ram, 10), Exit::Svc(0));
        // Another guest thread runs in between on the same library instance.
        let mut other = Cpu::new(Config::default());
        other.set_pc_interworking(CODE + 12);
        assert_eq!(other.run(&mut ram, 10), Exit::Svc(1));
        if clear {
            cpu.clear_exclusive();
        }
        assert_eq!(cpu.run(&mut ram, 10), Exit::Svc(1));
        cpu.regs[3]
    };
    assert_eq!(run(false), 0, "STREX succeeds: the reservation came back with the thread's context");
    assert_eq!(run(true), 1, "STREX fails after clear_exclusive");
}

#[test]
fn bind_refuses_a_table_of_another_architecture() {
    let t = heirloom_cpu_arm::vtable();
    let not_arm: &'static CpuVTable = Box::leak(Box::new(CpuVTable {
        version: t.version,
        size: t.size,
        arch_id: heirloom_cpu_abi::ARCH_M68K,
        create: t.create,
        destroy: t.destroy,
        reset: t.reset,
        run: t.run,
        interrupt: t.interrupt,
        get_reg: t.get_reg,
        set_reg: t.set_reg,
        invalidate_code: t.invalidate_code,
        context_size: t.context_size,
        save_context: t.save_context,
        load_context: t.load_context,
        recent_pcs: t.recent_pcs,
    }));
    assert!(matches!(bind(not_arm), Err(BindRefusal::Open(OpenRefusal::WrongArch { found: 1, required: 3 }))));
}

#[test]
fn the_table_refuses_what_no_core_has() {
    let t = heirloom_cpu_arm::vtable();
    let mut ram = Ram::with_code(&[]);
    let mut slot = BusSlot { mem: &mut ram, fault: None };
    let record = bus(&mut slot);
    assert!((t.create)(7, &record as *const ArmBusCallbacks as *const CpuBusCallbacks).is_null(), "sub_arch 7 is not ARMv6");
    let inst = (t.create)(SUB_ARCH_ARMV6, &record as *const ArmBusCallbacks as *const CpuBusCallbacks);
    assert!(!inst.is_null());
    let mut bad = Cpu::new(Config::default()).context();
    bad.unaligned = 9;
    let size = core::mem::size_of::<ArmContext>() as u32;
    assert_eq!((t.load_context)(inst, (&bad as *const ArmContext).cast(), size), -1, "an unknown alignment mode");
    bad.unaligned = 0;
    bad.exclusive_size = 3;
    assert_eq!((t.load_context)(inst, (&bad as *const ArmContext).cast(), size), -1, "a 3-byte reservation");
    assert_eq!((t.load_context)(inst, (&bad as *const ArmContext).cast(), size - 1), -1, "a short buffer");
    assert_eq!((t.interrupt)(inst, 1), -1, "a user-mode core takes no interrupt");
    let mut v = 0u64;
    assert_eq!((t.get_reg)(inst, 20, &mut v), -1, "register 20 does not exist");
    (t.destroy)(inst);
}
