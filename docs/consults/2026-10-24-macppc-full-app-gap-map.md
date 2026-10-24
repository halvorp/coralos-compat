# macppc full-app gap map: what a large Mac OS 9 PowerPC game (Warcraft III class) imports, what the engine has, what is missing

2026-10-24, branch agent/macppc-1024. North star (Peter, 2026-10-24): launching the Mac OS 9 PowerPC build of Warcraft III must work.
Peter supplied his own retail discs and patchers (test inputs only; nothing from them is committed, only library and symbol NAMES and counts).
The measured import tables below REPLACE the inference this document started from; the rest of the document (library by library) is updated against them.

## Measured: Warcraft III for Mac OS 9 (owner-supplied discs, 2026-10-24)

**Where the executables are.** The retail CDs carry the game inside Blizzard's `Installer Tome` (an MPQ archive with encrypted, PKWARE-compressed blocks; names are not
listed, the keys were recovered from the sector-table structure and the blocks decoded with the zlib `blast` algorithm for ANALYSIS only). The unpacked members are PowerPC
applications whose data fork holds several CFM fragments: the application, a private shared library **StormCarbon** (Blizzard's Storm: Win32-shaped file/registry/memory calls) and
**CursorDeviceLib**; `Miles Carbon Library`, `MP3Dec.asi`, `MSSDolby.m3d`, `MSSFast.m3d`, `Reverb3.flt` (the Miles Sound System) are separate PEF files that sit beside the game. Disc
RoC: `Warcraft III` (11.4 MB data fork), `Warcraft III World Editor`, `BNUpdate` (Battle.net updater). Disc TFT: the same three (the TFT game is 14.0 MB and carries no StormCarbon of
its own: it imports 107 symbols from `Miles Carbon Library` and none from StormCarbon). **The three TFT patchers do NOT carry the game**: each is a 748 KB code + 28 KB data PowerPC
application (389 imports, CarbonLib only) followed by ~65 MB of patch data in the same data fork; the first call they cannot make is `MaxBlock`.
`War3.mpq` (RoC, 420 MB) is game data only (no executables). The RoC `Warcraft III Installer` (600 KB data fork, 345 imports) is also a multi-fragment application (it embeds StormCarbon
and CursorDeviceLib) and stops at its first StormCarbon call.

**What the first run does today** (census with `--run --idle 6 --libdir <Miles files>`), after round 4: the private libraries inside each application file (StormCarbon, CursorDeviceLib) and the Miles
files beside it load as real fragments and their init routines run; guest RAM is 256 MB. The RoC game runs 176k instructions of Blizzard's own code (StormCarbon's registry-in-resources, memory setup,
threads) and stops at `GetGWorld` (offscreen graphics worlds: NewGWorld, LockPixels, GetPixBaseAddr, SetGWorld ... are not in the shared QuickDraw core); the TFT game 161k, also at `GetGWorld`; the RoC
installer 93k, at `SndDoImmediate` (Sound Manager, being built on the 68k side); BNUpdate 96k at `RegisterAppearanceClient` (Appearance Manager). Before: every StormCarbon importer stopped
after ~20 instructions and the 14 MB TFT game could not start.

#### RoC Warcraft III (Mac OS 9 game) — 888 imports from 9 libraries (private libraries and Miles loaded as real fragments)

| library | bound | named refusal | weak-missing | first missing symbols (names only) |
|---|---|---|---|---|
| StormCarbon | 257 | 0 | 0 |  |
| CursorDeviceLib | 9 | 0 | 0 |  |
| CarbonLib | 195 | 127 | 93 | DeactivateTSMDocument, GetWindowFromPort, CountDITL, EmbedControl, GetControlBounds, GetKeyboardFocus, ActivateControl, SetKeyboardFocus |
| Apple;Carbon;Multimedia | 0 | 0 | 12 | GetMovieBox, DisposeMovie, NewMovie, SetMovieBox, EnterMovies, SetMovieGWorld, StopMovie, GetMoviesError |
| DrawSprocketLib | 0 | 0 | 10 | DSpFindBestContextOnDisplayID, DSpContext_SetState, DSpGetVersion, DSpGetNextContext, DSpStartup, DSpContext_Release, DSpContext_Reserve, DSpContext_FadeGamma |
| InputSprocketLib | 0 | 0 | 16 | ISpElementList_GetNextEvent, ISpElement_GetConfigurationInfo, ISpElementList_Dispose, ISpGetVersion, ISpElementList_New, ISpDevice_GetElementList, ISpShutdown, ISpStartup |
| OpenGLLibrary | 0 | 62 | 0 | aglDestroyRendererInfo, glNormal3fv, glActiveTextureARB, aglDescribeRenderer, glTexEnvi, aglSetDrawable, glTexCoordPointer, glLockArraysEXT |
| OpenGLMemory | 0 | 0 | 2 | glmSetFunc, glmSetMode |
| Miles Carbon Library | 105 | 0 | 0 |  |

