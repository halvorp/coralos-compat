//! Branch semantics (PEM 4.2.3): BO/BI decoding, CTR decrement, link register, bctr, conditional blr.
//! The program in asm/branches.s is assembled by LLVM; every taken/not-taken pair is a negative control.
mod common;
use common::*;
use retro_ppc::{Cpu, Trap};

fn run_branches() -> Cpu {
    let (c, _, st) = run_words(programs::BRANCHES, |_, _| {});
    assert!(st.halted && st.trap.is_none(), "{st:?}");
    c
}

#[test]
fn counted_loops_and_ctr_decrement() {
    let c = run_branches();
    assert_eq!(c.gpr[3], 10, "bdnz ran five times");
    assert_eq!(c.gpr[20], 0, "and left CTR at 0 (falls through, not taken at 0)");
    assert_eq!(c.gpr[5], 0, "bdz taken when CTR hit 0: the li 99 was skipped");
    assert_eq!(c.gpr[13], 3, "bdnzt loops while CTR != 0 AND the CR bit is set");
    assert_eq!(c.gpr[14], 1, "CONTROL bdnzf with the bit set does not branch");
    assert_eq!(c.gpr[23], 2, "bdnzf still decremented CTR (3 -> 2)");
}

#[test]
fn conditional_branches_taken_and_not() {
    let c = run_branches();
    assert_eq!(c.gpr[6], 0, "beq taken");
    assert_eq!(c.gpr[7], 0, "bne not taken (CONTROL)");
}

#[test]
fn link_register_and_register_indirect_branches() {
    let c = run_branches();
    assert_eq!(c.gpr[8], 77, "bctrl reached the subroutine");
    assert_eq!(c.gpr[21] % 4, 0);
    assert_ne!(c.gpr[22], c.gpr[21], "bctrl produced its own LR");
    assert_eq!(c.gpr[10], 0, "bctr skipped the marker store");
    assert_eq!(c.gpr[11], 5, "and landed on the next instruction");
}

#[test]
fn conditional_blr() {
    let c = run_branches();
    assert_eq!((c.gpr[24], c.gpr[25]), (1, 2), "beqlr returned when EQ was set, fell through when clear");
}

#[test]
fn bl_sets_lr_to_the_next_instruction_and_b_does_not() {
    // bl +8 ; (skipped word) ; mflr: LR must be pc+4 of the bl.
    let bl = 0x4800_0009u32; // bl .+8  (opcode 18, LI=8, AA=0, LK=1)
    let b = 0x4800_0008u32;  // b  .+8
    let (c, _, _) = run_words(&[bl, 0, asm("mflr 3")], |_, _| {});
    assert_eq!(c.gpr[3], CODE + 4);
    let (c, _, _) = run_words(&[b, 0, asm("mflr 3")], |c, _| c.lr = EXIT);
    assert_eq!(c.gpr[3], EXIT, "CONTROL: plain b leaves LR alone");
}

#[test]
fn absolute_branch_aa_ignores_pc() {
    // ba 0x1008 ; (word that would fault) ; li 3,7
    let ba = 0x4800_0000u32 | 0x1008 | 2;
    let (c, _, st) = run_words(&[ba, 0, asm("li 3,7")], |_, _| {});
    assert!(st.trap.is_none(), "{st:?}");
    assert_eq!(c.gpr[3], 7);
}

#[test]
fn bcctr_that_decrements_ctr_is_a_named_invalid_form() {
    // bcctr 16,0 (BO=16: "decrement CTR, branch if CTR != 0") is architecturally invalid.
    let w = 0x4C00_0420u32 | 16 << 21;
    let (_, _, st) = run_words(&[w], |c, _| c.ctr = 0x1000);
    assert!(matches!(st.trap, Some(Trap::InvalidForm { pc: CODE, .. })), "{st:?}");
    // CONTROL: bctr (BO=20) is fine.
    let (_, _, st) = run_words(&[asm("bctr")], |c, _| c.ctr = EXIT);
    assert!(st.trap.is_none() && st.halted, "{st:?}");
}

#[test]
fn target_registers_are_aligned_down() {
    // blr to LR with low bits set: the low two bits are ignored (PEM: LR[0:29] || 0b00).
    let (_, _, st) = run_words(&[asm("blr")], |c, _| c.lr = EXIT | 3);
    assert!(st.halted && st.trap.is_none(), "{st:?}");
}
