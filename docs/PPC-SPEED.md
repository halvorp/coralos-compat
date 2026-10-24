# PowerPC guest speed: measurements

The plan is CoralOS `docs/consults/2026-10-24-ppc-recompiler.ruling.md` (Phase 0 measure, Phase 1 decoded-block
interpreter, later phases a JIT). This file holds the numbers, how to reproduce them, and what they say.

## The harness

`crates/retro-ppc-bench` (host-only, its own cargo workspace like `retro-ppc-oracle`; the root workspace excludes it):

    cd crates/retro-ppc-bench
    cargo run --release --bin ppc-bench -- --tiers 0 --reps 5        # baseline table
    cargo run --release --bin ppc-bench -- --census                  # opcode / block census, tier 0
    cargo run --release --bin ppc-bench -- --natives                 # time inside native callouts
    cargo run --release --bin ppc-bench -- --only warcraft --tiers 0,1

Each workload is run `--reps` times per tier; the MEDIAN elapsed time is reported, with instructions retired, native
callouts, guest instructions/s, a `[PPC-PROF ...]` line and a digest (FNV-1a of every register, the time base and all of
guest RAM at the end). A tier whose digest or instruction count differs from tier 0 on a workload is reported as a
DIVERGENCE and gets no speed number. Elapsed time is the run loop only (loading, instantiation and relocation excluded).

`macppc-run --prof [--tier T]` prints the same `[PPC-PROF insns= native_calls= elapsed_us= mips= tier=]` line for any
program (the counters live in `retro_ppc::Exec`, which every tier keeps; none costs anything per instruction).

Workloads:

| name | what it is |
|---|---|
| synth-int | integer ALU mix (shifts, xor, mullw, rlwinm, record forms), 14M instructions |
| synth-branch | data-dependent conditional branches, bl/blr, bctrl |
| synth-mem | lwzu/stwu copy, lwz/lbz/lhz/stw walk |
| synth-fp | fmadd/fmul/fsub/fadds/fabs through lfd/stfd |
| synth-callback | a native BlockZero callout every 11 instructions |
| synth-smc | self-modifying code: stw + dcbst/sync/icbi/isync patches the next instruction every iteration |
| warcraft3-roc-startup | Warcraft III RoC (owner's disc, corpus only), first 950,000 instructions: the startup up to where its two threads start idling around its own "needs Mac OS 9" StandardAlert |
| warcraft3-roc-census | the same program as `corpus-census-mac.py` runs it: 50M instructions, idle 6 ticks |
| retro68-* | the Retro68 PPC samples in the corpus (20M instructions or to their stop) |

The synthetic programs are `crates/retro-ppc/tests/asm/bench_*.s`, assembled by LLVM through `tools/ppc-regen.py` into
`tests/common/programs.rs`. Corpus files that are absent are SKIPPED with a line saying so; no game byte is in git.

## Baseline: tier 0 (2026-10-24, build host)

Host: AMD Ryzen 9 8945HX, 32 threads; the box was busy (load average 14-33 from other CoralOS work during every run),
so single numbers move by up to ~40% (synth-callback 93 -> 52 Mips between two runs); compare tiers inside ONE run.

| workload | insns | natives | elapsed | guest Mips |
|---|---:|---:|---:|---:|
| synth-int | 14,000,006 | 1 | 147.0 ms | 95.3 |
| synth-branch | 12,750,010 | 1 | 102.8 ms | 124.0 |
| synth-mem | 12,872,004 | 1 | 135.1 ms | 95.3 |
| synth-fp | 10,000,008 | 1 | 165.2 ms | 60.5 |
| synth-callback | 5,500,007 | 500,001 | 59-105 ms | 52-93 |
| synth-smc | 8,000,011 | 1 | 84-140 ms | 57-96 |
| warcraft3-roc-startup | 950,000 | 3,086 | 29.3 ms | 32.4 |
| warcraft3-roc-census | 50,000,000 | 1,087,705 | 4,184 ms | 12.0 |
| retro68-FixedBenchmark | 20,000,000 | 962,611 | 1,951 ms | 10.3 |
| retro68-Raytracer | 20,000,000 | 92,887 | 330 ms | 60.6 |
| retro68-Raytracer2 | 20,000,000 | 169 | 241 ms | 83.1 |
| retro68-HelloWorld | 20,000,000 | 1,060,295 | 2,244 ms | 8.9 |
| retro68-HelloWorldCpp | 20,000,000 | 1,060,254 | 2,100 ms | 9.5 |
| retro68-WDEFShell | 20,000,000 | 1,818,172 | 719 ms | 27.8 |
| retro68-Dialog | 366 | 23 | (stops at its dialog) | - |

**The brief's "~4M guest instructions/s" is not what today's release build does**: the interpreter itself runs
60-125M guest instructions/s on straight guest code. What makes the application numbers small is the time inside
native callouts (`--natives`):

| workload | share of elapsed inside natives | top callout |
|---|---:|---|
| warcraft3-roc-census | 76% | `CFM/$CallbackReturn` 433,823 calls, 7.6 us each (the thread switch through the callback machine while both threads idle) |
| warcraft3-roc-startup | 43% | one `EraseRect` of 11.9 ms (a whole-screen erase) |
| retro68-WDEFShell | 61% | GetNextEvent + SystemTask, 0.3 us each, every 11 instructions (its event loop) |
| retro68-HelloWorld / FixedBenchmark | 15-18% | WaitNextEvent / TickCount / SystemTask (event loop) |
| retro68-Raytracer | 21% | `Line` 1.7 us each |

## Census (tier 0)

Warcraft III census trace: lwz 22.5%, addi 13.6%, bc 13.1%, stw 12.6%, cmpli 8.9%, b 8.5%, or 4.6%, bclr 3.6%,
cmpl 2.7%, mtspr 2.7%; 9,212 distinct blocks, mean DYNAMIC block length **3.83 instructions** (a block ends at a
taken branch, `sc`, a native call or a page end). The Retro68 event-loop samples look the same (lwz ~30%, mean block
~5.3); Raytracer2 is compute (rlwinm 26.6%, mean block 7.5). FP is < 10% everywhere except Raytracer.

Short blocks mean per-block dispatch cost matters as much as per-instruction cost: that shapes Phase 1 (cheap block
lookup, natives checked once per block, not per instruction) and is the case for chaining blocks in a later phase.

## A Z83 form (not built)

The harness is host-only (std). To run it on the Z83 inside CoralOS it would need: the synthetic workloads as a
no_std runner (they only need `retro-ppc` + `retro-guest-mem` + a monotonic clock: the CoralOS time syscall), the
corpus files on the eMMC datavol (they are not shippable: owner's copies, test input only), and a launch row for a
bench `.engine` that prints the `[PPC-PROF]` lines to the ring, graded with `z83_ring --grep PPC-PROF`. Nothing of
that is built here.
