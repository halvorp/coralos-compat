//! HexRaysSA/rax's hand-written AArch32 executor tests (src/isa/arm/aarch32/instructions/tests.rs) and
//! Thumb decoder tests (src/isa/arm/decoder/thumb/tests.rs), commit 526acfa, ported to this crate's API.
//! rax checks the decoded mnemonic; here each encoding is executed and its ARMv6 effect checked. rax's
//! Thumb-2 (T32) cases become checks of what those byte sequences do on an ARMv6 core, which has no
//! Thumb-2: a BL/BLX half, or an Undefined trap.

mod common;
use common::{a32, peek32, t16, trap_kind, CODE};
use heirloom_cpu_arm::{Exit, TrapKind};

const OK: Exit = Exit::BudgetSpent;

#[test]
fn add_immediate() {
    let (mut cpu, mut mem) = a32(&[0xE281_0032]); // ADD r0, r1, #50
    cpu.regs[1] = 100;
    assert_eq!(cpu.step(&mut mem), OK);
    assert_eq!(cpu.regs[0], 150);
}

#[test]
fn adds_sets_flags() {
    let (mut cpu, mut mem) = a32(&[0xE291_0001]); // ADDS r0, r1, #1
    cpu.regs[1] = 0xFFFF_FFFF;
    assert_eq!(cpu.step(&mut mem), OK);
    assert_eq!(cpu.regs[0], 0);
    assert!(cpu.z && cpu.c && !cpu.n && !cpu.v);
}

#[test]
fn sub_immediate() {
    let (mut cpu, mut mem) = a32(&[0xE241_001E]); // SUB r0, r1, #30
    cpu.regs[1] = 100;
    assert_eq!(cpu.step(&mut mem), OK);
    assert_eq!(cpu.regs[0], 70);
}

#[test]
fn mov_immediate() {
    let (mut cpu, mut mem) = a32(&[0xE3A0_00FF]);
    assert_eq!(cpu.step(&mut mem), OK);
    assert_eq!(cpu.regs[0], 0xFF);
}

#[test]
fn cmp_sets_flags() {
    let (mut cpu, mut mem) = a32(&[0xE350_0032]); // CMP r0, #50
    cpu.regs[0] = 50;
    assert_eq!(cpu.step(&mut mem), OK);
    assert!(cpu.z && cpu.c);
}

#[test]
fn branch() {
    let (mut cpu, mut mem) = a32(&[0xEA00_0040]); // B +0x100
    assert_eq!(cpu.step(&mut mem), OK);
    assert_eq!(cpu.pc(), CODE + 8 + 0x100);
}

#[test]
fn a32_blx_immediate_preserves_halfword_target() {
    // rax runs this at 0xC016_0864 (target 0xC015_F2B2, i.e. PC + 8 - 0x15BA); same encoding at CODE.
    let (mut cpu, mut mem) = a32(&[0xFBFF_FA91]);
    assert_eq!(cpu.step(&mut mem), OK);
    assert_eq!(cpu.pc(), (CODE + 8).wrapping_sub(0x15BA), "H supplies bit 1 of the target");
    assert_eq!(cpu.regs[14], CODE + 4);
    assert!(cpu.thumb);
}

#[test]
fn user_mode_cp15_system_registers_trap_without_side_effects() {
    // MCR/MRC SCTLR (c1, c0, 0) from user mode: refused, registers untouched.
    let (mut cpu, mut mem) = a32(&[0xEE01_0F10, 0xEE11_1F10]);
    cpu.regs[1] = 0xDEAD_BEEF;
    let kind = trap_kind(cpu.step(&mut mem));
    assert_eq!(kind, TrapKind::Cp15 { read: false, opc1: 0, crn: 1, crm: 0, opc2: 0 });
    cpu.regs[15] += 4;
    assert!(matches!(trap_kind(cpu.step(&mut mem)), TrapKind::Cp15 { read: true, crn: 1, .. }));
    assert_eq!(cpu.regs[1], 0xDEAD_BEEF);
}