#### RoC World Editor — 862 imports from 9 libraries (private libraries and Miles loaded as real fragments)

| library | bound | named refusal | weak-missing | first missing symbols (names only) |
|---|---|---|---|---|
| Miles Carbon Library | 95 | 0 | 0 |  |
| CarbonLib | 209 | 129 | 116 | GetWindowFromPort, CountDITL, EmbedControl, GetControlBounds, GetKeyboardFocus, ActivateControl, SetMenuItemModifiers, SetKeyboardFocus |
| Apple;Carbon;Multimedia | 0 | 0 | 12 | GetMovieBox, DisposeMovie, NewMovie, SetMovieBox, EnterMovies, SetMovieGWorld, StopMovie, GetMoviesError |
| DrawSprocketLib | 0 | 0 | 7 | DSpFindBestContextOnDisplayID, DSpContext_SetState, DSpGetVersion, DSpStartup, DSpContext_Release, DSpContext_Reserve, DSpContext_FadeGamma |
| InputSprocketLib | 0 | 0 | 16 | ISpElementList_GetNextEvent, ISpElement_GetConfigurationInfo, ISpElementList_Dispose, ISpGetVersion, ISpElementList_New, ISpDevice_GetElementList, ISpShutdown, ISpStartup |
| OpenGLLibrary | 0 | 58 | 0 | glNormal3fv, glActiveTextureARB, glTexEnvi, aglSetDrawable, glTexCoordPointer, glLockArraysEXT, glDrawBuffer, glNormalPointer |
| OpenGLMemory | 0 | 2 | 0 | glmSetFunc, glmSetMode |
| StormCarbon | 209 | 0 | 0 |  |
| CursorDeviceLib | 9 | 0 | 0 |  |

#### RoC BNUpdate — 228 imports from 2 libraries (private libraries and Miles loaded as real fragments)

| library | bound | named refusal | weak-missing | first missing symbols (names only) |
|---|---|---|---|---|
| CarbonLib | 107 | 40 | 3 | CountDITL, GetControlBounds, SetWindowContentColor, SetKeyboardFocus, SetThemeCursor, GetDialogPort, DrawThemeFocusRect, StandardAlert |
| StormCarbon | 78 | 0 | 0 |  |

#### RoC Installer — 345 imports from 2 libraries (private libraries and Miles loaded as real fragments)

| library | bound | named refusal | weak-missing | first missing symbols (names only) |
|---|---|---|---|---|
| StormCarbon | 55 | 0 | 0 |  |
| CarbonLib | 176 | 58 | 56 | PBHGetVolParmsSync, DisposeGWorld, LGetSelect, InsTime, LSetSelect, TEGetHeight, TEStyleNew, PBUnmountVol |

#### TFT Warcraft III (game) — 736 imports from 10 libraries (private libraries and Miles loaded as real fragments)

| library | bound | named refusal | weak-missing | first missing symbols (names only) |
|---|---|---|---|---|
| CarbonLib | 256 | 138 | 72 | DeactivateTSMDocument, GetWindowFromPort, CountDITL, EmbedControl, GetControlBounds, GetKeyboardFocus, SetKeyboardFocus, DMGetNextScreenDevice |
| Apple;Carbon;Multimedia | 0 | 0 | 12 | GetMovieBox, DisposeMovie, NewMovie, SetMovieBox, EnterMovies, SetMovieGWorld, StopMovie, GetMoviesError |
| Apple;Carbon;Networking | 1 | 0 | 41 | OTOpenAppleTalkServicesInContext, OTATalkGetZoneList, OTATalkGetMyZone, OTAsyncOpenEndpointInContext, OTCloseProvider, OTRcvOrderlyDisconnect, OTInetStringToAddress, OTRcvUData |
| DrawSprocketLib | 0 | 0 | 10 | DSpFindBestContextOnDisplayID, DSpContext_SetState, DSpGetVersion, DSpGetNextContext, DSpStartup, DSpContext_Release, DSpContext_Reserve, DSpContext_FadeGamma |
| InputSprocketLib | 0 | 0 | 16 | ISpElementList_GetNextEvent, ISpElement_GetConfigurationInfo, ISpGetVersion, ISpElementList_New, ISpElementList_Dispose, ISpShutdown, ISpDevice_GetElementList, ISpStartup |
| Miles Carbon Library | 107 | 0 | 0 |  |
| OpenGLUtility | 0 | 0 | 1 | gluErrorString |
| OpenGLMemory | 0 | 0 | 2 | glmSetFunc, glmSetMode |
| OpenGLLibrary | 0 | 0 | 70 | glClientActiveTextureARB, glDeleteFencesAPPLE, glLightModelfv, glAlphaFunc, glViewport, glNormalPointer, glColorPointer, glClear |
| CursorDeviceLib | 10 | 0 | 0 |  |

