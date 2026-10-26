# heirloom-cpu-ppc-oracle

Host-only differential fuzz of **heirloom-cpu-ppc** against **ppc-rs** (benletchford/ppc-rs 0.7.2, MIT, the `ppc` crate,
pinned `=0.7.2` from crates.io; builds anywhere with network access). `publish = false`, its own cargo workspace root, **not** a
dependency of anything and **not** part of the no_std workspace (ppc-rs is `std`, edition 2024, `rustc_apfloat`).
The root `Cargo.toml` `exclude`s it, so run it from here:

    cargo test                      # debug: ~14 s
    cargo test --release
    cargo run --release --bin ppc-fuzz  [seed] [cases-per-family] [sequences]   # default 1 2000 20000
    cargo run --release --bin ppc-gaps                  # opcode-space sweep: who refuses what
    cargo run --release --bin ppc-corpus > corpus/regressions.txt   # rebuild the corpus

## What it does

* **177 instruction families** (`src/table.rs`): every integer, rotate/shift, compare, all branch forms (b/bc/bclr/bcctr,
  every BO/BI, AA, LK), CR logic, mfcr/mtcrf/mcrxr, mfspr/mtspr/mftb, trap words, `sc`, all integer loads and stores
  (update, indexed, byte-reversed, lmw/stmw, lswi/lswx/stswi/stswx, lwarx/stwcx., dcbz and the cache/ordering hints),
  FP loads/stores (lfs/lfd/stfs/stfd, update and indexed, stfiwx), FP single (op 59: fadds … fmadds family, fres, fsqrts),
  FP double (op 63: fadd … fnmsub, fsel, frsqrte, fsqrt, frsp, fctiw, fctiwz, fcmpu/o, fmr/fneg/fabs/fnabs) and
  mffs/mtfsf/mtfsfi/mtfsb0/mtfsb1/mcrfs. Reserved bits are generated zero; OE, Rc, LK, AA are random.
* **Random state**: GPRs from corner-weighted values, FPRs from a table of special doubles (±0, ±Inf, SNaN, QNaN with
  payload, denormals, values around the single-precision exponent edges and integer boundaries, exact singles), random
  CR/XER/LR/CTR/FPSCR (consistent VX/FEX), a 8 KiB random data window, base registers aimed into the window (10 % wild
  or straddling the end of RAM).
* **Lockstep**: both machines execute one instruction at a time. After each step GPR/FPR bits/CR/XER/LR/CTR/FPSCR/pc/
  time base/all 16 KiB of RAM and the trap class are compared. Single-instruction cases and 2-8 instruction random
  sequences (including lwarx/stwcx. pairs); a store may patch the next instruction, which is then executed as patched.
* Seeded (`splitmix64`/`xorshift64*`), deterministic, no dependencies. `corpus/regressions.txt` has 231 cases (one
  agreeing case per family, six for the defects found, three per named exclusion), each with its complete starting
  state and the verdict it must keep.

## Numbers (release run, seed 11 plus the triage seeds)

3 940 000 cases in the headline run (177 × 20 000 single-instruction + 400 000 sequences), 3 293 918 agreeing outright,
the rest compared under a named relaxation (below), **0 discrepancies**; six further seeds of 591 000 cases and the
triage runs likewise. Mutation controls (a wrong CA in sraw, an inverted BO\[2\], a missing byte swap in lhbrx, a dropped
negation in fnmsub, each injected into heirloom-cpu-ppc) are caught within 74 000 cases.

## Discrepancies found, and who was right

| Finding | Verdict | Action |
|---|---|---|
| `stfs` rounds a double to nearest-even (heirloom) / per FPSCR[RN] (ppc-rs) | **both wrong.** PEM `stfs` pseudocode: `WORD[2:31] <- FRS[5:34]` when `FRS[1:11] > 896` (truncation, RN not consulted), denormalise-and-truncate for `874..896`, undefined below. (Power ISA Book I 4.6.3 has the same pseudocode.) | heirloom-cpu-ppc fixed (`fpmem.rs`); ppc-rs excluded by model (`ppcrs-stfs-rounds`) |
| `fmul`/`fmadd`: ∞×0 with a NaN addend: heirloom skipped VXIMZ | ppc-rs right (the ⊗ in the PEM pseudocode flags ∞×0 whatever the addend) | heirloom-cpu-ppc fixed (`fpu.rs`) |
| `fdiv`/`fres`/`frsqrte` with FPSCR[ZE]=1 wrote the infinity | ppc-rs right (PEM 3.3.6: enabled zero-divide leaves frD unchanged) | heirloom-cpu-ppc fixed |
| Rc=1 FP instruction whose write was suppressed (VE/ZE) left CR1 stale | ppc-rs right (CR1 is always loaded from FPSCR) | heirloom-cpu-ppc fixed (arith, frsp, fctiw) |
| `lfs` of a signalling NaN: heirloom keeps it, ppc-rs quiets it | heirloom right (PEM `lfs` conversion copies the fraction of an Inf/NaN unchanged; a load never touches FPSCR) | test `oracle_fixes.rs`; ppc-rs excluded (`ppcrs-lfs-quiets-snan`) |
| NaN operand selection: ppc-rs prefers a signalling NaN, flips the sign of a NaN through fmsub/fnmsub's negation, drops the payload of fsqrt/frsqrte | heirloom right (first NaN among frA, frB, frC, quieted, sign and payload kept) | excluded only when heirloom equals the model (`ppcrs-nan-selection`) |
| `fctiw`/`fctiwz` of a NaN: ppc-rs 0 | heirloom right (0x80000000, VXCVI) | `ppcrs-fctiw-nan` |
| `frsp` of a value below the single denormal range: ppc-rs returns garbage | heirloom right (rounds to ±0 / min denormal) | `ppcrs-frsp-deep-underflow` |
| FPRF[C] of a single result that is denormal in single | heirloom classifies in single (as the 750 does), ppc-rs as a double | `ppcrs-fprf-single-denormal` |

