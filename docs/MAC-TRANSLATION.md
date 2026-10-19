# The Mac translation layer (ruling 17 pieces 2, 3, 5, 8; shared by mac68k-engine and macppc-engine)

One design for every Toolbox program, 68k or PowerPC: the engine shows the program's OWN UI objects as semantic controls
through `guestui-abi`, shows its drawing as a display list, and turns native input back into the exact input the original
OS produced. Nothing here names an application or looks at what pixels look like.

## Layout (`crates/mac68k-engine/src`)

| file | what it is |
|---|---|
| `retro-translate/src/project.rs` + `rows.rs` | THE generic mapper (`guestui-abi` stays the seam types only), shared by all three engines (amiga68k, mac68k, macppc): `Projections::sync` (add / diff-push / retire a native control), `check_ref` (the single stale-intent rule: token + generation + window), `restore`, `adopt`; plus the role x intent row tables (`Row`, `lookup_role`) |
| `translate/mod.rs` | the Mac engine's thin use of it: which guest object (`Meta`) is which token, `sync_projection`, `deliver_intent` |
| `translate/fallback.rs` | a control the kit has no part for (a `procID` with no row, a pop-up whose MENU cannot be read) is drawn as the role's default look into the content plane, never dropped |
| `translate/controls.rs` | ONE `procID` -> CDEF table (`PROCS`) and ONE role x intent row table (`TABLE`: effect, then the `TrackControl` part code), the attribute map into the real `ControlRecord`, part geometry, the Control Manager traps |
| `translate/dialogs.rs` | ONE table: DITL item type -> item (`ITEM_TABLE`) and the text-intent row table (`TABLE`), `DialogRecord` + DITL copy with item handles, `ModalDialog` and the item traps |
| `translate/input.rs` | pointer and key events for content (`mouseDown/Up`, `keyDown/autoKey`, low-memory `Mouse`/`MBState`) |
| `qd/plane.rs` | the retained display list per window port, pens as roles, `present_plane` once per step |

macppc-engine adds nothing but one native (`ModalDialog` parks the PowerPC caller on the front dialog's item hit,
`Mac::modal_ready`); every table, handler and record is the 68k engine's, reached through the Mixed Mode core.

## What an intent becomes (exact guest input)

| intent on | guest sees |
|---|---|
| push button `Activate` | `mouseDown` in the button (global `where`) then `mouseUp` at the same place (`MBState` up: the click is complete), `FindControl` -> `inButton` (10), `TrackControl` -> 10 |
| check box `SetValue(Checked)` / `Activate` | same with `inCheckBox` (11); the VALUE IS NOT CHANGED (the program toggles); the native toggle is put back until it does |
| scroll bar `ScrollTo(n)` / `Drag(Scroll)` | `mouseDown` on the thumb where it was (`inThumb` 129); `TrackControl` leaves `contrlValue = min + n` and returns 129 |
| dialog button / check box (any `Activate` / `SetValue`) | `itemHit` = the item number from `ModalDialog` |
| dialog edit text `CommitText(s)` / a key typed into the dialog | the text is in the item's handle (`GetDialogItemText` reads it); `ModalDialog` returns the edit item (`editField + 1`, as `DialogSelect` after `TEKey`); a dialog's keys never reach the program's event queue |
| dialog scroll bar `ScrollTo(n)` | `contrlValue` = the dragged value (`TrackControl` ran), then `itemHit` = the item |
| Return / Enter in a dialog | `itemHit` = `aDefItem` (1); an `aDefItem` naming no enabled item is ignored by name |
| `Pointer` primary down / up | `mouseDown` / `mouseUp` at window + content origin; motion only moves the polled `Mouse` |
| `Pointer` secondary | `mouseDown` with `controlKey`; the middle button is a named refusal |
| `Key` down | `keyDown` (`autoKey` for the same key before its release), message = ASCII char; no character or non-ASCII: named refusal |

A stale or forged generation, a role that does not have the intent (`SelectRow` on a button), an inactive control: each is a
named refusal and produces NO guest input.

`ParamText` fills `^0`..`^3` in a dialog's static text when the item is made (a `ParamText` before `GetNewDialog`/`Alert`, as programs call it; a later one does
not re-fill an item already shown). `IsDialogEvent` is true for an update/activate of a dialog window, a mouse-down in its content and any key while a dialog is
front; `DialogSelect` returns the oldest pending item hit (the same queue `ModalDialog` drains) and is FALSE when there is none.