#### TFT World Editor — 829 imports from 10 libraries (private libraries and Miles loaded as real fragments)

| library | bound | named refusal | weak-missing | first missing symbols (names only) |
|---|---|---|---|---|
| CarbonLib | 277 | 161 | 151 | GetWindowFromPort, CountDITL, EmbedControl, ResolveAlias, GetControlBounds, GetKeyboardFocus, SetKeyboardFocus, ActivateControl |
| Apple;Carbon;Multimedia | 0 | 0 | 12 | GetMovieBox, DisposeMovie, NewMovie, SetMovieBox, EnterMovies, SetMovieGWorld, StopMovie, GetMoviesError |
| Apple;Carbon;Networking | 1 | 0 | 30 | OTOpenAppleTalkServicesInContext, OTATalkGetZoneList, OTATalkGetMyZone, OTAsyncOpenEndpointInContext, OTCloseProvider, OTRcvOrderlyDisconnect, OTInetStringToAddress, OTRcvUData |
| DrawSprocketLib | 0 | 0 | 7 | DSpFindBestContextOnDisplayID, DSpContext_SetState, DSpGetVersion, DSpStartup, DSpContext_Release, DSpContext_Reserve, DSpContext_FadeGamma |
| InputSprocketLib | 0 | 0 | 16 | ISpElementList_GetNextEvent, ISpElement_GetConfigurationInfo, ISpGetVersion, ISpElementList_New, ISpElementList_Dispose, ISpShutdown, ISpDevice_GetElementList, ISpStartup |
| OpenGLLibrary | 0 | 0 | 66 | glClientActiveTextureARB, glDeleteFencesAPPLE, glLightModelfv, glAlphaFunc, glViewport, glNormalPointer, glColorPointer, glClear |
| OpenGLUtility | 0 | 0 | 1 | gluErrorString |
| OpenGLMemory | 0 | 0 | 2 | glmSetFunc, glmSetMode |
| Miles Carbon Library | 95 | 0 | 0 |  |
| CursorDeviceLib | 10 | 0 | 0 |  |

Libraries that matter beyond CarbonLib, with the counts above: **StormCarbon** (257 imports by the RoC game: a private library to be loaded from the application's own
`cfrg` fragments, not emulated), **OpenGLLibrary** (62 strong imports in RoC, 70 weak in TFT: `agl*` and `gl*` entry points), **Miles Carbon Library** (105-107: `AIL_*`, a separate PEF
on disc that can be loaded as a real fragment, with its own `Apple;Carbon;Multimedia` weak imports), **DrawSprocketLib** / **InputSprocketLib** (all WEAK: the game takes a no-sprocket path
if they are absent), **QuickTime** (`Apple;Carbon;Multimedia`, weak), **Open Transport** (`Apple;Carbon;Networking`, weak, 42), **CursorDeviceLib** (a private library, also in the data fork).

## Stand-ins in the corpus (what we can measure today)

The earlier stand-ins, by what they exercise (the Warcraft III binaries above now supersede them for CarbonLib, OpenGL and the Sprockets):

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

## 4. Order I would build it in (revised against the measured tables)
0. The CFM prerequisites the measurements exposed: guest RAM large enough for a 14 MB application, and **multi-fragment loading** -- the application's own `cfrg` lists its private libraries (StormCarbon, CursorDeviceLib) as fragments in its data fork, and separate PEF files beside it (Miles) are imported by name: instantiate them first, run their init routines, bind imports to their exports.

1. (done) CFM: named refusal for every unprovided-library symbol; CarbonLib and the Toolbox sub-libraries resolve through the one table.
2. (done) Census per library, and the real binaries' census: the measured tables above.
3. StdCLib + MathLib natives (a C game cannot start without them).
4. `recent PCs` in `macppc-run` and the FixedBenchmark `locale` fault (interpreter correctness before any more library breadth).
5. File Manager: FSSpec C names, host-backed large files.
6. Carbon event loop + windows over guestui-abi (CarbonLib proper), Gestalt profile for PowerPC.
7. Sound Manager channels (needs the additive audio call).
8. Multiprocessing Services + Thread Manager (scheduler design).
9. DrawSprocket / InputSprocket shims.
10. OpenGL/AGL (command stream + a backend; the largest piece, and the one that makes WC3 visible).

