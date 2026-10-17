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
