# macppc full-app gap map: what a large Mac OS 9 PowerPC game (Warcraft III class) imports, what the engine has, what is missing

2026-10-24, branch agent/macppc-1024. North star (Peter, 2026-10-24): launching the Mac OS 9 PowerPC build of Warcraft III must work.
That binary is not in the corpus (it has to come from Peter's own copy; never committed). This map is built from the engine's code, the
PowerPC census, and the shape of a Carbon CFM game of that era (CarbonLib, Apple OpenGL 1.2 era libraries, Sound Manager, Sprockets,
Multiprocessing Services, Navigation Services, big data files, a PEF with hundreds to thousands of imports). Claims about what Warcraft III
imports are INFERENCE from that shape, not from the binary; the first action on getting the binary is `macppc-run --census` on it and
replacing this paragraph with its real per-library import counts.

## Stand-ins in the corpus (what we can measure today)

No corpus program imports CarbonLib or OpenGL. The honest stand-ins, by what they exercise:

| program | libraries it imports (census) | what it stands in for |
|---|---|---|
| Glider PRO 1.1.2 PEF (Macintosh Garden) | InterfaceLib 278 (205 bound, 73 named NEXT-NEEDED), weak DragLib 9, weak QuickTimeLib 25 | a shipped commercial game: Sound Manager, Icon Suites, weak-absent libraries (the CFM weak path) |
| Systemless toolbox-showcase PEF | InterfaceLib 216, StdCLib 8, "QuickDraw 3D" library 26 | a second shared library family beside InterfaceLib; the MPW startup |
| Retro68 FixedBenchmark / Raytracer (PowerPC) | InterfaceLib + C++ runtime | big code (FixedBenchmark: 1.5 MB code section) and libstdc++ static init; its fault (a `locale::id` facet table load from 0x01249224 at 0x2a4f38) is the one interpreter-level stop left in the Retro68 set |
| Retro68 Application | `Library` (an unprovided library) | an import from a library nobody provides: the named-refusal path |

## 1. The CFM import-resolution path (first, because everything else depends on it)

An app with 1000+ imports across 10+ libraries must LOAD, and each unserved symbol must refuse BY NAME when (and only when) it is called.

What the engine does today (code):
* `MacPpc::instantiate` (`crates/macppc-engine/src/cfm.rs`, the imports loop) resolves every import through `bind_from(library, symbol)` and
  never fails the load for a missing function in the default mode: an unserved function gets a glue cell whose call refuses
  (`Some(why) => ... self.glue.intern(...)`), a weak one relocates to 0 (`ImportStatus::WeakAbsent`), a strong missing DATA import points at
  `UNBOUND_DATA` (0xBAD00000) so any touch is a bus fault that names the list (`unbound_data`). `--strict` refuses at load instead.
  Glue is allocated in chunks of 256 cells from the guest heap (`glue.rs`, `CHUNK_CELLS`), so 1000+ imports need no fixed table (no hard cap).
* Only `InterfaceLib` and `StdCLib` are "provided" libraries (`natives.rs`, `PROVIDED_LIBRARIES`; `machine.rs`, `bind_from`). Every other library
  name is `Unbound("library X is not provided by the engine")` for every symbol. That is correct and named, but it means a CarbonLib app binds NOTHING
  (CarbonLib re-exports the Toolbox under the same C names InterfaceLib does).
* BUG found by this map (fixed in the commit that carries it): the refusal of a symbol from a non-provided library was NOT named. `bind_from` did not record the
  Unbound binding under the glue key (`glue.rs`, `binding_key` = `Library::name`), so a call ended in "no binding was recorded for this glue stub"
  (Retro68 `Application` calling `Library::is_computer_on` after 115 instructions) instead of `NEXT-NEEDED Library/is_computer_on: library Library is not provided`.

Missing, in build order:
1. Record every Unbound binding under its glue key (fixes the unnamed refusal). DONE with this map.
2. The Toolbox family of libraries resolves through the one Toolbox name table: `CarbonLib`, and the Mac OS 8.6+ sub-libraries InterfaceLib is split into
   (`ControlsLib`, `WindowsLib`, `MenusLib`, `DialogsLib`, `AppearanceLib`, `NavigationLib` for the names already served, `TextCommon`, `QuickDrawText`...). Same
   handlers, same refusals; a name nobody serves is still NEXT-NEEDED with the library in the message. DONE for the first group with this map (CarbonLib,
   the *Lib sub-libraries listed in `glue.rs`); `GetSharedLibrary`/`FindSymbol` accept them too.
3. A census mode that prints per-library counts (bound / named / weak-absent), so the first run on a big binary produces THE worklist, ranked by library.
4. Carbon-only names (`GetWindowPort`, `GetControlBounds`, `NewControl`-era accessors, `InitCursor` vs `InitCursor`...) are not Toolbox traps; each needs a handler in
   mac68k-engine's table or a native here. Rank by call count from the first real census, not by guess.

## 2. Library by library

| library (as imported) | engine today | missing | order |
|---|---|---|---|
| InterfaceLib | 205/278 of Glider PRO bound by name through the shared Toolbox table (`symbols.rs`, `lookup`); File Manager C routines (`files.rs`), OS routines with pointer outputs (`osroutines.rs`), events, windows, menus, dialogs, controls, QuickDraw/Color QuickDraw, Resource Manager, Memory Manager | the Window Manager tail (GrowWindow, ZoomWindow, TrackBox...), Icon Suites, SetPtrSize, FSSpec forms (`FSpOpenDF`...), Apple Events (AE*), Sound Manager (below) | with the first real census |
| CarbonLib | not provided: every symbol refuses by name (`machine.rs` `bind_from`) | everything; step 2 above maps its Toolbox-named exports onto the shared table; Carbon event model (`RunApplicationEventLoop`, `InstallEventHandler`, `CreateNewWindow`, HIToolbox) is NEW work: a window/event layer over `guestui-abi`, no 68k analogue | after the family mapping, by call count |
| StdCLib | 8 symbols (`stdclib.rs`: `__setjmp`, `exit`, `_BreakPoint`, five runtime variables) | `longjmp`, `malloc`/`free`/`memcpy`/`strlen`/`printf`/`fopen` family as natives over guest memory and the File Manager; `atexit`, `qsort`, `rand`, `time` | high: a C game links hundreds of these; natives, never interpreted |
| MathLib | not provided (QuickDraw 3D showcase names it) | the libm set (sin/cos/pow/sqrt/fabs/floor ...) as host-side natives with PowerPC FP argument passing (FPRs, `retro-ppc` has the FPU) | high; cheap |
| OpenGLLibrary / OpenGLUtility / AGL (`aglCreateContext`, `aglSetDrawable`, `glBegin`...) | nothing | the whole GL 1.2 surface (~250 entry points) as natives that record to a command stream; a software or host-GPU backend behind a new GuestUi surface call (`guestui-abi` has no GL surface yet: an ADDITIVE shared change, to be agreed); `aglSwapBuffers` as the present | big; after CarbonLib windows, because a GL context needs a window |
| DrawSprocketLib | nothing | `DSpStartup`, context reserve/queue, `DSpContext_GetBackBuffer`, display switching: map to ONE native window (no mode switch); fullscreen only by the user's toggle (Peter 10-20 games ruling) | medium |
| InputSprocketLib | nothing | `ISpStartup`, `ISpElement_NewVirtual`, `ISpInit`, `ISpConfigure`; keyboard/mouse devices from the GuestUi intents | medium |
| SoundLib / Sound Manager (InterfaceLib) | `SndNewChannel`/`SndDoCommand`/`SndDoImmediate`/`SndDisposeChannel`/`SetSoundVol` are named NEXT-NEEDED (Glider PRO) | channels with callbacks (the app's callback runs as PowerPC code through `callback.rs`, Mixed Mode slice 3), `bufferCmd`/`soundCmd`, sound resources 'snd ', `SndPlayDoubleBuffer`; output needs a GuestUi audio call (additive shared change) | medium-high (every game) |
| MultiprocessingServicesLib | nothing; `retro-ppc` is ONE interpreter with no threads (`run.rs`) | `MPCreateTask`, queues, semaphores, critical regions: tasks as cooperatively scheduled interpreters with their own register file; `lwarx`/`stwcx.` exist in the CPU (`retro-ppc/src/cpu.rs`) | high for a multithreaded game; design the scheduler first |
| ThreadsLib (Thread Manager) | nothing | `NewThread`/`YieldToAnyThread`: cooperative threads on the same scheduler | with MP |
| NavigationLib (Navigation Services) | `StandardGetFile`/`StandardPutFile` are named NEXT-NEEDED (Glider PRO) | `NavGetFile`, `NavPutFile`, `NavCreateGetFileDialog`: ask the host for a file chooser (GuestUi request), like `Alert` | medium |
| File Manager on large data files | `files.rs` + mac68k `fm/` over `retro-volume` (an in-memory tree: `Kind::File(Vec<u8>)`) | a Warcraft-III-size data set (hundreds of MB, memory-mapped by MPW-era `FSRead` calls) cannot live in a `Vec` per file: needs a host-file-backed volume node (lazy, read-through) in `retro-volume` (a shared crate: additive), plus `PBRead` async completion and `FSpOpenDF`/`FSMakeFSSpec`/`PBHOpenDF`/`PBGetCatInfo` (the mac68k `fm/fsspec.rs` has the handlers; the C-name binding is missing) | high: before any game data loads |
| QuickTimeLib (weak in Glider PRO) | weak-absent: the app takes its no-QuickTime path | movie playback: only if a game cut-scene path is required; leave weak | low |
| DragLib | weak-absent in Glider PRO | Drag Manager | low |
| Gestalt / SysEnvirons | `Gestalt` answers 'sysa' = PowerPC and the shared 68k-profile selectors (`osroutines.rs`); `sysv` is 7.0.0 | a PowerPC machine profile: `sysv` >= 0x0910 (a Carbon game checks for Mac OS 9.1), 'cput' (G3/G4), 'mach', 'qdrw', 'cbon' (CarbonLib present), 'appr' (Appearance), 'nav ' , 'thds', 'mpsv', 'cfrg', 'gfxf'. Each answer must be TRUE of the engine when given (the profile rule in gestalt.rs); do not claim 9.1 until CarbonLib's window/event layer exists. `SysEnvirons` stays refused for PowerPC (its record is 68k-shaped) | with CarbonLib |
| AltiVec | `retro-ppc` has none (`lib.rs:3`) | Warcraft III (2002, Mac OS 9) targeted G3 and later: AltiVec use is likely optional or gated by Gestalt 'altv'; leave Gestalt reporting NO AltiVec so the app takes its scalar path. | none |

## 3. Interpreter level (not a library, but it will stop a 1.5 MB game first)

* FixedBenchmark (1.5 MB code) faults at 0x2a4f38 loading through a `locale` facet table whose base is bad: a libstdc++-style static init
  reading `std::locale::id::_M_id()`. The cause is not yet known (a wrong value from an earlier load/store, an unimplemented
  special-register read, or a missing relocation); it needs the recent-PC ring the 68k side has (`mac68k-run --pcs`) added to `macppc-run`. Next stop.
* Performance: a 1.5 MB game runs hundreds of millions of instructions per frame; the interpreter (`retro-ppc`, 1293 lines) is a decode-and-execute loop with no block cache.
  A threaded or cached-block core is needed before frame rates matter. Measure with FixedBenchmark first (it is a benchmark by design).

## 4. Order I would build it in

1. (done) CFM: named refusal for every unprovided-library symbol; CarbonLib and the Toolbox sub-libraries resolve through the one table.
2. Census per library (counts), then run the real binary's census: replace the inference in this document with measurement.
3. StdCLib + MathLib natives (a C game cannot start without them).
4. `recent PCs` in `macppc-run` and the FixedBenchmark `locale` fault (interpreter correctness before any more library breadth).
5. File Manager: FSSpec C names, host-backed large files.
6. Carbon event loop + windows over guestui-abi (CarbonLib proper), Gestalt profile for PowerPC.
7. Sound Manager channels (needs the additive audio call).
8. Multiprocessing Services + Thread Manager (scheduler design).
9. DrawSprocket / InputSprocket shims.
10. OpenGL/AGL (command stream + a backend; the largest piece, and the one that makes WC3 visible).