## Honest limits (all named in the log, none silent)

* `radioButProc` keeps the check box's intent rows (the app keeps the group; a click is `inCheckBox`, the value untouched) and is SHOWN as a one-label `Choice{Radio}` whose `selected` is 0 while `contrlValue` is non-zero and `CHOICE_NONE` while it is off (the seam can say "none chosen" since 2026-10-19; the host draws the kit radio group), `popupMenuProc` (1008 + variation bits) is a `Choice{Popup}` over its MENU resource and a choice returns `inButton` with the value set, `_Pack0` is the List Manager (`translate/lists.rs`: a real `ListRec`, standard text LDEF shown as a native `List`, a custom LDEF stays a plane). A `procID` with no table row has no part at all: the control exists in guest memory and is DRAWN into the content plane as its default look (`fallback.rs`), so it is visible and the census counts it as `replayed`. A custom CDEF's own art is not run.
* `ModalDialog`'s `filterProc` and `TrackControl`'s `actionProc` are guest callbacks the trap bridge cannot run from inside a trap: a non-NIL one is named, never ignored silently.
* A program's own CDEF, icon / picture / user dialog items: no contract to translate; they stay content (a user item is a plane).
* Press / Release / Drag gestures are not bridged: a complete click arrives as `Activate`.
* cicn / color icons are not decoded; `PlotIcon` of an `ICON` is recorded as the pixels it drew (a raw op).
* Text is drawn in the topaz stand-in in the bitmap; the display list carries the string, a font role and the point size.

## Measured, not claimed

`RecordingGuestUi::route_census(win)` reports native / replayed / raw per window. The tests pin it: a Control Manager window is
`native`, a QuickDraw window is one `replayed` plane, a window whose list holds any `Pixels` op (a `CopyBits`) is `raw`.
`tests/translate_planes.rs` replays the oracle application's list and requires it to agree with the bitmap on every pixel the
list names (and to name at least 60 % of them).

## Compressed resources and machine profiles (engine, not UI)

