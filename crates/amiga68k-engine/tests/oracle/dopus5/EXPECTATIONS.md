# Expectations the engine can test against (from the AROS m68k oracle, see README.md for the setup and caveats)

Times are emulated seconds on the oracle (not comparable); only ORDER and arguments matter. Pointers differ per OS.
Stub libraries (asl, diskfont, datatypes, commodities, amigaguide, rexxsyslib, locale, mathieee*trans/doubbas) answered
every call with 0, so behaviour that depends on their results is NOT ground truth.

## Startup call order (main process, from `RunCommand`)

1. `OpenLibrary("dos.library",0)`, `OpenLibrary("rexxsyslib.library",0)` (as `OldOpenLibrary` by the C runtime).
2. `GetProgramName`, `DupLock(progdir)`, `AssignLock("DOPUS5", lock)`, `Lock("D5THEMES:")` (fails) then
   `Lock("DOpus5:Themes")`, `AssignLock("D5THEMES", lock)`.
3. `OpenLibrary("PROGDIR:Libs/dopus5.library", 73)` (0x49). (With a wrong PROGDIR it fails and
   `DOpus5:Libs/dopus5.library` is tried next.)
4. dopus5.library `libinit` (a *separate* OpenLibrary burst, all version 37 = 0x25 unless noted), in this order:
   dos, utility, graphics, intuition, gadtools, asl, layers, diskfont, icon, workbench, datatypes,
   rexxsyslib (version 0), locale. Then main opens: locale 38, diskfont 36, commodities, datatypes (0), amigaguide (0),
   newicon (0, NULL is fine).
5. Main's own opens: dos 37, utility, graphics, intuition, gadtools, asl, layers, diskfont (36), icon, workbench,
   datatypes, rexxsyslib (0), locale (37), `OpenDevice("timer.device", unit 1)`, `OpenFont(topaz.font 8)`.
6. Five `GetVar("dopus/3DLook" | "dopus/OuEstLeMinibar" | "dopus/DOSPatch" | "dopus/FiletypeCache" | "dopus/NoSysPatch",
   size 2|14, flags 0x100)` (all fail, IoErr 205) then three `CreatePool` (puddle 0x518/0x418/0x1030).
7. **Nine `MakeClass` + `AddClass` pairs**, instance sizes include the superclass (`classPtr` 0, flags 0):

   | classID | superClassID |
   |---|---|
   | dopusiclass | imageclass |
   | dopusbuttongclass | gadgetclass |
   | dopusstrgclass | strgclass |
   | dopuscheckgclass | gadgetclass |
   | dopusviewgclass | gadgetclass |
   | dopuspalettegclass | gadgetclass |
   | dopusframeclass | gadgetclass |
   | dopusgaugeclass | gadgetclass |
   | dopuslistviewgclass | gadgetclass |
8. `CreateNewProc(NP_Name="DO_LAUNCHER", NP_StackSize=0x4000, NP_Cli=1, NP_Path=0, NP_WindowPtr=-1)`;
   `FindPort("Directory Opus")` (NULL: not yet running); `OpenDevice` input.device 0, timer.device 0 and 1,
   console.device -1; `LockPubScreen(NULL)`; `FindPort("WORKBENCH")`.
9. `CreateNewProc("dopus_progressbar", stack 0x2000)` -> that process: `LockPubScreen(NULL)`,
   `OpenWindowTagList`: Left 150 Top 100 Width 339 Height 55, `WA_Title="Directory Opus"`,
   `WA_WindowName="dOpUs5.win"`, WA_IDCMP 0x644, simple refresh, pubscreen = the Workbench screen.
10. `AddPort("Directory Opus")` (main), `CreateNewProc("dopus_rexx")` -> `FindPort("DOPUS.1")` (NULL) then
    `AddPort("DOPUS.1")`, timer.device unit 1.
11. Modules are opened as libraries with version 73 in the order cleanup, diskinfo, filetype, fixicons, ftp, join, misc,
    pathformat, themes, xadopus (all `PROGDIR:Modules/<name>.module`); `update.module` is tried first and is absent
    (NULL, both `PROGDIR:` and `DOpus5:` spellings), `version.library` 0 NULL. Each module's init opens
    dos, utility, graphics, intuition, gadtools, asl, layers, diskfont, icon, workbench, datatypes, rexxsyslib, locale
    (37), the four `mathieee{singbas,singtrans,doubbas,doubtrans}.library` and `dopus5.library` 73. Optional libraries
    that may be absent: newicon, codesets 6, xadmaster 7, SysInfo 2.
12. Main process: `OpenScreenTagList`: SA_LikeWorkbench, SA_Width/Height -1, SA_Depth 3, SA_Overscan 1,
    SA_Title "Directory Opus", SA_DisplayID 0x29000 (the Workbench mode), SA_PubName "DOPUS.1", SA_AutoScroll,
    SA_Interleaved, SA_SharePens, SA_SysFont 1, SA_Pens, SA_PubSig; then a backdrop borderless window
    Left 0 Top 15 Width 640 Height 241 (`WA_Backdrop`, `WA_ScreenTitle "Directory Opus"`, `WA_WindowName "dOpUs5.win"`,
    `WA_BackFill`, IDCMP 0x10da77e).
13. Processes created later (order): `dopus_buttons`, `dopus_lister` (the default Environment's first lister),
    `dopus_clock`, `dopus_pattern_stuff`, `dopus_wbstartup`; then `FindPort("REXX")` (NULL). A lister window:
    Left 209 Top 51, Width 443, Height 583 (see `trace/1-startup.txt`: WA_Left 0xd1, WA_Top 0x33, WA_Width 0x1bb,
    WA_Height 0x247, closegadget/dragbar/depth/size gadgets, custom screen = the DOpus screen).
14. Main then idles in `Wait` on its IPC port; screen title bar shows free memory:
    "Directory Opus  <chip> graphics  <fast> other" (set by `dopus_clock` via `SetWindowTitles(screenTitle=...)`, about
    once per second).

## Process list at idle
`<main: RunProg:launch>`, `DO_LAUNCHER`, `dopus_progressbar` (exits after startup), `dopus_rexx`, `dopus_buttons`,
`dopus_lister` (one per lister), `dopus_clock`, `dopus_pattern_stuff`, `dopus_wbstartup`.
Ports: `Directory Opus`, `DOPUS.1`.

## Screens (see `screens/`)
01 startup: custom screen "Directory Opus", disk icons on the backdrop, the SRCE lister (opened by the default
Environment) with the 14-button bank on top (tooltip text "LMB: Parent  RMB: root") and an empty path field.
02/03: double-clicking a disk icon opens another lister window (`dopus_lister`).
04: path field accepts typed text `DOpus5:` (no listing followed).

## Where the engine currently diverges (census on main a42623b)
`amiga68k-run --census ...DirectoryOpus` matches this trace step for step through item 6 (dos/rexxsyslib open,
AssignLock DOPUS5/D5THEMES, `PROGDIR:Libs/dopus5.library`, the libinit burst, newicon NULL, timer.device,
`OpenFont topaz.font`, the five `GetVar`s, the CreatePool triple) and first stops at **item 7: `intuition.library/
MakeClass` (REFUSED: unimplemented, NEXT-NEEDED)**. That is the boopsi lane. Items 8-14 are untested by the engine.
