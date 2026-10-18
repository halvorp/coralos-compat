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
Results: INTEGER sign-extended, Boolean zero-extended. (Event calls: see "macppc RUNS" below.) The Code Fragment Manager calls an application makes
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
1. DONE: the CPU lane (retro-ppc) calls `glue_symbol_at` per native hook and `glue_call` on a hit.
2. Float arguments (MathLib, `Fixed`/`Fract` helpers) need f1..f13 in the glue contract.
3. Imports from other fragments (an app's own shared libraries) via `instantiate` of a second PEF and export lookup.
4. UPP/routine descriptors (68k <-> PPC callbacks both ways) once a CPU exists to hand control between the two.
5. DONE (see "macppc RUNS"): `WaitNextEvent` with a sleep parks the CPU (`GlueOutcome::Idle`).
6. DONE: bindings are keyed `Library::name` for every provided library but InterfaceLib (`glue::binding_key`), so two libraries exporting one name stay separate.


---

# macppc RUNS (2026-10-18, ppcrun lane)

`retro-ppc` is now the CPU. `MacPpc::start(&frag)` builds the CFM entry state (fresh 256 KiB stack, r1 16-byte aligned over a
zero back chain, r2 = RTOC of the main TVector, r12 = the TVector address, LR = the ExitToShell glue); a fragment with an
`init` routine runs it first (LR = the `$InitDone` pseudo-glue, which switches the CPU to `main`; r3 = 0 where real CFM passes a
`CFragInitBlock *`; its OSErr result is honoured: nonzero is a sticky fault and `main` does not run; term routines are not run). `MacPpc::step(budget) -> (Step, RunStats)` has the 68k engines' shape:
`Exited | Blocked | BudgetSpent | Fault(String)`; `run_loop(max)` drives it. Faults are sticky and named: a CPU trap
(`PowerPC fault: unimplemented ... at pc`), a refused import (`NEXT-NEEDED <symbol>: why`), a bus fault on the placeholder
address of an unbound DATA import (names the unbound data imports).

How a Toolbox call happens: every glue stub address is bound with `Cpu::bind_native`; the interpreter calls `Host::native`, which
copies r0..r31 into `glue_call`, runs the same Rust handler the 68k personality runs, and writes r3/r4 back; the CPU then
returns through LR. Arguments past r10 are read from the stack parameter area (r1 + 24 + 4*i). Results: INTEGER sign-extended,
Boolean zero-extended. Pascal strings, VAR parameters and Rect/Point pointers are identical to the 68k conventions (a pointer is a
pointer; a Point is the two shorts packed in one register). `qd` is the real QDGlobals block; `InitGraf(&qd.thePort)` fills it in place.
`WaitNextEvent` with sleep != 0 and no matching event returns `GlueOutcome::Idle`: the CPU is parked ON the call
(`Step::Blocked`, no instructions run) and the call is retried when a queued event matches its mask. `GetNextEvent` never waits
(Inside Macintosh): FALSE and a null event at once. (mac68k's `GetNextEvent` still blocks; that is its lane's to change.) The sleep's tick count is not
honoured (no timer wakes the guest).

Not handled, each a named limit: floating-point arguments/results (f1..f13; no InterfaceLib import this engine binds takes one),
64-bit results in r3:r4, Mixed Mode UPPs/RoutineDescriptors (callbacks into PPC or 68k code), `longjmp`, `atexit`, term routines.
`SysBeep` is a macppc native that logs only (GuestUi has no beep; a UI-side beep event is the missing piece); the 68k engine does not
register it yet (its tests use SysBeep as their unimplemented-trap example).

InterfaceLib export names that differ from the 68k trap table's spelling are aliased (`GetMenu`=`GetRMenu`, `BeginUpdate`=
`BeginUpDate`, `EndUpdate`=`EndUpDate`, `XorRgn`=`XOrRgn`); found by importing a real PEF.

`stdclib.rs` serves the 8 StdCLib symbols the real showcase PEF imports: `_IntEnv`, `__C_phase`, `_exit_status`, `__NubAt3`,
`__target_for_exit` (zeroed guest blocks sized as classix lays them out: `_IntEnv` 16 = the 14-byte packed `{u16, argc, argv, envp}`, `__NubAt3` 4, `__target_for_exit` 256), `__setjmp` (private 23-word layout), `exit` (stores `_exit_status`, ends the run),
`_BreakPoint` (logged). Other StdCLib names are NEXT-NEEDED by name.

`fixture::startup_app()` is a real program assembled with `asm.rs`: InitGraf(&qd.thePort), the Init calls, NewWindow, SetPort,
MoveTo, DrawString, a WaitNextEvent loop, FindWindow/TrackGoAway on the native close, SysBeep, ExitToShell.
`tests/run.rs` pins it with pixels (every glyph of the string through `present_content`), and negative controls.

Real application: the Systemless "Toolbox Showcase" (PEF + resource fork from `toolbox-showcase.sit`), see `tests/real_app.rs`.
`macppc-run --census --rsrc showcase.rsrc showcase.pef` (`--rsrc` attaches a raw resource fork; a MacBinary/AppleDouble file's own
fork is attached automatically): 249 imports, 95 bound (StdCLib 8/8). It runs 261 PowerPC instructions through its C startup, the
Init calls, `GetNewMBar 128` (menus 128..131 from its REAL resource fork), four `GetMenu`/`InsertMenu`, `DrawMenuBar`, and stops at
`NEXT-NEEDED GetNewCWindow` (Color QuickDraw windows). Without the resource fork it stops earlier: no MBAR 128, the app quits.

`asm.rs` and `fixture.rs` are behind the `fixtures` cargo feature (off for the product library; the tests and `macppc-run` enable it).
