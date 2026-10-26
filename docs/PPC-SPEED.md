# PowerPC guest speed: measurements

The plan is CoralOS `docs/consults/2026-10-24-ppc-recompiler.ruling.md` (Phase 0 measure, Phase 1 decoded-block
interpreter, later phases a JIT). This file holds the numbers, how to reproduce them, and what they say.

## The harness

`crates/heirloom-cpu-ppc-bench` (host-only, its own cargo workspace like `heirloom-cpu-ppc-oracle`; the root workspace excludes it):

    cd crates/heirloom-cpu-ppc-bench
    cargo run --release --bin ppc-bench -- --tiers 0 --reps 5        # baseline table
    cargo run --release --bin ppc-bench -- --census                  # opcode / block census, tier 0
    cargo run --release --bin ppc-bench -- --natives                 # time inside native callouts
    cargo run --release --bin ppc-bench -- --only warcraft --tiers 0,1

Each workload is run `--reps` times per tier; the MEDIAN elapsed time is reported, with instructions retired, native
callouts, guest instructions/s, a `[PPC-PROF ...]` line and a digest (FNV-1a of every register, the time base and all of
guest RAM at the end). A tier whose digest or instruction count differs from tier 0 on a workload is reported as a
DIVERGENCE and gets no speed number. Elapsed time is the run loop only (loading, instantiation and relocation excluded).

`macppc-run --prof [--tier T]` prints the same `[PPC-PROF insns= native_calls= elapsed_us= mips= tier=]` line for any
program (the counters live in `heirloom_cpu_ppc::Exec`, which every tier keeps; none costs anything per instruction).

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

The synthetic programs are `crates/heirloom-cpu-ppc/tests/asm/bench_*.s`, assembled by LLVM through `tools/ppc-regen.py` into
`tests/common/programs.rs`. Corpus files that are absent are SKIPPED with a line saying so; no game byte is in git.

## Baseline: tier 0 before this work (2026-10-24, build host, commit b5a1d72)

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

## Tier 1: decoded blocks (the default since this change)

`heirloom_cpu_ppc::Exec` with `Tier::Blocks` (`Exec::default()`); `Tier::Interp` is tier 0, kept as the reference;
`Tier::Lockstep` runs both. Selected at run time (`macppc-run --tier 0|1|lockstep`, `ppc-bench --tiers 0,1,lockstep`,
`HEIRLOOM_PPC_TIER` for the oracle).

What it does (crates/heirloom-cpu-ppc/src/t1*.rs):
- **Blocks** keyed by guest pc in a direct-mapped table (65,536 slots; a colliding block replaces the old one), the
  decoded instructions in one arena dropped whole past 1M instructions: a cache bounded by eviction, never a refusal.
  A block runs to the first branch (`sc` is always a block of its own: it calls the host), the end of its 4 KiB page,
  the instruction before a bound native address, or the instruction before one that cannot be fetched.
- **Decoded instructions** (`t1op.rs`): a handler id with registers and immediates extracted, for the ~70 forms the
  census shows hot (loads/stores incl. update and indexed forms, the integer ALU, compares, rotates, LR/CTR moves, all
  branch forms, FP loads/stores; FP arithmetic goes straight to tier 0's `fpu` handler). Only VALID forms are
  specialised (an update load with RA=0 or RA=RT, `bcctr` decrementing CTR, an SPR other than LR/CTR...); everything
  else is `Generic`, which runs tier 0's own `execute` on the word with pc/next/time base exactly as tier 0 has them.
- **Invalidation by generation** (`heirloom-mem-flat32` `watch.rs`): the cache marks each page it decodes from; the first
  write to a marked page (through `write_bytes`, `zero` or any sized store: the guest CPU, the native Toolbox,
  resource loads, BlockMove -- there is no other path, `ram` is private) or an `icbi` bumps the page's generation and
  unmarks it. A block is used only while its page's generation is the one it was decoded at. A store inside a block
  to the block's own page ends the block after the store, so the next instruction is fetched fresh (self-modifying
  code in one straight run). With nothing watched a write pays one branch.
- **Volatile pages**: a page whose blocks went stale 64 times runs on tier 0 from then on (decoding it again would
  cost more than interpreting it).
- **Natives**: the CPU's bound-native set has a version (process-unique per change); blocks are re-cut (cache
  dropped) only when the set really changes, so CPU context switches between equal sets cost an integer compare. The
  set is now shared copy-on-write (`Arc`): cloning a CPU (a thread context, a callback frame) went from 4.5 us to
  29 ns with 900 natives bound; both tiers gain from that.
- **Exactness**: at every exit pc, the time base (exact also before every Generic instruction, the only ones that
  read it), the trap and the trace ring are tier 0's. The budget is exact (a block is cut to the instructions left).

### Gates (all measured 2026-10-24)

1. **Differential oracle, every tier**: `heirloom-cpu-ppc-oracle` `cargo test --release` 9/9 (the family sweep now runs on
   the default tier AND explicitly on tier 0, tier 1 and lockstep); `ppc-fuzz 11` (374,000 cases) and the README's
   headline run `ppc-fuzz 11 20000 400000` (3,940,000 cases) under `HEIRLOOM_PPC_TIER=1` and `=lockstep`: **0
   discrepancy groups**, the same 3,297,378 agreeing outright as tier 0.
2. **Lockstep, block by block, zero divergence** (GPR, FPR, CR, LR, CTR, XER, FPSCR, pc, time base, reservation, the
   trap, every byte either run wrote; MSR is not modelled by this user-mode interpreter): Warcraft III startup 182,790
   blocks / 946,914 instructions / 567,309 bytes; the 50M census trace 12,771,628 blocks / 48,912,295 instructions /
   32.7M bytes; every Retro68 sample (FixedBenchmark, Raytracer, Raytracer2, HelloWorld, HelloWorldCpp, WDEFShell,
   Dialog) and every synthetic workload; every final digest equal to tier 0's. `macppc-run --tier lockstep` on the
   game: "no divergence".
