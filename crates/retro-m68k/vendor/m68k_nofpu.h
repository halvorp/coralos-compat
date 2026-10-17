/* m68k_nofpu.h - coralos-compat (MIT, see LICENSE). Replaces Musashi's MAME-derived m68kfpu.c and m68kmmu.h,
 * which were included by m68kcpu.c. coralos-compat runs a 68020 with no FPU and no PMMU: Musashi already sends
 * every FPU opcode to the F-line trap below a 68030 and the PMMU opcode to it when the CPU has no PMMU, so these
 * entry points are unreachable at 68020 and behave exactly like that hardware (F-line exception) if reached.
 * A guest that needs an FPU or an MMU needs a real implementation here, not these stubs. */
void m68040_fpu_op0(void) { m68ki_exception_1111(); }
void m68040_fpu_op1(void) { m68ki_exception_1111(); }
void m68881_mmu_ops(void) { m68ki_exception_1111(); }
uint pmmu_translate_addr(uint addr_in) { return addr_in; }
