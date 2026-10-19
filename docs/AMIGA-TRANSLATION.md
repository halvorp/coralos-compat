# amiga68k-engine: how a guest UI reaches the host (ruling 17, Amiga half)

One seam (`guestui-abi`), per-TOOLKIT tables, no application names (`tools/check-translation-seam.sh` is the gate).

## Intents back: `src/translate/`

* The generic mapper is the ONE `retro-translate` crate (shared with mac68k/macppc): the row tables (`Row { role, on, effect, post }`,
  `lookup(table, control kind, intent)`) and the projection registry (`Projections`: sync / retire / resolve, the one stale-intent
  rule). `translate/mod.rs` holds only the Amiga payloads (`Effect`, `Post`) and `message(row, Gadget.Activation, value)`.
  Native gadgets AND GadTools gadgets are both projected through `Projections<u32>` (`native_gads`, `gt_ctls`).
* `translate/boopsi.rs` is the Intuition / BOOPSI system-class table (what `native_gadgets` made into controls),
  `translate/gadtools.rs` the GadTools table. A row says: which guest state is written FIRST (`Effect`), then which IntuiMessage
  follows (`Post`: class, the Activation bit that gates it, the Code). Procedural effects (PropInfo pots, StringInfo, a GadTools
  kind's value) are the adapters' `apply` code next to `lookup`: `native_gadgets_input.rs` and `gt_kinds::control_changed`.
* `translate/mui.rs` is the MUI table (a row's guest effect is the list of `set` calls MUI's own classes make; see `docs/MUI-NATIVE.md`):
  MUI windows are projected through the same `Projections<u32>` (`Objects::proj`), with MUI's own layout over the real screen-font metrics.
* A role / intent pair without a row is a named refusal. Adding a gadget kind is adding rows, not a new code path.

## Requesters: one generic `request` (`requester.rs`)

* `EasyRequestArgs` / `AutoRequest` / `BuildEasyRequestArgs` / `BuildSysRequest` become `GuestUi::request(win, RequestModel { title, text,
  buttons, default, cancel })`; the answer is `GuestUiEvent::RequestAnswered { req, button }` (the index asked, so the seam carries no
  Amiga number). The ENGINE maps the index to Intuition's return: buttons 1..n-1 left to right, the rightmost 0, AutoRequest TRUE/FALSE,
  -1 + the stored class when an IDCMP class the caller listed closes it early (`Amiga::notify_requesters`).
* A synchronous call parks only the asking TASK (`Waiting::Request`); the async pair returns a real `struct Window` whose UserPort
  signals the answer and `SysReqHandler` consumes it. `FreeSysRequest` of an open one calls `GuestUi::close_request`.
* A host without alerts refuses by name and the guest gets 0, as for a requester that could not open. A headless run
  (`amiga68k-run`, `RecordingGuestUi::auto_cancel_requests`) dismisses every request with its Escape button.

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
