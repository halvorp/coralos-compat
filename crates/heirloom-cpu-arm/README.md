# heirloom-cpu-arm

A user-mode ARMv6 interpreter (ARM1136/ARM1176, ARMv6K) for Heirloom's Symbian engine: the target is
S60 5th edition code such as Hero of Sparta. `#![no_std]`, no dependencies. The CPU knows nothing about
an OS or a memory layout: the host implements `Memory` and drives `Cpu::run(&mut mem, budget)` /
`Cpu::step(&mut mem)`.

```rust
let mut cpu = Cpu::new(Config::default());    // Unaligned::Armv6 (U=1, EKA2's setting), VFP on
cpu.regs[15] = entry;
match cpu.run(&mut mem, 100_000) {
    Exit::Svc(n) => { /* EKA2 exec call n; PC is already past the SVC */ }
    Exit::BudgetSpent => { /* time slice over */ }
    Exit::Wait => { /* WFI/WFE */ }
    Exit::Bkpt(imm) => { /* PC on the BKPT */ }
    Exit::Fault { pc, fault } => { /* the host's Memory refused an access; nothing was written */ }
    Exit::Trap(t) => { /* t.kind names why: Undefined, Unpredictable, Privileged, Unaligned, ... */ }
}
```

## What it executes

| Group | Instructions |
|---|---|
| Data processing | all 16 operations, immediate / immediate-shift / register-shift operands, every shifter carry rule, writes to PC (ARMv6 `BranchWritePC`) |
| Multiply | MUL, MLA, UMULL, UMLAL, SMULL, SMLAL, UMAAL; SMLA/SMUL/SMLAW/SMULW/SMLAL `<x><y>` |
| DSP / saturating | QADD, QSUB, QDADD, QDSUB, CLZ, SSAT, USAT, SSAT16, USAT16 (Q flag) |
| ARMv6 media | S/Q/SH/U/UQ/UH ADD16/ASX/SAX/SUB16/ADD8/SUB8 (GE flags), SEL, PKHBT/PKHTB, SXTB/SXTH/SXTB16/UXT\*, SXTA\*/UXTA\* (with rotation), REV/REV16/REVSH, USAD8/USADA8, SMLAD/SMLSD/SMUAD/SMUSD (X), SMLALD/SMLSLD, SMMLA/SMMLS/SMMUL (R) |
| Loads / stores | LDR/STR/LDRB/STRB (imm, scaled register, pre/post, writeback, T forms), LDRH/STRH/LDRSB/LDRSH, LDRD/STRD, LDM/STM IA/IB/DA/DB, SWP/SWPB, LDREX/STREX{B,H,D}, CLREX, PLD |
| Branch / status | B, BL, BX, BLX (imm and register), BXJ (as BX, Jazelle off), MRS, MSR (APSR fields only from user mode), SETEND, CPS (no effect in user mode), the NOP/YIELD/WFE/WFI/SEV hints, SVC, BKPT |
| Thumb (ARMv6, no Thumb-2) | the whole 16-bit set including the ARMv6 additions (extends, REV\*, SETEND, CPS) and the BL/BLX prefix + suffix pair (retired as one step when adjacent; a lone half executes as ARMv6 defines) |
| VFPv2 | VMLA/VMLS/VNMLA/VNMLS, VMUL/VNMUL, VADD/VSUB, VDIV, VMOV/VABS/VNEG/VSQRT, VCMP/VCMPE (also with #0), VCVT single<->double, int<->float, VCVT/VCVTR to int; VMOV core<->S, Dn[x], pairs; VMRS/VMSR (FPSID read, FPSCR, APSR_nzcv); VLDR/VSTR, VLDM/VSTM/VPUSH/VPOP (and FLDMX/FSTMX) |
| CP15 | TPIDRURW (read/write), TPIDRURO (read), the user-mode c7 barriers (ISB/DSB/DMB); anything else is `TrapKind::Cp15 { .. }` |

The floating point is exact IEEE 754 (`softfloat.rs`, no host floats): all four FPSCR rounding modes,
flush-to-zero with IDC, default NaN, ARM NaN propagation, tininess before rounding, every cumulative flag.
FPSCR.LEN/STRIDE short vectors execute element by element as on VFP11. An exception whose FPSCR trap
enable is set returns `TrapKind::VfpException` with nothing changed.

## Policies

- **Nothing panics on guest input.** UNDEFINED returns `TrapKind::Undefined` (this includes ARMv6T2 and
  ARMv7 encodings: MOVW/MOVT, bitfields, RBIT, MLS, SDIV, DMB, Thumb-2, CBZ/IT/NOP-hints, NEON, VFPv3).
- **UNPREDICTABLE is refused, not guessed**: `TrapKind::Unpredictable` (PC operands where the ARM ARM forbids
  them, RdHi == RdLo, writeback with Rn == Rt or Rn == PC, the ARMv6 P=0/W=1 extra loads, an STM whose
  stored base is not the lowest register, a loaded PC with bits [1:0] = 10, should-be-zero/one fields
  that are not). Exception returns, SPSR, SRS/RFE and SMC are `TrapKind::Privileged`.
- **A refused or faulting instruction changes nothing**: every handler checks and performs its memory
  accesses before writing a register, and the PC stays on the instruction.
- **Alignment** is a `Config` choice: `Unaligned::Armv6` (U=1, A=0: LDR/STR/LDRH/STRH/LDRSH unaligned
  allowed; LDM/STM, LDRD/STRD, SWP, VFP transfers word-aligned; exclusives naturally aligned),
  `Unaligned::Legacy` (U=0: rotated loads, aligned stores) or `Unaligned::Strict` (A=1).
- `Memory` is only called with naturally aligned 16- and 32-bit addresses; allowed unaligned accesses are
  composed from bytes. `CPSR.E` (SETEND BE) byte-swaps data accesses; instruction fetches stay
  little-endian (BE-8).

## Tests (`cargo test -p heirloom-cpu-arm`)

All expectations come from an independent model, Unicorn's ARM1176 (QEMU), except where noted.

- `rax_vectors`: rax's generated A32/T16 tests (`tests/generated/arm/{a32,t16}`, 46,696 test functions).
  Many of their assertions were generated from AArch64 semantics and are false for the A32/T16 encoding
  they test (`SMULL X0, W1, W2` on 0x9B227C20, which is a BL in A32), so each test is run on Unicorn
  first and ported only if all of its rax assertions hold there; the ported vector then checks the whole
  Unicorn state. After de-duplication: 12,583 vectors, 11,529 full-state checks, 636 no-panic smoke cases
  (rax asserts nothing for them), 418 refusals of UNPREDICTABLE encodings (llvm-mc flags them, or they
  are listed with the reason in `tests/data/unpredictable_reviewed.txt`). Per-group counts, including
  how many rax tests were dropped and why, are in `tests/data/rax_port_summary.txt`.
- `fuzz_vectors`: 6,900 random valid ARMv6 encodings with random operands, flags and FPSCR, 300 per
  instruction-group template (23 templates, A32, Thumb and VFP). The ignored `soak_directory` test runs
  a larger set generated outside the tree (115,000 vectors at 5,000 per template: all pass).
- `unit_ported`: rax's hand-written AArch32 executor tests and Thumb decoder tests, ported to execution
  checks (rax's T32 cases become checks of what those byte sequences do on a core without Thumb-2).
- `exits`, `vfp_hand`: the host contract (SVC, BKPT, WFI, budget, faults, traps, the three alignment
  modes, CP15, VFP off) and VFP cases Unicorn cannot judge (see below), with expected values from the
  ARMv6 ARM.

`tools/` regenerates the vectors (`pip install unicorn`; `port_rax_tests.py <rax checkout> tests/data`,
`fuzz_vectors.py tests/data 300`; `llvm-mc` is used to flag UNPREDICTABLE encodings).

Where Unicorn is wrong or more permissive than ARMv6 the vectors avoid or record it, and a hand test covers
the case: QEMU 5.0 advances a stride-2 short vector by 4 and does not vectorise double-precision
VMOV/VABS/VNEG/VSQRT; it does not enforce the U=1 alignment rule for LDM/STM/LDRD/VFP; it holds the FPSCR
trap enables RAZ; it executes the ARMv6T2 Thumb CBZ/CBNZ/IT/hints, exception returns in user mode and
many UNPREDICTABLE forms.

## Not done

- Thumb-2, NEON, VFPv3 (VMOV immediate, fixed-point and half-precision VCVT), Jazelle.
- Privileged modes, exception entry, banked registers, MMU: user mode only, by design.
- CP14 (debug) registers, which ARM11 user mode can read: `TrapKind::Coprocessor { cp: 14 }`.
- No decode cache or JIT: one decode per executed instruction.

## THIRD_PARTY: HexRaysSA/rax

Parts of this crate are adapted from [rax](https://github.com/HexRaysSA/rax), commit 526acfa
(`src/isa/arm/aarch32/{cpu,vfp}.rs`, `src/isa/arm/aarch32/instructions/{data,control,misc,memory,decode,coproc,fp_access}.rs`,
`src/isa/arm/decoder/aarch32/{mod,vfp}.rs`, `src/isa/arm/decoder/thumb.rs`, `src/backend/emulator/aarch32.rs`),
and its tests are ported as described above. Each adapted source file names its rax origin in a header.
`softfloat.rs` is not a port: rax's VFP layer delegates the arithmetic to its SMIR interpreter.

```
MIT License

Copyright (c) 2026 RAX contributors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