* `compressed.rs`: a resource with attribute bit 0 and the `$A89F6572` header is expanded by the `'dcmp'` resource its header names
  (the application's own fork first, then the System file set `ResMgr::add_system`), whose 68k code RUNS IN THE GUEST: the handler
  that needed the resource queues the call, the A-line bridge enters the decompressor through the completion trampoline and then
  executes the same trap word again (registers, SR and stack as they were; the abandoned run's halt/block request is cancelled).
  Version 8 headers use the classic `dcmp(src, dst, working, len)` entry at byte 0; version 9 the dispatch-table `FUNCTION
  dcmp(src, dst, header): OSErr`. No dcmp = `CantDecompress` (-186) with a named refusal; there is no host-side Apple algorithm.
* `profile.rs`: `Mac::set_machine("mac-plus")` (a bundle's `(guest (machine "mac-plus"))`) picks the CPU model (68000 / 68020), the
  boot address mode and the Gestalt / SysEnvirons answers. The default is `mac-ii` (68020, 24-bit). The 68000 pushes a 6-byte
  exception frame, the 68020 an 8-byte one, which is why a handler written for one corrupts the stack on the other.

## Event timing, retrace tasks, Apple Events and the rest of the macnext walls (engine, not UI)

* **Null events (`events.rs`).** `GetNextEvent` / `WaitNextEvent` with nothing queued still BLOCK the guest (the host has nothing to
  do), but the call now carries a deadline on the host's 60 Hz clock: `GetNextEvent`'s is the next tick, `WaitNextEvent`'s is its
  `sleep` (a sleep of 0xFFFFFFFF, or over a day, is "until an event comes"). When `Mac::advance_ticks` reaches it, the call completes
  with the NULL event and a FALSE result, exactly what a real Mac returns every time through the loop. Programs that begin their work
  on the first idle event (MacPaint's personalization dialog and tool windows) need this. `mac68k-run --idle N` passes N ticks while
  the guest waits; the census sets `MAC_IDLE`. Without ticks nothing changes.
* **Vertical Retrace Manager (`vbl.rs`).** `VInstall` / `VRemove`; each tick counts every task's `vblCount` down and a task reaching
  zero runs IN THE GUEST (`A0` = the task) at the program's next Toolbox/OS trap, with every register and the SR saved and restored
  (the resource decompressor's mechanism). `vblPhase` is not applied; a task never runs inside another.
* **Apple Events (`appleevents.rs`).** `_Pack8` carries its frame size in D0 (`paramWords << 8 | selector`), so any selector can be
  refused by name without corrupting the stack. Modelled: the handler registry (`AEInstallEventHandler`, `AERemoveEventHandler`,
  `AEGetEventHandler`), `AEDisposeDesc`, and ONE event: the Finder's `'aevt'/'oapp'`, queued when a program installs its `oapp`
  handler and returned by an event call whose mask has `highLevelEventMask` (0x0400); `AEProcessAppleEvent` rebuilds the stack and
  runs the handler as the Pascal function `handler(event, reply, refcon): OSErr` in the guest. SimpleText 1.3/1.4 create their
  Untitled window from that handler. Everything else (`AECreateAppleEvent`, `AESend`, `AEGetParamPtr`, lists) is `paramErr` with a
  named refusal.
* **Alias Manager and FindFolder (`fm/alias.rs`, `_AliasDispatch`).** Real `AliasRecord`s (the documented fixed part plus an absolute
  path tag) over FSSpecs: `NewAlias`, `NewAliasMinimal`, `NewAliasMinimalFromFullPath`, `ResolveAlias`, `UpdateAlias`, `GetAliasInfo`,
  `ResolveAliasFile`; `FindFolder` makes the System Folder and its standard subfolders (and the per-volume Trash / Desktop Folder) on
  first use. `MatchAlias` and the rest halt the guest by name.
* **The application is a file (`fm/forks.rs: install_app`).** At launch the program's own file is placed on the boot volume under
  its name and its resource fork is open as an FCB under the Resource Manager's refnum (2), which the File Manager never hands out
  again: before this, the first file a program opened got refnum 2 and its `Close` unloaded the application's own resources
  (Glider PRO, ResEdit). The Segment Loader reads `CODE` from the application's fork, not from `CurMap`. `OpenRFPerm`,
  `HOpenResFile`, `HCreateResFile` are modelled; every engine-side resource load leaves `ResErr` as the Resource Manager does
  (a stale -192 after a successful `GetNewMBar` made SimpleText 1.4 report a serious error).
* **Register packages.** `_Pack7` (`NumToString`, `StringToNum`) takes D0/A0 with only the selector on the stack (Advanced Mac
  Substitute `ams-pack` and the corpus applications' own glue); the stack-based form corrupted the stack of MacWrite 4.5, WriteNow 2.0
  and Glider PRO.
* **Palette Manager (`palette.rs`).** Real `PaletteRecord`s (16-byte `ColorInfo`, read off a corpus `'pltt'`), `GetNewPalette`,
  `NewPalette`, `NSetPalette` / `GetPalette` per window, entry colour / usage accessors. Activating a palette changes nothing: the
  display is 1 bit deep (honest absence of Color QuickDraw is kept: Glider PRO and Glider Edit show their own "Need Color
  Quickdraw" alert).
* **Smaller:** `GetKeys` (all up: releases are not tracked), `GetAppParms` + `AppParmHandle` (message 0, no documents), `Munger` /
  `PackBits` / `UnpackBits`, the Fixed / Fract / bit / `Random` utilities (`fixmath.rs`), `GetMouse` / `Button` / `StillDown` /
  `WaitMouseUp` / `Delay` (`mouse.rs`), the menu item accessors and `InsertMenuItem`, `GetIcon` / `GetPicture` / `GetPattern`,
  `Get/SetCRefCon`, `UpdtControl`, `LActivate`, `DrawDialog` / `UpdtDialog` / `CloseDialog` / `SelectDialogItemText`, `BringToFront`
  / `SendBehind` / `HiliteWindow` / `DrawGrowIcon` / `CloseWindow`.

Next walls (census): TextEdit (`TENew`, `TEStyleNew`: SimpleText 1.3/1.4, Toolbox Showcase), window regions
(WriteNow 2.0 / 2.2 call `PtInRgn` on NIL `contRgn`), `SetDialogItem` (a user item's draw procedure is a guest callback), PICT v2
(`DrawPicture` of Moving Van's pictures), the List Manager's scroll-bar controls (`vScroll` is NIL, so `SetCRefCon(NIL)` is
refused), `ModalDialog` filter procedures.