#[test]
fn pl0_cp15_reaches_only_the_thread_ids_and_the_barriers() {
    let (mut cpu, mut mem) = a32(&[
        0xEE0D_0F50, // MCR p15, 0, r0, c13, c0, 2   TPIDRURW = r0
        0xEE1D_1F50, // MRC p15, 0, r1, c13, c0, 2
        0xEE1D_2F70, // MRC p15, 0, r2, c13, c0, 3   TPIDRURO
        0xEE07_0F9A, // DSB (c7, c10, 4)
        0xEE07_0FBA, // DMB (c7, c10, 5)
        0xEE07_0F95, // ISB (c7, c5, 4)
        0xEE0D_0F70, // MCR TPIDRURO: read-only from user mode
    ]);
    cpu.tpidruro = 0x1111_2222;
    cpu.regs[0] = 0x3333_4444;
    assert_eq!(cpu.run(&mut mem, 6), OK);
    assert_eq!((cpu.tpidrurw, cpu.regs[1], cpu.regs[2]), (0x3333_4444, 0x3333_4444, 0x1111_2222));
    assert!(matches!(trap_kind(cpu.step(&mut mem)), TrapKind::Cp15 { read: false, crn: 13, opc2: 3, .. }));
    // TPIDRPRW, another opc1, and MRC2 are refused too.
    for word in [0xEE1D_3F90u32, 0xEE3D_3F70, 0xFE1D_3F70] {
        let (mut cpu, mut mem) = a32(&[word]);
        assert!(matches!(cpu.step(&mut mem), Exit::Trap(_)), "{word:#010x}");
        assert_eq!(cpu.regs[3], 0);
    }
}

#[test]
fn user_mode_rfe_and_srs_are_refused_without_side_effects() {
    let (mut cpu, mut mem) = a32(&[0xF8B0_0A00]); // RFEIA r0!
    cpu.regs[0] = 0x200;
    mem.poke(0x200, &0x1234_5678u32.to_le_bytes());
    assert_eq!(trap_kind(cpu.step(&mut mem)), TrapKind::Privileged);
    assert_eq!((cpu.regs[0], cpu.pc()), (0x200, CODE));
    let (mut cpu, mut mem) = a32(&[0xF8ED_0513]); // SRSIA sp!, #svc
    cpu.regs[13] = 0x200;
    assert_eq!(trap_kind(cpu.step(&mut mem)), TrapKind::Privileged);
    assert_eq!(cpu.regs[13], 0x200);
    assert!(mem.writes.is_empty());
}

#[test]
fn ldr_str() {
    let (mut cpu, mut mem) = a32(&[0xE591_0000, 0xE581_2004]); // LDR r0, [r1]; STR r2, [r1, #4]
    mem.poke(0x100, &0xDEAD_BEEFu32.to_le_bytes());
    cpu.regs[1] = 0x100;
    cpu.regs[2] = 0x0BAD_F00D;
    assert_eq!(cpu.run(&mut mem, 2), OK);
    assert_eq!(cpu.regs[0], 0xDEAD_BEEF);
    assert_eq!(peek32(&mem, 0x104), 0x0BAD_F00D);
}

#[test]
fn mul() {
    let (mut cpu, mut mem) = a32(&[0xE000_0291]); // MUL r0, r1, r2
    cpu.regs[1] = 7;
    cpu.regs[2] = 6;
    assert_eq!(cpu.step(&mut mem), OK);
    assert_eq!(cpu.regs[0], 42);
}

#[test]
fn condition_ne_not_taken() {
    let (mut cpu, mut mem) = a32(&[0x13A0_0001]); // MOVNE r0, #1
    cpu.z = true;
    assert_eq!(cpu.step(&mut mem), OK);
    assert_eq!((cpu.regs[0], cpu.pc()), (0, CODE + 4));
}

#[test]
fn svc_returns_its_immediate() {
    let (mut cpu, mut mem) = a32(&[0xEF00_007B]);
    assert_eq!(cpu.step(&mut mem), Exit::Svc(123));
    assert_eq!(cpu.pc(), CODE + 4, "PC is past the SVC");
}