3. **Planted negative controls** (`plant_stale_code_mutant`: blocks never invalidated): `tests/tier1.rs` --
   `bench_smc` (stw + dcbst/sync/icbi/isync patching the next instruction in the same block) and `smc_native` (a
   NATIVE writes the instruction, no icbi) both give r5 = 150 on tier 1 and a wrong r5 with the mutant; lockstep with
   the mutant reports the divergence in gpr[5] at the block; the oracle with the mutant goes RED at case 1. Also: a
   native bound inside an already decoded block is entered; traps mid-block (bus fault, unimplemented, sc, alignment)
   leave the same pc/time base/trace.
4. **Speed** (`ppc-bench --tiers 0,1 --reps 5`, median, load average 6-15):

| workload | tier 0 Mips | tier 1 Mips | speedup |
|---|---:|---:|---:|
| synth-int | 98.4 | 561.0 | 5.70x |
| synth-branch | 129.6 | 253.2 | 1.95x |
| synth-mem | 98.0 | 304.7 | 3.11x |
| synth-fp | 77.8 | 120.8 | 1.55x |
| synth-callback | 99.1 | 281.8 | 2.84x |
| synth-smc | 98.4 | 98.7 | 1.00x |
| warcraft3-roc-startup | 37.4 | **56.2** | 1.50x |
| warcraft3-roc-census | 16.9 | **20.4** | 1.20x |
| retro68-FixedBenchmark | 10.5 | 12.3 | 1.17x |
| retro68-Raytracer | 60.6 | 128.5 | 2.12x |
| retro68-Raytracer2 | 81.6 | 377.5 | 4.63x |
| retro68-HelloWorld | 10.2 | 11.1 | 1.09x |
| retro68-HelloWorldCpp | 10.0 | 11.1 | 1.11x |
| retro68-WDEFShell | 30.1 | 39.5 | 1.31x |

**Geomean 1.84x over 14 workloads, worst 1.00x** (Astra's floor: >= 1.5x, nothing regressing > 5%: met). Against
the pre-branch baseline above, Warcraft III: startup 32.4 -> 56.2M ips, census trace 12.0 -> 20.4M ips (part of
that is the cheaper CPU clone, which tier 0 gets too). **Argon's >= 22M ips**: met by the startup (56.2M), NOT by
the 50M census trace (20.4M).

### The bottleneck that remains (measured, not tuned around)

On the application workloads the CPU is no longer where the time goes:
- **warcraft3-roc-census: 70% of elapsed is inside native callouts**, 2.09 s of 3.25 s in `CFM/$CallbackReturn`
  (433,823 calls, 4.8 us each, down from 7.6 us): the thread switch while both threads idle around the StandardAlert.
  What is left of it is heirloom-macppc-engine `sched.rs` `raw_switch` re-binding EVERY glue stub into the incoming CPU
  (`for i in 0..self.glue.len() { cpu.bind_native(..) }`: ~900 set lookups, 6-9 us per full re-bind measured in
  isolation). Binding only the stubs interned since that task last ran would remove it; that is engine code, outside
  this change (the engine was touched only at the interpreter entry point).
- **warcraft3-roc-startup: 42% is one EraseRect** (11.8 ms, a whole-screen erase in the native QuickDraw).
  Guest code alone there runs ~50M ips on tier 0 and roughly 4x that on tier 1 (elapsed minus the separately
  measured native time: an estimate).
- The event-loop samples (HelloWorld, FixedBenchmark, WDEFShell) spend 15-60% in WaitNextEvent / TickCount /
  SystemTask / GetNextEvent at 0.2-0.6 us per call, called every 10-20 guest instructions.
- BlockZero / BlockMoveData are NOT hot in any trace (BlockZero 2,192 calls, 0.13 us each, 0.3 ms of the startup):
  no fast path was added for them, the profile does not justify one.
- On guest code, per-BLOCK cost dominates (mean dynamic block 3.8-5.3 instructions): synth-branch (1.9 per block)
  gains least. That is what chaining blocks (phase 5) or a JIT (phase 3) attacks.

### What phase 3 (the JIT) gets from this

- One entry point and switch: `Exec::resume` with a `Tier`; `Tier::Lockstep` already compares a faster tier against
  tier 0 block by block (registers, time base, reservation, trap, every written byte, journaled and rolled back
  through `GuestMem::journal_*`) -- a JIT tier plugs into the same comparator.
- Block discovery and boundaries (`t1cache.rs`), the decoded form as a translation IR (`t1op.rs`: handler id +
  fields, the Generic fallback for everything not specialised = a JIT's "precise exit to the interpreter"), and
  invalidation by page generation, which a JIT's translated blocks reuse unchanged (plus the volatile-page policy).
- The exactness contract written down and tested: pc/time base/trace at every exit, budget-exact cut, natives
  checked once per block, `sc` and natives always outside translated code.
- The workloads, digests and numbers here as its baseline, and the census to pick what to translate first.

## A Z83 form (not built)

The harness is host-only (std). To run it on the Z83 inside CoralOS it would need: the synthetic workloads as a
no_std runner (they only need `heirloom-cpu-ppc` + `heirloom-mem-flat32` + a monotonic clock: the CoralOS time syscall), the
corpus files on the eMMC datavol (they are not shippable: owner's copies, test input only), and a launch row for a
bench `.engine` that prints the `[PPC-PROF]` lines to the ring, graded with `z83_ring --grep PPC-PROF`. Nothing of
that is built here.
