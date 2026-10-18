# macppc plumbing (2026-10-18): everything around the PowerPC CPU that does not need the CPU

Three pieces, none of which executes a PowerPC instruction. The interpreter (`retro-ppc`, another lane) plugs into the
contract in `crates/macppc-engine/src/host.rs`.

## retro-pef (no_std)
PEF container: header, section headers/names, section images (pattern-initialised data expanded with all five pidata
opcodes), the loader section (imported libraries/symbols with class and weak flags, relocation headers, the export hash
table with lookup through the hash, main/init/term), every relocation opcode decoded and interpreted
(BySectDWithSkip, BySectC/D, TVector12/8, VTable8, ImportRun, SmByImport/SetSectC/SetSectD/BySection, IncrPosition,
SmRepeat, SetPosition, LgByImport, LgRepeat, LgSetOrBySection), and a writer for fixtures. Verified against a real
CodeWarrior PEF (SheepShaver `ethernet.ndrv`: its hash words match our hash function, its 126 relocations land inside
their sections). Executor's loader asserts on SmRepeat/LgRepeat/LgSetOrBySection; these are implemented here from the
spec (repeat = the preceding `blockCount + 1` INSTRUCTIONS, `repeatCount + 1` more times; no real fragment in the
corpus exercises them, so they are tested synthetically only).

## The Mixed Mode seam (mac68k-engine `mixed.rs`)
A Toolbox handler's inputs were already values (`Call.args`, `Regs`); what was missing was a way to fill them that is not
the 68k stack/registers. `Mac::pascal_call(opcode, values)` and `Mac::os_call(opcode, OsSig, values)` run THE SAME handler
function the A-line bridge runs, and `finish_pascal`/`finish_os` apply the outcome (log line or named refusal) for both
callers. `OS_SIGS` maps the Memory Manager's C prototypes onto the register contracts. `retro_m68k::Cpu::detached()`
lets a handler run for another processor without touching Musashi's global state. 68k-only handlers (`LoadSeg`) are
listed by name in `macppc-engine/src/symbols.rs` and bind as NEXT-NEEDED.

## macppc-engine
`MacPpc::instantiate(&Pef)`: sections placed in the shared guest heap, imports bound by SYMBOL NAME (natives, `qd`, OS
routines, Toolbox traps), relocations applied, entry TVectors read. Every imported function is a 16-byte glue cell
`{TVector{stub,0}, blr}`; the CPU asks `PpcHost::glue_symbol_at(pc)` and calls `PpcHost::glue_call(symbol, &mut regs)`.
Results: INTEGER sign-extended, Boolean zero-extended. `WaitNextEvent`/`GetNextEvent` poll (sleep forced to 0) because a
PowerPC caller cannot be suspended through the 68k run loop. The Code Fragment Manager calls an application makes
(`GetSharedLibrary`, `FindSymbol`, `CloseConnection`) are native. Weak missing imports relocate to 0; strong missing
functions are TVectors that refuse by name when called (`--strict`: refuse at load); strong missing data points outside
guest RAM.

Run: `cargo run -p macppc-engine --bin macppc-run -- [--census] [--strict] [--data FORK] (--fixture | <PEF | app>)`.
An application file is MacBinary, or an AppleDouble header file (`%name`/`._name`) with its data fork beside it; the
`cfrg` 0 resource names the `pwpc` application fragment.

## Census (imports bound / NEXT-NEEDED; there is no CPU to run to the first missing call yet)
| fragment | before | after |
|---|---|---|
| SheepShaver `ethernet.ndrv` (6 InterfaceLib imports) | no PPC engine | 6 bound, 0 NEXT-NEEDED |
| built-in startup app (19 imports: Init*, NewHandle, GetResource, NewWindow, WaitNextEvent, qd, MoveTo, DrawString, SysBeep ...) | no PPC engine | 16 bound, NEXT-NEEDED MoveTo, DrawString, SysBeep (QuickDraw drawing is the qd lane; they bind automatically when it registers them) |

## Next
1. The CPU lane: call `glue_symbol_at` per branch target, `glue_call` on a hit, treat `Return` as `blr`.
2. Float arguments (MathLib, `Fixed`/`Fract` helpers) need f1..f13 in the glue contract.
3. Imports from other fragments (an app's own shared libraries) via `instantiate` of a second PEF and export lookup.
4. UPP/routine descriptors (68k <-> PPC callbacks both ways) once a CPU exists to hand control between the two.
5. `WaitNextEvent` with a non-zero sleep is a poll today (the run loop cannot suspend a PowerPC caller), so an idle PPC event loop busy-spins the host; the call logs the ignored sleep. Needs an idle outcome in `GlueOutcome` once the CPU lane exists.
6. Bindings are keyed by symbol and only honoured for `InterfaceLib` (other libraries are Unbound by name); a second provided library needs a per-library binding table.
