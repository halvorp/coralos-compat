# The Mac translation layer (ruling 17 pieces 2, 3, 5, 8; shared by mac68k-engine and macppc-engine)

One design for every Toolbox program, 68k or PowerPC: the engine shows the program's OWN UI objects as semantic controls
through `guestui-abi`, shows its drawing as a display list, and turns native input back into the exact input the original
OS produced. Nothing here names an application or looks at what pixels look like.

## Layout (`crates/mac68k-engine/src`)

| file | what it is |
|---|---|
| `translate/mod.rs` | the generic mapper: `sync_projection` (add / diff-push / retire a native control), the single stale-intent check (token + generation), `restore_native` |
| `translate/controls.rs` | ONE table: `procID` -> role (`TABLE`), the attribute map into the real `ControlRecord`, part geometry, the Control Manager traps |
| `translate/dialogs.rs` | ONE table: DITL item type -> item (`ITEM_TABLE`), `DialogRecord` + DITL copy with item handles, `ModalDialog` and the item traps |
| `translate/input.rs` | pointer and key events for content (`mouseDown/Up`, `keyDown/autoKey`, low-memory `Mouse`/`MBState`) |
| `qd/plane.rs` | the retained display list per window port, pens as roles, `present_plane` once per step |

macppc-engine adds nothing but one native (`ModalDialog` parks the PowerPC caller on the front dialog's item hit,
`Mac::modal_ready`); every table, handler and record is the 68k engine's, reached through the Mixed Mode core.

## What an intent becomes (exact guest input)

| intent on | guest sees |
|---|---|
| push button `Activate` | `mouseDown` in the button (global `where`), `FindControl` -> `inButton` (10), `TrackControl` -> 10 |
| check box `SetValue(Checked)` / `Activate` | same with `inCheckBox` (11); the VALUE IS NOT CHANGED (the program toggles); the native toggle is put back until it does |
| scroll bar `ScrollTo(n)` / `Drag(Scroll)` | `mouseDown` on the thumb where it was (`inThumb` 129); `TrackControl` leaves `contrlValue = min + n` and returns 129 |
| dialog button / check box (any `Activate` / `SetValue`) | `itemHit` = the item number from `ModalDialog` |
| dialog edit text `CommitText(s)` | the text is in the item's handle (`GetDialogItemText` reads it); `ModalDialog` does not return |
| Return / Enter in a dialog | `itemHit` = `aDefItem` (1) |
| `Pointer` primary down / up | `mouseDown` / `mouseUp` at window + content origin; motion only moves the polled `Mouse` |
| `Pointer` secondary | `mouseDown` with `controlKey`; the middle button is a named refusal |
| `Key` down | `keyDown` (`autoKey` for the same key before its release), message = ASCII char; no character or non-ASCII: named refusal |

A stale or forged generation, a role that does not have the intent (`SelectRow` on a button), an inactive control: each is a
named refusal and produces NO guest input.

## Honest limits (all named in the log, none silent)

* `radioButProc` has no kit role yet (the closed vocabulary reserves "choice"): the control exists in guest memory, is not shown.
* A program's own CDEF, icon / picture / user dialog items: no contract to translate; they stay content (a user item is a plane).
* Press / Release / Drag gestures are not bridged: a complete click arrives as `Activate`.
* cicn / color icons are not decoded; `PlotIcon` of an `ICON` is recorded as the pixels it drew (a raw op).
* Text is drawn in the topaz stand-in in the bitmap; the display list carries the string, a font role and the point size.

## Measured, not claimed

`RecordingGuestUi::route_census(win)` reports native / replayed / raw per window. The tests pin it: a Control Manager window is
`native`, a QuickDraw window is one `replayed` plane, a window whose list holds any `Pixels` op (a `CopyBits`) is `raw`.
`tests/translate_planes.rs` replays the oracle application's list and requires it to agree with the bitmap on every pixel the
list names (and to name at least 60 % of them).
