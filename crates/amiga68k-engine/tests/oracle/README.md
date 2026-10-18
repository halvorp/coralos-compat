# Oracle dumps (ground truth from a real AROS m68k, 2026-10-17)

Captured with Copperline (headless, `--control`) running the AROS m68k ROM + extension ROM on an A1200 (68020,
8 MB fast), with `winwait.s` (assembled by `m68k-linux-gnu-as -m68020`, wrapped as a one-hunk executable) stopped
at its `Wait` call:

- `window.bin`: the 136 public bytes of the `struct Window` OpenWindow returned (at $247778). Left/Top/Width/Height
  40/30/320/120, IDCMPFlags @82 = 0x300, UserPort @86 = $2479D0, Title @32 = the program's string.
- `msgport.bin`: the window's UserPort (34 bytes): ln_Type 4 (NT_MSGPORT), mp_Flags 0 (PA_SIGNAL), mp_SigBit 16,
  mp_SigTask = the program's task, an empty mp_MsgList (lh_Head = &lh_Tail, lh_TailPred = &lh_Head, lh_Type 0).

Pointers are AROS's own addresses: compare offsets, field values and invariants, not absolute pointers.

- `screen.bin` (from `scrlock.s`: LockPubScreen(NULL), stopped before UnlockPubScreen): the 346-byte `struct Screen`
  of the AROS Workbench screen. Left/Top/Width/Height 0/0/640/256, BarHeight 14, BarVBorder 1, BarHBorder 5,
  MenuVBorder 2, MenuHBorder 4, WBorTop/Left/Right/Bottom 6/4/4/4, Font -> TextAttr { "topaz.font", ta_YSize 8 }.

- `gfxprobe.s` / `gfxprobe.bin` / `gfxprobe.aros` (graphics, 2026-10-18): the program draws a rectangle, "AB" in topaz 8 and two
  lines on a private 64x32x3 bitmap (AllocBitMap + InitRastPort defaults) and dumps the 100-byte RastPort plus every pixel
  (ReadPixel) into `out`. Run with `copperline --config aros.toml --run <hunk>` under `--control`, a pc breakpoint on the final
  `spin` loop, then `mem.read` of `out`. `tests/gfx_oracle.rs` runs the same binary here and requires identical pixels and
  RastPort pen/mode/cursor fields. Differences found and left: `linpatcnt` after multi-segment Draw (AROS 13, here 10) and
  `minterms[]` (not mirrored).

- `boopsiprobe.s` / `boopsiprobe.bin` / `boopsiprobe.aros` (BOOPSI, 2026-10-18): MakeClass("probeclass","gadgetclass",0,8,0) + AddClass +
  NewObjectA, then the program copies into `out` the new 60-byte IClass, the object's memory from its _Object header (12+72+8 bytes) and
  the built-in gadgetclass / rootclass / imageclass IClass structs. Run under `--control`: `ccp.py` `continue` stops at "loadseg"
  (the reply names the first hunk), then `mem.read` of `hunk + 0xE8` (`out`) once the machine sits in the final `spin` loop.
  `tests/boopsi_oracle.rs` runs the same binary here. Findings: probeclass cl_InstOffset 72 / cl_InstSize 8 / cl_ObjectSize 92 / cl_Flags 1
  (CLF_INLIST), gadgetclass InstSize 72 and ObjectSize 84, imageclass InstSize 20 and ObjectSize 32, rootclass ObjectSize 0; gadgetclass's
  OM_NEW sets Flags 0x8000, GadgetType 5, MoreFlags 8, MutualExclude = the class pointer (= o_Class).