## Named exclusions (every one counted by the run)

Where the architecture leaves a result undefined, heirloom-cpu-ppc deliberately does not model something (lib.rs "Deliberate
deviations"), or ppc-rs is wrong. Nothing is excluded because "they differ".

| Name | Why |
|---|---|
| `div-undefined-result` | PEM divw/divwu: x/0 and 0x80000000/-1 leave rD and CR0[LT,GT,EQ] undefined (OV is defined and is compared) |
| `fp-int-result-high-word-undefined` | PEM fctiw/fctiwz/mffs: FRT[0:31] undefined (heirloom stores 0xFFF80000, as the 750 does) |
| `invalid-form-lsw-overlap` (and `invalid-form-*`) | PEM invalid instruction forms (lswi/lswx RA or RB in the loaded range, lmw RA in range, update forms with RA=0 or RA=RT, stwcx. Rc=0, bcctr decrementing CTR): boundedly undefined, heirloom refuses by name |
| `misaligned-multiple-or-reserved` | lmw/stmw/lwarx/stwcx. need a word-aligned EA: alignment exception OR boundedly undefined (heirloom raises it; ppc-rs under `EmulateData` does not) |
| `reserved-bits-set-ignored-by-heirloom` | an encoding with a reserved bit set is an invalid form; ppc-rs refuses it, heirloom ignores the bit (reachable only when a store patches code) |
| `mfspr-time-base` | user code reads the time base with `mftb`; `mfspr` of SPR 268/269 is outside the architected user SPR set (heirloom allows it, ppc-rs refuses) |
| `xer-reserved-bits-read` | XER[3:24] reserved: `mfspr XER` reads them as 0 on heirloom-cpu-ppc, ppc-rs keeps what `mtspr` wrote |
| `single-op-nonsingle-operand` | PEM: an op-59 instruction with an operand not exactly representable in single is undefined |
| `heirloom-untracked-fpscr-xx-ux-fi-fr` | **heirloom-cpu-ppc does not track FPSCR XX, UX, FI, FR** (lib.rs); FX/FEX/CR1 derived from them are masked only when ppc-rs newly set or enabled one of them |
| `heirloom-rn-directed-rounding` | FP arithmetic under FPSCR[RN] != 0 is a named `Unimplemented` refusal in heirloom (fctiw honours RN and is compared) |
| `heirloom-unwrapped-result-ox-ux-enabled` | enabled overflow/underflow: the architecture writes the exponent-wrapped result, heirloom writes the unwrapped one |
| `fpscr-ni-non-ieee-mode` | FPSCR[NI] is ignored by heirloom-cpu-ppc |
| `stfs-below-single-denormal-undefined` | PEM stfs of a magnitude below the single denormal range is undefined (heirloom stores a signed zero) |
| `ppcrs-*` | the ppc-rs defects in the table above; each is applied only when heirloom matches a model (`src/models.rs`) of the PEM rule: stfs (arithmetic: clear-and-convert / scale-and-truncate, not the shift amounts of the implementation), lfs (every lfs-family load is checked against the load conversion; only a loaded SNaN is relaxed, in bit 51 only), FPRF of a single denormal (C plus FG/FL), NaN selection, fctiw of a NaN |
| `heirloom-unwrapped-result-ox-ux-enabled` | narrow: OX needs an infinity from finite operands that is not a divide by zero (ZX); UX needs a tiny non-zero result or a zero whose exact result is non-zero (0 + -0, 0 * x and cancelling sums are exact and compared) |

## Coverage gaps (`ppc-gaps`, whole opcode space)

heirloom-cpu-ppc executes every encoding ppc-rs executes except FP arithmetic under RN != 0. The reverse list is only
ppc-rs refusing reserved-bit and invalid forms that heirloom ignores. ppc-rs has no AltiVec, paired singles or supervisor
state either.

## Remaining named gaps in heirloom-cpu-ppc

FPSCR XX/UX/FI/FR tracking; exponent-wrapped results under enabled OE/UE; FP arithmetic under directed rounding; NI mode.

## Sequences keep going after an exclusion

An excluded step (one that is accounted for by a name above) does not end the case: the oracle machine adopts heirloom-cpu-ppc's
state (`Pair::resync_oracle`) and the remaining instructions are still compared. A sequence stops only at an excluded
trap, a pc mismatch, or lwarx/stwcx. (reservation state cannot be copied).
