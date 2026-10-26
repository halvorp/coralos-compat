# Directory Opus 5.102 on a real AROS m68k (oracle, 2026-10-18)

Ground truth for the engine lanes: DOpus5 (`/data/build/corpus-bin/dopus5/DOpus5/DirectoryOpus`, unmodified) running on
**Copperline + the AROS m68k ROM** (A1200, 68EC020, 8 MB fast), with every library call it makes logged through
breakpoints on library jump-table slots. No DOpus binaries are committed; only traces, screenshots and the tools.

## What runs, what does not (findings)

* **DOpus5 runs on AROS m68k**: the "Directory Opus" screen, the progress window, the desktop with disk icons, a lister
  with the button bank, the ARexx port `DOPUS.1` and all the `dopus_*` processes come up (screens: `screens/`).
* The AROS ROM + extension ROM ships **no disk libraries**. DOpus refuses to start without these, so `launch.s` adds
  **stub libraries** (every vector `moveq #0,d0; rts`, LibOpen returns the base, locale `GetCatalogStr` returns its default
  string): `rexxsyslib asl diskfont datatypes commodities amigaguide locale mathieeesingtrans mathieeedoubbas
  mathieeedoubtrans`. Consequently **calls into those libraries return 0**: no file requesters, no disk fonts, no
  datatypes, no locale. Everything else (exec, dos, intuition, graphics, layers, gadtools, icon, workbench, utility,
  mathieeesingbas, timer/input/console devices) is the real AROS implementation. The traces say so (`locale.*` etc.
  are stub calls).
* Without `SetProgramDir` the main process has `PROGDIR: = RunProg:`, `PROGDIR:Environment/default` is not found and
  DOpus starts bare (desktop icons only). `launch.s` therefore sets `PROGDIR: = DOpus5:` before `RunCommand`. (The committed traces
  are from the run with this fix.)
* Missing `mathieee*` libraries crash the module init path: `diskinfo.module` cleanup calls `CloseLibrary(-1)` ->
  address error `PC=$FFFFFFF3` in the main process (a DOpus 5.102 bug on any OS that lacks the math libraries).
* **Not reached**: a populated lister and a file operation (copy/rename/makedir). The listers of the default
  Environment open with an empty path; typing `DOpus5:` + Return into the path field showed the text but no
  `Examine/ExNext/ExAll` followed (`trace/3-type-path.txt`), so no entry could be selected. Probably the typed Return
  did not reach the string gadget activation, or a stubbed library (asl/locale) is in the way; unresolved, 2 h time box.
  The next person should drive the lister with the mouse (path field / drive-list button) and check the stubs first.

## Reproduce

Needs `/data/build/copperline/release/copperline` (+AROS ROMs), `m68k-linux-gnu-as/objcopy`, python3.

1. `cp -r /data/build/corpus-bin/dopus5/DOpus5 /data/build/oracle/dopus/tree` (the guest may write to it).
2. `python3 tools/wrap.py launch.s launch`  (assembles the PC-relative launcher into a one-hunk Amiga executable).
3. `tools/start.sh`  : `copperline --config dopus.toml --noaudio --model A1200 --fast 8M --run .../launch --control 127.0.0.1:0
   --control-info ccp.json`. `dopus.toml` = AROS ROM paths + `[[filesys]] path=tree volume=DOpus5` (guest sees `DOpus5:`).
   The launcher: stub libs, `AssignAdd LIBS: DOpus5:Libs`, `SetProgramDir(DOpus5:)`, `LoadSeg` + `RunCommand`.
4. `python3 tools/scenario.py OUTDIR` : connects over the control protocol, `continue` (stops at the launcher's LoadSeg),
   arms ~310 breakpoints (`symbols.rom` gives every ROM library's jump-table slot; disk/stub libraries and
   `dopus5.library` are scanned from their jump table when `OpenLibrary` returns, names from `lvo_names.rs` /
   AROS `*.conf` / DOpus `dopus_lib.fd`), then drives: startup to t=28 s, double-click the DOpus5 icon, type a path, click.
   Emulation is deterministic: the same run gives the same trace. ~6 min host time.
5. `python3 tools/compact.py OUTDIR/p1-startup.raw > trace/startup.txt` (DOpus's own tasks only, ROM-internal and noisy
   calls elided and counted in the footer).

Mechanism: Copperline `break.add {"kind":"pc"}` on each jump-table slot (`JMP abs.l`), `run_until`, `regs.get`, `mem.read`;
at a stop the return address is read from the stack and a temporary breakpoint there records the result in D0.
Arguments are decoded from AROS `.conf` register signatures (strings, TagItem lists with tag names from the AROS headers,
NewWindow, TextAttr, port names). The task name is `ThisTask->ln_Name`. `S<seq>` order = call order; the compact files
print `t(s) task lib.Func(args) = result`.

## Files

* `launch.s`, `tools/*` : launcher + the tracing/driving tools (paths are the ones on this machine).
* `trace/startup.txt` : DOpus-originated calls from `RunCommand` to the idle state (t = 2.8 .. ~29 s of emulated time).
* `trace/*.txt` : later phases (open lister, ...).
* `screens/*.png` : screenshots (716x285, the Copperline PAL frame).
* `EXPECTATIONS.md` : invariants the engine can test against.
