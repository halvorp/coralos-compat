//! The host-facing contract: every way `run`/`step` returns, where the PC is left, that refused
//! instructions change nothing, the alignment modes, and the CP15/VFP configuration traps.

mod common;
use common::{a32, peek32, t16, trap_kind, SparseMem, CODE};
use heirloom_cpu_arm::{Access, Config, Cpu, Exit, Fault, TrapKind, Unaligned};

#[test]
fn svc_exits_with_the_immediate_and_resumes_after_it() {
    // MOV r0, #1; SVC #0x123456; ADD r0, r0, #1
    let (mut cpu, mut mem) = a32(&[0xE3A0_0001, 0xEF12_3456, 0xE280_0001]);
    assert_eq!(cpu.run(&mut mem, 100), Exit::Svc(0x12_3456));
    assert_eq!((cpu.regs[0], cpu.pc()), (1, CODE + 8));
    cpu.regs[0] = 41; // the host's exec-call result
    assert_eq!(cpu.step(&mut mem), Exit::BudgetSpent);
    assert_eq!(cpu.regs[0], 42);
}

#[test]
fn conditional_svc_not_taken_does_not_exit() {
    let (mut cpu, mut mem) = a32(&[0x0F00_0001]); // SVCEQ #1 with Z clear
    assert_eq!(cpu.step(&mut mem), Exit::BudgetSpent);
}

#[test]
fn thumb_svc_and_bkpt() {
    let (mut cpu, mut mem) = t16(&[0xDFAB, 0xBE07]);
    assert_eq!(cpu.run(&mut mem, 10), Exit::Svc(0xAB));
    assert_eq!(cpu.pc(), CODE + 2);
    assert_eq!(cpu.run(&mut mem, 10), Exit::Bkpt(7));
    assert_eq!(cpu.pc(), CODE + 2, "PC stays on the BKPT");
}

#[test]
fn a32_bkpt_and_wait_hints() {
    let (mut cpu, mut mem) = a32(&[0xE12F_FF7F]); // BKPT #0xFFFF
    assert_eq!(cpu.step(&mut mem), Exit::Bkpt(0xFFFF));
    assert_eq!(cpu.pc(), CODE);
    let (mut cpu, mut mem) = a32(&[0xE320_F003, 0xE320_F002, 0xE320_F001, 0xE320_F004, 0xE320_F000]);
    assert_eq!(cpu.step(&mut mem), Exit::Wait); // WFI
    assert_eq!(cpu.step(&mut mem), Exit::Wait); // WFE
    assert_eq!(cpu.run(&mut mem, 3), Exit::BudgetSpent); // YIELD, SEV, NOP
    assert_eq!(cpu.pc(), CODE + 20);
}

#[test]
fn budget_counts_instructions_including_failed_conditions() {
    // loop: SUBS r0, r0, #1; BNE loop
    let (mut cpu, mut mem) = a32(&[0xE250_0001, 0x1AFF_FFFD]);
    cpu.regs[0] = 1000;
    assert_eq!(cpu.run(&mut mem, 11), Exit::BudgetSpent);
    assert_eq!((cpu.regs[0], cpu.pc()), (994, CODE + 4));
    assert_eq!(cpu.run(&mut mem, 0), Exit::BudgetSpent);
    assert_eq!(cpu.pc(), CODE + 4);
}

