# amiga68k-engine: how a guest UI reaches the host (ruling 17, Amiga half)

One seam (`guestui-abi`), per-TOOLKIT tables, no application names (`tools/check-translation-seam.sh` is the gate).

## Intents back: `src/translate/`

* `translate/mod.rs` is the whole generic mapper: a `Row { role, on, effect, post }`, `lookup(table, control kind, intent)` and
  `message(row, Gadget.Activation, value)`. It knows no toolkit and no application.
* `translate/boopsi.rs` is the Intuition / BOOPSI system-class table (what `native_gadgets` made into controls),
  `translate/gadtools.rs` the GadTools table. A row says: which guest state is written FIRST (`Effect`), then which IntuiMessage
  follows (`Post`: class, the Activation bit that gates it, the Code). Procedural effects (PropInfo pots, StringInfo, a GadTools
  kind's value) are the adapters' `apply` code next to `lookup`: `native_gadgets_input.rs` and `gt_kinds::control_changed`.
* A role / intent pair without a row is a named refusal. Adding a gadget kind is adding rows, not a new code path.

## Content out: planes (`graphics/record.rs`, `retro-present/src/plane.rs`)

* The guest bitmap stays authoritative (ReadPixel / blits keep working). Next to it each window records a display list.
* A pen is a ROLE, found through the DrawInfo pen table the guest reads (`dri_Pens`: TEXT, SHINE, SHADOW, FILL, FILLTEXT,
  BACKGROUND, HIGHLIGHTTEXT). A pen number in no slot has no role: the call is a `Pixels` hole read from the canvas at flush. A write
  mask, COMPLEMENT / INVERSVID, an AreaPtrn, soft styles or an installed clip region also make a hole. Nothing is inferred from a picture.
* Named: `Text` (font role from `tf_Flags` FPF_PROPORTIONAL: Ui / Mono), `RectFill` / `EraseRect` / `SetRast` / `ClearEOL` (Fill),
  `Draw` / `DrawBorder` (Line), `PrintIText` (Text), a default icon's Image (Icon{role}; the host's artwork, see `icon_host.rs`).
  Runs of SHINE then SHADOW lines around one box coalesce into `Bevel`.
* Regions under native controls (registered gadgets and GadTools gadgets) are not published. `present_plane` once per step; a host that
  answers `Unsupported` gets the same damage as `present_content` (fallback, logged once).

## Plain input: `content_input.rs`

A pointer or key in content (no guest-drawn gadget owns it) is IDCMP_MOUSEBUTTONS / MOUSEMOVE / RAWKEY / VANILLAKEY with MouseX/MouseY
(window-relative), Qualifier (held buttons, shift/ctrl/alt/command) and the virtual clock; `Window.MouseX/MouseY` follow either way.
The right button is the menu system's unless the window sets WFLG_RMBTRAP; MOUSEMOVE needs WFLG_REPORTMOUSE. `Key.code` is taken as the
Amiga raw key code (0..=0x7F); anything else is refused by name until the host and the engine agree on a key table.