## Status after round 4 (2026-10-24)
Landed since this map was written: StdCLib/MathLib natives, the FSSpec and File Manager C routines, the FILE stream layer (`stdio.rs`), qsort/bsearch/atexit callbacks (`callback_c.rs`), the PowerPC
Gestalt profile (`gestalt_ppc.rs`), the scheduler (`sched.rs`), the cooperative Thread Manager (`threads.rs`, `'thds'` answered), cooperative Multiprocessing Services (`mp.rs`), CoreFoundation strings
(`cf.rs`) and the Carbon Event Manager core with `CreateNewWindow`/`SetWindowTitleWithCFString` (`carbon.rs`). Still missing for a Carbon game, in the order I would take them:
1. Menu commands: `kEventCommandProcess` needs `SetMenuItemCommandID`/`GetMenuItemCommandID` and the menu bar to deliver HICommands; today menu choices are dropped by the Carbon converter (logged).
2. More window events and calls: `kEventWindowBoundsChanged`, `GetWindowBounds`/`SetWindowBounds`, `SetWindowProperty`, `InstallStandardEventHandler` defaults beyond close/update, floating windows (the seam has no floating role), `ShowWindow` ordering rules.
3. Timers and the event-loop services games use: `InstallEventLoopTimer`, `InstallApplicationEventHandler` variants are macros over what exists; `ReceiveNextEvent` ignores the type filter and refuses peek (`pullEvent` false).
4. HIToolbox views (`HIViewRef`, `HIObject`), Appearance Manager, Navigation Services, Control Manager Carbon accessors (`GetControlBounds`...), `Gestalt 'cbon'`/`'appr'`/`'nav '`/`'sysv'` once the above are true.
5. Preemption for MP Services (see `mp.rs`'s header), Sound Manager (the mac68k worker), DrawSprocket/InputSprocket shims, OpenGL/AGL.

## Status after round 5 (2026-10-24)

Done: offscreen GWorlds (bodies once in `mac68k-engine/src/cqd/gworld.rs`, C names in `macppc-engine/src/gworld.rs`; indexed 1/2/4/8 bits, direct 16/32 refused by name; the 68k
`_QDExtensions` selectors are NOT registered, no verified selector table exists); QuickDraw initialised for a Carbon program on its first CarbonLib import; Appearance registration and
the Control/Dialog/Window accessors (`carbon_ctl.rs`); the Carbon menu command path (`SetMenuItemCommandID`, `kEventCommandProcess` + `HICommand`).
Measured stops (50M-instruction budget, idle 6): Warcraft III RoC and the World Editors now stop at `ResolveAliasFileWithMountFlags` after 767,770 / 642,272 instructions (was
`DeactivateTSMDocument` at 176,611); TFT at the same call after 736,184; BNUpdate at `GetScriptManagerVariable` (95,575; was `RegisterAppearanceClient`); the Installer at
`SndDoImmediate` (the 68k worker's Sound Manager). Next, in order: the alias/File Manager names (`ResolveAliasFileWithMountFlags`, `NewAlias`), Script Manager variables, the
TSM document calls, CFString/CFArray/CFDictionary breadth, `KeyTranslate`/`TestControl`, `HideWindow` (needs a GuestUi hide call), then OpenGL (waits for the CoralOS-side design).

## Status after round 6 (2026-10-24)

Done: Alias Manager by C name (`alias_c.rs`, over the 68k bodies), Script Manager names (`script_c.rs`, the 68k Roman table), TSM documents without an input method (`tsm_c.rs`), CoreFoundation breadth
(`cf_more.rs`: strings, arrays, dictionaries, numbers, path URLs), `KeyTranslate` through a program's own KCHR (`keys_c.rs`), `TestControl`, `HideWindow`/`ShowWindow` with additive
`GuestUi::hide_window`/`show_window`, and the ROM's NewGWorld depth/flag rules. Measured stops (50M budget): RoC 950,168 and TFT 948,038 instructions to `GetMenuTitle`; the World
Editors 847,417 / 810,164 to `StandardAlert`; BNUpdate 95,575 to `GetScriptManagerVariable(40)` (no verified Roman answer); the Installer 92,970 to `SndDoImmediate` (68k worker).
Next: Menu Manager Carbon accessors (`GetMenuTitle`, `SetMenuTitle`, `GetMenuID`, `IsMenuItemEnabled`, `SetMenuItemProperty`, `MenuKey`, `PopUpMenuSelect`, Show/HideMenuBar), `StandardAlert`,
then the CarbonLib export list (5715 names on the 9.2.1 CD, /data/build/corpus-bin/mac/macos921/exports/) to rank the remaining gap; OpenGL waits for the CoralOS-side design.
