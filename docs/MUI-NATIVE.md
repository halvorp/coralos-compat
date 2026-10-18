# muimaster.library (MUI) translated natively onto guestui

Lane: `amimon-mui.020` (unmodified, MUI 3.8 front-end of amimcp) opens its window as GuestUi calls exactly like the
GadTools `amimon` does, updates its rows with `set()`, and quits on a native close. Nothing is drawn by MUI code: the
MUI object tree is a HOST-side model, the pixels are the native UI's.

## How the compiled program calls MUI (read off `m68k-linux-gnu-objdump -D` of amimon-mui.020)
- `ApplicationObject, ... End` etc. are `muimaster.library/MUI_NewObjectA(a0 = "Application.mui", a1 = taglist)`;
  `Label(x)` is `MUI_MakeObjectA(d0 = MUIO_Label, a0 = {text, flags})`; `MUI_DisposeObject(a0)`.
- `set(obj, tag, v)` is amiga.lib `SetAttrs` -> `intuition.library/SetAttrsA`; `get(obj, tag, &v)` is `GetAttr`.
- `DoMethod(obj, id, ...)` is amiga.lib CODE INSIDE THE GUEST: `a2 = obj; a0 = *(obj - 4)` (`OCLASS`, `_Object.o_Class`);
  `a1 = &msg`; it then does `move.l 8(a0), -(sp); rts`, i.e. jumps to `((struct Hook *)class)->h_Entry`.

## Design
- **Guest-visible object**: each object is a 16-byte heap block; the object pointer is block+12, `*(obj-4)` is a
  pointer to a per-kind `struct IClass` in a fixed region (0xE000, no heap), whose `Hook.h_Entry` (offset 8) is ONE
  shared `ILLEGAL; RTS` trap slot (`boopsi.rs`). The guest's own DoMethod therefore lands in the host with
  a0 = class, a1 = msg, a2 = object, exactly as a real BOOPSI dispatcher would be entered. Only `h_Entry`/`h_Data`
  of the class struct are filled; `cl_Super`, `cl_ID`... are zero (nothing reads them in the corpus).
- **Object state is host-side** (`boopsi::Objects`): kind, stored attributes (numbers; strings are copied at
  set time), ordered children, parent, notifications, and per-kind extension (application signal + pending return
  IDs; native window ref; native control ref). The guest cannot corrupt it. `MUI_DisposeObject` of a root frees the
  whole tree, closes its window, and releases the application's signal bit.
- **Attribute tables** (`mui/ids.rs`): ids/types are interface facts from the MUI 3.8 developer-kit `mui.h` (the
  header amimon-mui compiled against; also Zune's mui.h). An attribute the kind does not list is a NAMED refusal
  (NewObject returns NULL, SetAttrs refuses), never ignored.
- **Mapping**: Window -> `GuestUi::open_window` when `MUIA_Window_Open` becomes TRUE (size = layout of the content,
  `mui/layout.rs`; title; closable/resizable); Group/ColGroup/HGroup/VGroup -> layout only; a `Label` object
  immediately followed by a `Text` object in a group becomes ONE `ControlKind::Label { text, framed: true }` whose
  `label` is the label text; `set(text, MUIA_Text_Contents)` -> `set_control`; Menustrip/Menu/Menuitem ->
  `set_menu_strip` (token = the Menuitem object).
- **Events**: `Amiga::deliver` offers the event to `mui::try_deliver` first. `CloseRequested` sets
  `MUIA_Window_CloseRequest` = TRUE on the window object, `MenuCommand` sets `MUIA_Application_MenuAction` = the
  item's `MUIA_UserData` on the application. Setting an attribute fires its `MUIM_Notify` records (every set; the
  trigger value is matched, `MUIV_EveryTime` matches all; `MUIV_TriggerValue` in the follow-up message is replaced).
  amimon-mui's notifies call `MUIM_Application_ReturnID`, which queues the id and Signals the task.
- **`MUIM_Application_NewInput(&sigs)`**: pops one pending return id (returned in d0; 0 when none). `*sigs` = the
  application's signal mask, or 0 when more ids are still pending (MUI's "do not Wait yet" convention). The guest then
  `Wait()`s on the existing resumable wait; a delivered event signals that bit and wakes it.
- Unimplemented classes, attributes, methods, `MUIO_*` types and `MUI_*` functions are named refusals
  (`muimaster.library/MUI_RequestA: unimplemented` etc.).

## Not done (named)
Rows/pages/virtual groups, Area frames/backgrounds beyond storing them, `MUI_RequestA` (About box),
`MUIM_Application_OpenConfigWindow`, the Rectangle class, and changing a window title after open (GuestUi has no verb).