#[test]
fn undefined_trap_names_the_instruction_and_changes_nothing() {
    let (mut cpu, mut mem) = a32(&[0xE7F0_00F0]); // UDF
    cpu.regs[0] = 5;
    let before = cpu.clone();
    match cpu.step(&mut mem) {
        Exit::Trap(t) => {
            assert_eq!((t.pc, t.insn, t.thumb, t.kind), (CODE, 0xE7F0_00F0, false, TrapKind::Undefined));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(cpu.regs, before.regs);
}

#[test]
fn neon_and_other_coprocessors_are_named_traps() {
    let (mut cpu, mut mem) = a32(&[0xF201_0D02]); // VADD.F32 d0, d1, d2 (NEON)
    assert_eq!(trap_kind(cpu.step(&mut mem)), TrapKind::Undefined);
    let (mut cpu, mut mem) = a32(&[0xEE10_0E10]); // MRC p14 (debug)
    assert_eq!(trap_kind(cpu.step(&mut mem)), TrapKind::Coprocessor { cp: 14 });
    let (mut cpu, mut mem) = a32(&[0xED93_0100]); // LDC p1
    assert_eq!(trap_kind(cpu.step(&mut mem)), TrapKind::Coprocessor { cp: 1 });
}

#[test]
fn unpredictable_forms_are_refused() {
    for word in [
        0xE090_F001u32, // ADDS pc, r0, r1: an exception return (privileged)
        0xE081_0F12,    // ADD r0, r1, r2, LSL pc
        0xE8B1_0003,    // LDMIA r1!, {r0, r1}: base in the list with writeback is fine for LDM...
    ] {
        let (mut cpu, mut mem) = a32(&[word]);
        cpu.regs[1] = 0x200;
        let exit = cpu.step(&mut mem);
        if word == 0xE8B1_0003 {
            assert_eq!(exit, Exit::BudgetSpent, "...and the loaded value wins");
            continue;
        }
        assert!(matches!(exit, Exit::Trap(_)), "{word:#010x}: {exit:?}");
        assert_eq!(cpu.pc(), CODE);
    }
    // STMIA r1!, {r0, r1, r2}: the stored base is not the lowest register.
    let (mut cpu, mut mem) = a32(&[0xE8A1_0007]);
    cpu.regs[1] = 0x200;
    assert_eq!(trap_kind(cpu.step(&mut mem)), TrapKind::Unpredictable);
    assert!(mem.writes.is_empty());
}

#[test]
fn memory_fault_leaves_registers_and_pc_untouched() {
    // LDMIA r0!, {r1-r4} across the end of mapped memory: the last word faults.
    let (mut cpu, mut mem) = a32(&[0xE8B0_001E]);
    cpu.regs[0] = common::MEM_SIZE - 8;
    let before = cpu.clone();
    assert_eq!(
        cpu.step(&mut mem),
        Exit::Fault { pc: CODE, fault: Fault { addr: common::MEM_SIZE, access: Access::Read } }
    );
    assert_eq!(cpu.regs, before.regs);
    // A fetch fault reports Fetch.
    let mut cpu = Cpu::default();
    cpu.regs[15] = common::MEM_SIZE;
    assert!(matches!(cpu.step(&mut SparseMem::default()), Exit::Fault { fault: Fault { access: Access::Fetch, .. }, .. }));
}

#[test]
fn armv6_unaligned_word_and_halfword_accesses_work() {
    // LDR r1, [r0]; LDRH r2, [r0, #2]; STR r3, [r0, #5]; LDRSH r4, [r0, #1]
    let (mut cpu, mut mem) = a32(&[0xE590_1000, 0xE1D0_20B2, 0xE580_3005, 0xE1D0_40F1]);
    mem.poke(0x201, &[0x11, 0x22, 0x33, 0x44, 0x85]);
    cpu.regs[0] = 0x201;
    cpu.regs[3] = 0xA1B2_C3D4;
    assert_eq!(cpu.run(&mut mem, 4), Exit::BudgetSpent);
    assert_eq!(cpu.regs[1], 0x4433_2211);
    assert_eq!(cpu.regs[2], 0x4433);
    assert_eq!(peek32(&mem, 0x206), 0xA1B2_C3D4);
    assert_eq!(cpu.regs[4], 0x3322);
}

#[test]
fn armv6_multiples_doubles_and_exclusives_need_alignment() {
    for word in [0xE890_0006u32, 0xE1C0_20D0, 0xE190_1F9F, 0xED90_0A00, 0xE100_1092] {
        // LDMIA r0, {r1, r2}; LDRD r2, r3, [r0]; LDREX r1, [r0]; VLDR s0, [r0]; SWP r1, r2, [r0]
        let (mut cpu, mut mem) = a32(&[word]);
        cpu.regs[0] = 0x202;
        assert_eq!(
            trap_kind(cpu.step(&mut mem)),
            TrapKind::Unaligned { addr: 0x202, write: false },
            "{word:#010x}"
        );
        assert_eq!((cpu.regs[1], cpu.regs[2]), (0, 0));
    }
    // LDREXH needs only halfword alignment; LDREXD needs eight bytes.
    let (mut cpu, mut mem) = a32(&[0xE1F0_1F9F, 0xE1B4_2F9F]);
    cpu.regs[0] = 0x202;
    cpu.regs[4] = 0x204;
    assert_eq!(cpu.step(&mut mem), Exit::BudgetSpent);
    assert_eq!(trap_kind(cpu.step(&mut mem)), TrapKind::Unaligned { addr: 0x204, write: false });
}

#[test]
fn legacy_mode_rotates_unaligned_loads_and_aligns_stores() {
    let mut cpu = Cpu::new(Config { unaligned: Unaligned::Legacy, ..Config::default() });
    let (_, mut mem) = a32(&[0xE590_1000, 0xE580_3000, 0xE890_0030]); // LDR r1,[r0]; STR r3,[r0]; LDM r0,{r4,r5}
    cpu.regs[15] = CODE;
    mem.poke(0x200, &[0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]);
    cpu.regs[0] = 0x201;
    cpu.regs[3] = 0xCAFE_BABE;
    assert_eq!(cpu.run(&mut mem, 1), Exit::BudgetSpent);
    assert_eq!(cpu.regs[1], 0x1144_3322, "the aligned word rotated right by 8");
    assert_eq!(cpu.run(&mut mem, 2), Exit::BudgetSpent);
    assert_eq!(peek32(&mem, 0x200), 0xCAFE_BABE, "stored at the aligned address");
    assert_eq!((cpu.regs[4], cpu.regs[5]), (0xCAFE_BABE, 0x8877_6655), "LDM ignores bits [1:0]");
}

#[test]
fn strict_mode_faults_every_unaligned_access() {
    let mut cpu = Cpu::new(Config { unaligned: Unaligned::Strict, ..Config::default() });
    let (_, mut mem) = a32(&[0xE590_1000, 0xE1D0_10B0, 0xE5D0_1000]); // LDR, LDRH, LDRB
    cpu.regs[15] = CODE;
    cpu.regs[0] = 0x201;
    assert_eq!(trap_kind(cpu.step(&mut mem)), TrapKind::Unaligned { addr: 0x201, write: false });
    cpu.regs[15] += 4;
    assert_eq!(trap_kind(cpu.step(&mut mem)), TrapKind::Unaligned { addr: 0x201, write: false });
    cpu.regs[15] += 4;
    assert_eq!(cpu.step(&mut mem), Exit::BudgetSpent, "bytes are always aligned");
}

#[test]
fn ldr_pc_interworks_and_rejects_bad_arm_targets() {
    let (mut cpu, mut mem) = a32(&[0xE590_F000]); // LDR pc, [r0]
    cpu.regs[0] = 0x200;
    mem.poke(0x200, &0x3001u32.to_le_bytes());
    assert_eq!(cpu.step(&mut mem), Exit::BudgetSpent);
    assert_eq!((cpu.pc(), cpu.thumb), (0x3000, true));
    let (mut cpu, mut mem) = a32(&[0xE590_F000]);
    cpu.regs[0] = 0x200;
    mem.poke(0x200, &0x3002u32.to_le_bytes());
    assert_eq!(trap_kind(cpu.step(&mut mem)), TrapKind::Unpredictable);
}

#[test]
fn data_processing_to_pc_branches_without_interworking() {
    let (mut cpu, mut mem) = a32(&[0xE1A0_F000]); // MOV pc, r0
    cpu.regs[0] = 0x4001;
    assert_eq!(cpu.step(&mut mem), Exit::BudgetSpent);
    assert_eq!((cpu.pc(), cpu.thumb), (0x4000, false), "ARMv6 ALUWritePC is BranchWritePC");
}

#[test]
fn host_clear_exclusive_makes_strex_fail() {
    let (mut cpu, mut mem) = a32(&[0xE191_0F9F, 0xE181_2F93]);
    cpu.regs[1] = 0x100;
    assert_eq!(cpu.step(&mut mem), Exit::BudgetSpent);
    cpu.clear_exclusive(); // a thread switch
    assert_eq!(cpu.step(&mut mem), Exit::BudgetSpent);
    assert_eq!(cpu.regs[2], 1);
    assert!(mem.writes.is_empty());
}

#[test]
fn setend_be_swaps_data_but_not_instructions() {
    // SETEND BE; LDR r1, [r0]; STRH r1, [r0, #4]; SETEND LE
    let (mut cpu, mut mem) = a32(&[0xF101_0200, 0xE590_1000, 0xE1C0_10B4, 0xF101_0000]);
    mem.poke(0x200, &[0x11, 0x22, 0x33, 0x44]);
    cpu.regs[0] = 0x200;
    assert_eq!(cpu.run(&mut mem, 3), Exit::BudgetSpent);
    assert!(cpu.big_endian);
    assert_eq!(cpu.regs[1], 0x1122_3344);
    assert_eq!((mem.peek(0x204), mem.peek(0x205)), (0x33, 0x44));
    assert_eq!(cpu.cpsr() & (1 << 9), 1 << 9, "CPSR.E");
    assert_eq!(cpu.step(&mut mem), Exit::BudgetSpent);
    assert!(!cpu.big_endian);
}

#[test]
fn msr_from_user_mode_writes_only_the_apsr() {
    // MSR CPSR_fsxc, r0 with every bit set; MRS r1, CPSR
    let (mut cpu, mut mem) = a32(&[0xE12F_F000, 0xE10F_1000]);
    cpu.regs[0] = 0xFFFF_FFFF;
    assert_eq!(cpu.run(&mut mem, 2), Exit::BudgetSpent);
    assert_eq!(cpu.regs[1], 0xF80F_0010, "NZCVQ and GE set; mode stays User, T/E/A/I/F unchanged");
    assert!(!cpu.thumb && !cpu.big_endian);
    let (mut cpu, mut mem) = a32(&[0xE16F_F000]); // MSR SPSR_fsxc, r0
    assert_eq!(trap_kind(cpu.step(&mut mem)), TrapKind::Privileged);
}

#[test]
fn thumb_bl_prefix_alone_then_suffix() {
    // An ARMv6 BL prefix followed by something else executes on its own (LR = PC + 4 + (imm << 12)).
    let (mut cpu, mut mem) = t16(&[0xF001, 0x2000, 0xF800]);
    assert_eq!(cpu.step(&mut mem), Exit::BudgetSpent);
    assert_eq!(cpu.regs[14], CODE + 4 + 0x1000);
    assert_eq!(cpu.run(&mut mem, 2), Exit::BudgetSpent); // MOVS r0, #0; lone suffix
    assert_eq!(cpu.pc(), CODE + 4 + 0x1000);
    assert_eq!(cpu.regs[14], (CODE + 6) | 1);
}

#[test]
fn thumb_blx_pair_switches_to_arm_word_aligned() {
    let (mut cpu, mut mem) = t16(&[0x4600, 0xF000, 0xE802]); // MOV r0, r0; BLX +4 (from PC 0x1002+4)
    assert_eq!(cpu.run(&mut mem, 2), Exit::BudgetSpent);
    assert!(!cpu.thumb);
    assert_eq!(cpu.pc(), (CODE + 2 + 4 + 4) & !3);
    assert_eq!(cpu.regs[14], (CODE + 6) | 1);
}

#[test]
fn vfp_disabled_traps_cp10_cp11() {
    let mut cpu = Cpu::new(Config { vfp: false, ..Config::default() });
    let (_, mut mem) = a32(&[0xEE30_0A01]); // VADD.F32 s0, s0, s2
    cpu.regs[15] = CODE;
    assert_eq!(trap_kind(cpu.step(&mut mem)), TrapKind::Coprocessor { cp: 10 });
}