#[test]
fn ldrex_strex() {
    let (mut cpu, mut mem) = a32(&[0xE191_0F9F, 0xE181_2F93]); // LDREX r0, [r1]; STREX r2, r3, [r1]
    mem.poke(0x100, &0x1234_5678u32.to_le_bytes());
    cpu.regs[1] = 0x100;
    cpu.regs[3] = 0xDEAD_BEEF;
    assert_eq!(cpu.run(&mut mem, 2), OK);
    assert_eq!((cpu.regs[0], cpu.regs[2]), (0x1234_5678, 0));
    assert_eq!(peek32(&mem, 0x100), 0xDEAD_BEEF);
}

#[test]
fn strex_fails_without_ldrex() {
    let (mut cpu, mut mem) = a32(&[0xE181_2F93]);
    mem.poke(0x100, &0x1234_5678u32.to_le_bytes());
    cpu.regs[1] = 0x100;
    cpu.regs[3] = 0xDEAD_BEEF;
    assert_eq!(cpu.step(&mut mem), OK);
    assert_eq!(cpu.regs[2], 1);
    assert_eq!(peek32(&mem, 0x100), 0x1234_5678);
}

#[test]
fn armv7_only_encodings_are_undefined() {
    // SDIV, MOVW, MOVT, BFI, UBFX, RBIT, MLS, DMB (A32) are ARMv6T2/ARMv7.
    for word in [0xE710_F211u32, 0xE300_0001, 0xE340_0001, 0xE7C7_0011, 0xE7E7_0051, 0xE6FF_0F31, 0xE060_3291,
        0xF57F_F05F]
    {
        let (mut cpu, mut mem) = a32(&[word]);
        assert_eq!(trap_kind(cpu.step(&mut mem)), TrapKind::Undefined, "{word:#010x}");
        assert_eq!(cpu.pc(), CODE);
    }
}

// ---- Thumb (rax decoder/thumb/tests.rs) ----

#[test]
fn t16_mov_r0_r0_and_push_lr() {
    let (mut cpu, mut mem) = t16(&[0x4600, 0xB500]); // MOV r0, r0; PUSH {lr}
    cpu.regs[13] = 0x800;
    cpu.regs[14] = 0x1234;
    assert_eq!(cpu.run(&mut mem, 2), OK);
    assert_eq!((cpu.regs[13], peek32(&mem, 0x7FC)), (0x7FC, 0x1234));
}

#[test]
fn t16_nop_hint_is_armv6t2() {
    let (mut cpu, mut mem) = t16(&[0xBF00]);
    assert_eq!(trap_kind(cpu.step(&mut mem)), TrapKind::Undefined);
}

#[test]
fn t16_movs_imm() {
    let (mut cpu, mut mem) = t16(&[0x2042]);
    cpu.n = true;
    cpu.z = true;
    assert_eq!(cpu.step(&mut mem), OK);
    assert_eq!(cpu.regs[0], 0x42);
    assert!(!cpu.n && !cpu.z);
}

#[test]
fn t16_adds_reg() {
    let (mut cpu, mut mem) = t16(&[0x1888]); // ADDS r0, r1, r2
    cpu.regs[1] = 0x7FFF_FFFF;
    cpu.regs[2] = 1;
    assert_eq!(cpu.step(&mut mem), OK);
    assert_eq!(cpu.regs[0], 0x8000_0000);
    assert!(cpu.n && cpu.v && !cpu.c && !cpu.z);
}

#[test]
fn t16_ldr_imm() {
    let (mut cpu, mut mem) = t16(&[0x6808]); // LDR r0, [r1]
    mem.poke(0x200, &0xCAFE_F00Du32.to_le_bytes());
    cpu.regs[1] = 0x200;
    assert_eq!(cpu.step(&mut mem), OK);
    assert_eq!(cpu.regs[0], 0xCAFE_F00D);
}

#[test]
fn t16_push_pop_pc_interworks() {
    let (mut cpu, mut mem) = t16(&[0xB510, 0xBD10]); // PUSH {r4, lr}; POP {r4, pc}
    cpu.regs[13] = 0x800;
    cpu.regs[4] = 0x44;
    cpu.regs[14] = 0x2000; // an ARM-state return address
    assert_eq!(cpu.run(&mut mem, 2), OK);
    assert_eq!((cpu.regs[4], cpu.regs[13], cpu.pc(), cpu.thumb), (0x44, 0x800, 0x2000, false));
}

