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

* `radioButProc` is a check-box-shaped boolean part (a lone radio cannot be a `Choice`: it has no "none selected"; the app keeps the group), `popupMenuProc` (1008 + variation bits) is a `Choice{Popup}` over its MENU resource and a choice returns `inButton` with the value set, `_Pack0` is the List Manager (`translate/lists.rs`: a real `ListRec`, standard text LDEF shown as a native `List`, a custom LDEF stays a plane). A `procID` with no table row has no part at all: the control exists in guest memory and is DRAWN into the content plane as its default look (`fallback.rs`), so it is visible and the census counts it as `replayed`. A custom CDEF's own art is not run.
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