#[test]
fn t16_bx_lr() {
    let (mut cpu, mut mem) = t16(&[0x4770]);
    cpu.regs[14] = 0x3001;
    assert_eq!(cpu.step(&mut mem), OK);
    assert_eq!((cpu.pc(), cpu.thumb), (0x3000, true));
}

#[test]
fn t16_b_and_beq() {
    let (mut cpu, mut mem) = t16(&[0xE004]); // B +8 (from PC+4)
    assert_eq!(cpu.step(&mut mem), OK);
    assert_eq!(cpu.pc(), CODE + 4 + 8);
    let (mut cpu, mut mem) = t16(&[0xD004]); // BEQ, Z clear: falls through
    assert_eq!(cpu.step(&mut mem), OK);
    assert_eq!(cpu.pc(), CODE + 2);
    let (mut cpu, mut mem) = t16(&[0xD004]);
    cpu.z = true;
    assert_eq!(cpu.step(&mut mem), OK);
    assert_eq!(cpu.pc(), CODE + 12);
}

#[test]
fn t16_cbz_cbnz_are_armv6t2() {
    for hw in [0xB110u16, 0xB910] {
        let (mut cpu, mut mem) = t16(&[hw]);
        assert_eq!(trap_kind(cpu.step(&mut mem)), TrapKind::Undefined);
    }
}

#[test]
fn t16_cmp_imm_lsl_svc() {
    let (mut cpu, mut mem) = t16(&[0x2800, 0x0108, 0xDF00]); // CMP r0, #0; LSLS r0, r1, #4; SVC #0
    cpu.regs[1] = 0x1000_0001;
    assert_eq!(cpu.run(&mut mem, 3), Exit::Svc(0));
    assert_eq!(cpu.regs[0], 0x10);
    assert!(cpu.c, "LSLS #4 shifted bit 28 out into C");
    assert_eq!(cpu.pc(), CODE + 6);
}

#[test]
fn t32_bl_pair_retires_as_one_step() {
    let (mut cpu, mut mem) = t16(&[0xF000, 0xF880]); // BL +0x100
    assert_eq!(cpu.step(&mut mem), OK);
    assert_eq!(cpu.pc(), CODE + 4 + 0x100);
    assert_eq!(cpu.regs[14], (CODE + 4) | 1);
}

#[test]
fn t32_thumb2_sequences_do_what_armv6_does_with_them() {
    // MOV.W r0, #1 (f04f 0001): on ARMv6 the first halfword is a BL prefix, the second LSLS r1, r0, #0.
    let (mut cpu, mut mem) = t16(&[0xF04F, 0x0001]);
    assert_eq!(cpu.step(&mut mem), OK);
    assert_eq!((cpu.regs[14], cpu.regs[0], cpu.pc()), ((CODE + 4).wrapping_add(0x4F << 12), 0, CODE + 2));
    // ADD.W r0, r1, #1 (f101 0001): a BL prefix again.
    let (mut cpu, mut mem) = t16(&[0xF101, 0x0001]);
    assert_eq!(cpu.step(&mut mem), OK);
    assert_eq!(cpu.regs[14], (CODE + 4).wrapping_add(0x101 << 12));
    // LDR.W r0, [r1] (f8d1 0000): a lone BL suffix, PC = LR + 0xD1 * 2.
    let (mut cpu, mut mem) = t16(&[0xF8D1, 0x0000]);
    cpu.regs[14] = 0x4000;
    assert_eq!(cpu.step(&mut mem), OK);
    assert_eq!((cpu.pc(), cpu.regs[14]), (0x4000 + 0x1A2, (CODE + 2) | 1));
    // PUSH.W {r4-r11, lr} (e92d 4ff0): a BLX suffix with bit 0 set is UNDEFINED.
    let (mut cpu, mut mem) = t16(&[0xE92D, 0x4FF0]);
    assert_eq!(trap_kind(cpu.step(&mut mem)), TrapKind::Undefined);
}
