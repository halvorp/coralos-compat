# The retro-compat corpus: real binaries, census results, and the toolkit usage matrix

Peter, 2026-10-18: "Do not just focus on DOpus, but the entire Amiga UI/MUI and test more apps! Also check how this will work
with MacOS apps, both m68k and PPC." This file is the evidence behind the per-TOOLKIT translation tables of
`docs/retro-compat/17-TRANSLATION-ARCHITECTURE.md` (in the CoralOS repo): which apps exist to test with, how far the engines get
on each today, which toolkit rows each one uses, and therefore which table rows matter first.

**No third-party binary is in this repository.** Downloads live under `/data/build/corpus-bin/<app>/` (untrusted data, one
directory per source), each recorded with URL, version, sha256 and licence in `/data/build/corpus-bin/CORPUS.tsv` (reproduced
in the table below). `tests/census_corpus.rs` in `amiga68k-engine`, `mac68k-engine` and `macppc-engine` drive them and SKIP loudly when a binary is absent.

## How this was produced (all reproducible)

```
tools/corpus-census.py APPS.tsv --out census.json     # amiga68k-run --census per app; keeps going with --stub to record the NEXT-NEEDED chain
tools/corpus-mui.py [--json out.json]                 # only the MUI programs: first NEXT-NEEDED, named refusals, native controls per window (route census)
tools/corpus-strings.py APPS.tsv > strings.json       # static name evidence: libraries, MUI classes/MCCs, BOOPSI classes in the hunk
tools/corpus-census-mac.py MAC_APPS.tsv --out mac.json # mac68k-run / macppc-run --census
tools/corpus-traps-mac.py FILE...                     # static: resource types + Toolbox traps in CODE, by manager
tools/corpus-report.py census.json strings.json [mac.json]   # renders the tables below
```

"Name evidence" is a string found in the executable: a class named there may be unused and an indirectly built class is missed.
"Created at run time" comes from the engine's own log of the run and is only as deep as the run got (most runs stop early, see
the findings). Read both columns together; neither is a call graph.

## The Amiga corpus

64 binaries are censused: 53 from the AROS nightly, 11 classic AmigaOS ones (3 amimcp tools, iGame, 3 Directory Opus programs, YAM, AWeb, AWebCfg, SnoopDos).

* **AROS m68k nightly 20261018** (`amiga-m68k-boot-iso.zip`, APL): the system's own Workbench programs. 43 of the 53 are
  Zune/MUI programs (Prefs editors, System/, Tools/, Utilities/Clock, Installer...), 10 GadTools/Intuition/BOOPSI ones
  (MultiView, More, Editor, Workbook, AHIRecord, gtdemo, menudemo...). They are the broadest MUI sample there is, and the only one built
  against Zune (AROS's MUI).
* **Classic AmigaOS 3.x binaries**: YAM 2.9p1 (GPL-2, MUI + the NList/TextEditor/TheBar MCCs), AWeb 3.6b8 (AWeb Public License 1.0,
  ReAction-style `layout.gadget`), SnoopDos 3.11 (freely distributable, GadTools + custom), Directory Opus 5.102 (APL,
  nine own BOOPSI classes), iGame (GPL-3, MUI + TextEditor.mcc), amimcp (Apache-2.0).
* **Not usable**: the AROS m68k *contrib* archive (10,213 files; its programs, e.g. `C/UnZip`, `AmiChess`) are ELF 32-bit MSB
  *relocatable* objects (`ET_REL`), not Hunk executables, so the engine cannot load them (an AROS ELF loader would unlock BGUI, MCC_TheBar, Scout, games, AMP2...).
  SimpleMail, FastForward and Legadon publish no m68k binary release; FileX-68K has no licence text (skipped).

### Running the AROS programs (the `mini` volume)

AROS-built programs need AROS's own `stdc.library`, `stdcio.library`, `posixc.library` (rellibs) from `LIBS:`. The engine bridges
the standard libraries itself and prefers a disk file of the same name, so the volume must NOT contain the ones it bridges:

```
cd /data/build/corpus-bin/aros-m68k           # nightly iso extracted with 7z into sys/
for d in Libs Classes Devs S L Locale; do cp -rl sys/$d mini/$d; done       # hard links, ~63 MB
cd mini/Libs && rm -f locale.library muimaster.library asl.library commodities.library diskfont.library \
  cybergraphics.library datatypes.library iffparse.library mathieee*.library rexxsyslib.library   # the engine's own
amiga68k-run --census --sys mini --assign LIBS=SYS:Libs sys/Utilities/Clock
```

The classic binaries get the MCC classes YAM ships (`/data/build/corpus-bin/mccs/Libs/mui/*.mcc`: NList, NListtree, TextEditor,
BetterString, TheBar; LGPL-2.1) as `LIBS:mui/` through `--mount MCC=... --assign-add LIBS=MCC:Libs`.


### Sources, versions, checksums, licences (`/data/build/corpus-bin/CORPUS.tsv`)

| app | version | source_url | sha256_of_download | licence | notes |
|---|---|---|---|---|---|
| aros-m68k system (Utilities/Prefs/Tools/System/Demos/C/Libs/Classes) | nightly 20261018 amiga-m68k boot-iso | https://sourceforge.net/projects/aros/files/nightly2/20261018/Binaries/AROS-20261018-amiga-m68k-boot-iso.zip | c5eaff3ebffba3ef0cddeab2f7d74cf500c9b8c071b602f69fd7d1d1e615eb3f | AROS Public License (APL), file LICENSE in the archive | extracted with 7z; HUNK executables carrying HUNK_RELRELOC32 (0x3FD); need task.resource + stdc.library |
| aros-m68k contrib | nightly 20261018 amiga-m68k contrib | https://sourceforge.net/projects/aros/files/nightly2/20261018/Binaries/AROS-20261018-amiga-m68k-contrib.tar.bz2 | 42058eb8e7a5477126d6f7547b2d16f0e5c4d037a2a13f262abbb5ee36525337 | per-package (see ACKNOWLEDGEMENTS in the archive) | ELF 32-bit MSB RELOCATABLE objects, not HUNK: not runnable by the engine (no ELF/AROS loader); downloaded for the inventory only |
| YAM | 2.9p1 AmigaOS3 | https://github.com/jens-maus/yam/releases/download/2.9p1/YAM29p1-AmigaOS3.lha | 2df011722d29bb9a7bcb76f7176c60e85370957ca29d59d1532cdd6177054e6c | GPL-2.0 (repo COPYING) | MUI; ships NList/NListtree/TextEditor/BetterString/TheBar MCCs (LGPL-2.1) in Install/MUI |
| AWeb | 3.6b8 | https://github.com/amigazen/AWeb3/releases/download/AWeb3.6b8/AWeb36b8.lha | 85232ba1f82a3cfa6aab0469ea0994df9e9b8927a7d5bace37419bd3c19bf0a8 | AWeb Public License 1.0 (repo LICENSE) | ReAction-style layout.gadget; not MUI |
| SnoopDos | 3.11 | https://aminet.net/util/moni/SnoopDos.lha | 0fb51361fe82b125971b190b78a81cb43c0a631c9d075a37b0110a0617612aba | Freely distributable (Eddy Carroll, readme) | GadTools + custom; test-only, never redistributed here |
| amimcp (amiagent/amimon/amimon-mui) | 0.13.2 | https://github.com/thomas-luebker/amimcp (release) | d893cabeb50e49678f47bbad192a0c921bde0ee410d61ee65b1724b05430fae3 | Apache-2.0 (repo LICENSE) | already in the corpus before this lane |
| iGame | 2.6.x (extracted tree) | https://github.com/MrZammler/iGame/releases | n/a (archive not retained) | GPL-3.0 (repo LICENSE) | MUI + TextEditor.mcc; already in the corpus |
| Directory Opus 5 (Magellan) | 5.102 OS3 | https://github.com/BlitterStudio/dopus5/releases | ea6f59d23a9256227781824b24e78b67642fd1fababd94c01829e458e2f98cfa | AROS Public License (repo COPYING / THIRD_PARTY_NOTICES) | own BOOPSI classes; already in the corpus |
| Executor packages/go (go.appl, icontest.appl) | tree of /data/refs/retro/executor | https://github.com/autc04/executor | 5b81b2047bb186da9c8e4ee9d6357a33a052ab01757476d1240dfca0179d776d (go) fa1e5a7186686aa063787163fd9d42a5057105b07916aa1d3703b241d39be9a2 (icontest) | MIT (Executor tree) | AppleDouble header files holding the resource forks |
| Retro68 Samples + the toolchain that built them | Retro68 afedcdc (autc04/Retro68) + multiversal 057c802, GCC 16.1.0 for m68k-apple-macos and powerpc-apple-macos | built locally: /data/build/retro68 (log /data/build/retro68/build.log, host tools build-host.log); outputs copied to /data/build/corpus-bin/mac-retro68/{68k,ppc}/ (16 .bin MacBinary files, hashes in mac-retro68/SHA256SUMS) | see mac-retro68/SHA256SUMS | toolchain GPL-3; the Samples sources are Retro68 (GPL-3 with the GCC runtime exception for the linked runtime); outputs are test data only, never redistributed | prerequisites installed locally without sudo: Boost 1.86.0 (built into /data/build/retro68/boost), texinfo stubbed (docs not built) |
| Skipped: FileX-68K 2.7a | 2.7a | https://aminet.net/disk/moni/FileX-68K.lha | 3b78a191c875ad560f71fee798d29d97048c4bf609af40270332cc529a66610a | UNCLEAR ("Open Source Team", no licence text in readme) | downloaded, NOT used (instruction: skip unclear licences) |
| Skipped: SimpleMail | - | https://github.com/sba1/simplemail | - | GPL-2.0 | repo publishes no m68k release asset; Aminet name search found no SimpleMail binary |
| Skipped: FastForward, Legadon | - | https://github.com/amigazen/FastForward , https://github.com/alb42/Legadon | - | - | no release assets; Legadon's package/ directory holds only the .info icon |
| systemless Toolbox Showcase (fat app, forks cut out of the stored SIT5) | tests/toolbox-showcase of benletchford/systemless | /data/refs/retro/systemless/tests/toolbox-showcase/toolbox-showcase.sit | 373a6a1e268b558228d4bd911b3982557c32d52d75ff9bccbb65eff89c1eb254 (archive); showcase.rsrc 3021fd64872986a0c9903adbbca6fcb773cf7883e8b231d9ece60c7c069197bf; showcase.pef db1ddcad335d9ccc3b5d1fec654273b84c78fcab7b47f59c28dfda008fd77472 | GPL-3 (systemless); test data only, never committed | tools/corpus-extract-showcase.py cuts the forks out |

## What the census found and fixed (all in this branch, each with a test and a negative control)

The first run of the engine over the AROS binaries stopped at the loader. Each fix below moved the whole set one step; the
"before" is the count of the 53 AROS programs that stopped there.

1. **`HUNK_RELRELOC32` (0x3FD)** in `retro-hunk` (53 of 53 refused at load: "unsupported hunk type 0x3fd"). Word-encoded like
   RELOC32SHORT; the field gains `base[target] - base[this hunk]` (AROS `rom/dos/internalloadseg_aos.c`).
2. **`exec.library/OpenResource` + AROS `task.resource`** (UNIMPLEMENTED within the first 12 instructions of every AROS program;
   then `stdc.library`'s init returned NULL in 50 of 53). New `task_resource.rs`: Alloc/Free/Set/Get storage slots per task, InitTaskHooks/AddTaskHook/RunTaskHooks.
   Running a *registered* hook, Save/RestoreTaskStorage and a created task's parent slot are named refusals. Any other resource
   name is NULL, as on real exec.
3. **`timer.device/GetSysTime`/`GetUpTime`** on the virtual clock (21 stdc-only programs stopped there) and **`aros.library` v41**
   (29 `posixc.library` programs stopped asking for it; `LibraryBuilder::version`).
4. **A disk library that gives every opener its own base** (AROS rellibs build one with `MakeLibrary` in `Open`; 45 of 53 then
   ended with "CloseLibrary: 0x... is not an open library base"). `DiskLib.openers` + Close with A6 = the base passed.
5. **dos `SetVBuf`** (30 of 53, the stdcio startup) and **ExecBase `lib_Version`** (was 0; AmigaOS 3.1 = 40.68).

With these, AROS-built programs run their real startup (`task.resource`, three rellibs, locale, dos), and **`gtdemo` creates its
real GadTools `BUTTON_KIND` gadget** and `Clock` builds its MUI application tree. This is the first time the engine has run
AROS-built code at all; it is the only source of MUI/Zune programs, and AROS-built binaries are 53 of the 64 in the corpus.

## Where each group stops now (the next work, ranked by apps blocked)

| gap | apps | owner lane |
|---|---|---|
| ~~`dos Open("NIL:")` is "not modelled yet", then `Close(0)` ("not an open file") in stdcio/posixc cleanup~~ DONE (mui2 lane) | 30 / 26 | dos |
| MUI custom classes: ~~`muimaster/MUI_CreateCustomClass`~~ DONE (mui2 lane); disk classes `Clock.mcc`, `SystemPrefsWindow.mcc`, `PrefsEditor.mcc`, MCCs from `LIBS:mui/` remain | 16 + 8 | mui |
| ~~MUI attributes~~ DONE (mui2 lane): `MUIA_Application_SingleTask` 0x8042a2c8 (10 apps), `_DiskObject` 0x804235cb, `_BrokerPri` 0x8042c8d0, `_UseCommodities` 0x80425ee5, `_UsedClasses` 0x8042e9a7, `MUIA_Window_Activate` 0x80428d2f, `_Window_AppWindow` 0x804280cf, `MUIA_Group_PageMode` 0x80421a5f (the Register/page layouts), `MUIA_Frame` | 3-10 each | mui |
| ~~`MUI_MakeObjectA` types other than `MUIO_Label`~~ DONE (mui2 lane): 2 `Button` (6 apps), 13 `MenustripNM` (4), 3 `Checkmark`, 4 `Cycle`, 8 `PopButton` (2 each) | 6+4+2+2+2 | mui |
| ~~`intuition/EasyRequestArgs` (3), `DupLockFromFH` on the console handles (3), `iffparse/AllocIFF` (2), `exec/RawPutChar` (2)~~ DONE (amilibs lane, see below); `WindowAction` (1), `MUI_RequestA` (2, MUI's own requester class) remain | 1-3 each | intuition / dos |
| YAM: `AllocDosObject type 5` (DOS_CLI), `locale/ParseDate` with a hook | 1 | dos / locale |

### The `amilibs` lane (2026-10-19): requesters, iffparse, keymap, commodities commands, aros.library

Landed (`docs/AMIGA-TRANSLATION.md` has the requester seam): `intuition` EasyRequestArgs / BuildEasyRequestArgs / SysReqHandler /
FreeSysRequest / AutoRequest / BuildSysRequest as native alerts; `iffparse.library` in full over dos file handles (AllocIFF ...
ParseIFF SCAN/STEP/RAWSTEP, PushChunk/PopChunk, Prop/Collection/Stop, guest entry/exit/purge hooks; clipboard and guest stream hooks
are named refusals); `keymap.library` with a real US KeyMap; `commodities` BrokerCommand + CxMsgType/ID/Data; `exec` RawPutChar (engine
log); `aros` ArosInquireA (facts only); `dos` DupLockFromFH / ParentOfFH on the console handle (ERROR_ACTION_NOT_KNOWN).

One-pass census (`amiga68k-run --census`, 64 apps, same inputs, the binary of the base commit e814e29 against this branch):

| | before | after |
|---|---|---|
| apps whose run reaches no unimplemented function | 50 | 61 |
| windows opened (apps that opened one) | 29 (25) | 31 (27) |

| app | NEXT-NEEDED before | after | what the run does now |
|---|---|---|---|
| YAM | intuition/EasyRequestArgs | none | asks its requester (dismissed with Escape by the headless runner), then exits |
| AWebCfg | intuition/EasyRequestArgs | none | the same |
| aros-AHIRecord | intuition/EasyRequestArgs | none | the same |
| aros-Wanderer | intuition/EasyRequestArgs (+3 refusals) | none (0 refusals) | asks, then exits |
| aros-KeyShow | keymap/AskKeyMapDefault | none | **opens its window** (the first from this lane) |
| aros-prefs-OpenURL | iffparse/AllocIFF | none | **opens its window** |
| aros-prefs-Editor, aros-Editor | iffparse/AllocIFF | none | iffparse answers (no prefs file: OpenIFF fails); Editor then ends without a log line before its window (cause not in the log), prefs-Editor stops at `OpenWindow: size 0x0` |
| aros-Exchange | commodities/BrokerCommand | none | BrokerCommand answers -1 (no Exchange); it then prints "Can't create CxBroker object!": **`MUIA_Application_Broker` (a MUI Application that makes its CxBroker, BrokerPort, BrokerHook) is the next gap** |
| aros-About | aros/ArosInquireA | none | ArosInquireA answers; the About window needs MUI string attributes it reads past 256 bytes (`string at 0x12447 unterminated`) |
| aros-gtdemo | exec/RawPutChar | none | RawPutChar to the log; gtdemo stops at GadTools `PALETTE_KIND` (named refusal) |
| aros-VMM | exec/RawPutChar | muimaster/MUI_RequestA | past RawPutChar, now at MUI's requester class (`MUI_RequestA`, MUI lane) |

And three things about the *classic* binaries:

* **MUI apps ship their own MCC classes as disk libraries** (`LIBS:mui/TextEditor.mcc`...). iGame loads `TextEditor.mcc` v15 and
  stops at `MUI_CreateCustomClass`: a custom-class host for MUI is on the critical path exactly like DOpus's `MakeClass`, and
  doc 17's "app-defined classes inherit the contract of their nearest standard ancestor" must be implemented for MUI *and*
  BOOPSI (`dopusstrgclass` < `strgclass`, a TextEditor < `Area`).
* **AWeb needs a ReAction class set** (`OpenLibrary("gadgets/layout.gadget")`; AWebCfg and the OS 3.2 style apps use
  `window.class`, `layout.gadget`, `button.gadget`...). That is a fourth toolkit layer, not covered by the doc 17 tables.
  AROS ships `Classes/window.class` (found in the nightly) so there is a reference.
* **Directory Opus is the only binary that reaches an event loop with real UI today** (4 windows, 60 content planes, 3 menu
  strips), through nine own BOOPSI classes over `gadgetclass`/`imageclass`/`strgclass`, `NewObjectA` and `CreateMenusA`: the plane
  and binding path of doc 17 is what makes it usable, not the MUI table. SnoopDos prints "I need AT LEAST Kickstart 2.0" and quits
  (a version gate that is not ExecBase's `lib_Version`; not yet identified).

The AROS m68k *contrib* archive is ELF relocatable (`ET_REL`), not Hunk: an AROS ELF loader would unlock 10,213 files (BGUI,
MCC_TheBar, Scout, MultiMedia, games...) and is worth a lane of its own.

## The MUI lane (2026-10-18, branch push-mui2): the 47 MUI programs before and after

`tools/corpus-mui.py` runs every MUI row of `corpus-apps.tsv` once (`amiga68k-run --census`) and reports the first unimplemented call, the
first named refusals and, per window, which controls reached the seam (the route census: native roles; no program here draws a content
plane yet). "Before" is compat main 4844fac, "after" this branch. The MUI work is `docs/MUI-NATIVE.md`; the dos fix (`Open("NIL:")`,
`Close(0)`) moved about 30 programs past their C-library startup.

| app | NEXT-NEEDED before | NEXT-NEEDED after | window before | window after (native roles) | first named gap left |
|---|---|---|---|---|---|
| amimon-mui | none | none | opened | opened: 8 Label |  |
| iGame | muimaster.library/MUI_CreateCustomClass | none | no | no |  |
| YAM | none | none | no | no | ParseDate: the hook 0x14 has no h_Entry |
| aros-Clock | none | none | no | no | MUI class "Clock.mcc" is not implemented |
| aros-Installer | none | none | no | no |  |
| aros-Calculator | none | none | no | opened: 20 Button, 1 Label |  |
| aros-KeyShow | muimaster.library/MUI_CreateCustomClass | keymap.library/AskKeyMapDefault | no | no |  |
| aros-HDToolBox | none | none | no | no |  |
| aros-ScreenGrabber | muimaster.library/MUI_RequestA | none | no | opened: 4 Button, 4 Group, 3 Label, 1 List, 1 Slider, 1 Checkbox, 1 Field, 1 Gauge |  |
| aros-SysExplorer | none | none | no | no |  |
| aros-WiMP | none | none | no | opened: 13 Button, 1 Group, 1 List |  |
| aros-Exchange | commodities.library/BrokerCommand | commodities.library/BrokerCommand | no | no |  |
| aros-ASCIITable | muimaster.library/MUI_CreateCustomClass | none | no | no |  |
| aros-FKey | none | none | no | no | MUI class "HotkeyString.mcc" is not implemented |
| aros-QuickPart | none | none | no | no |  |
| aros-Find | muimaster.library/MUI_CreateCustomClass | intuition.library/EasyRequestArgs | no | no | MUI class "NList.mcc" is not implemented |
| aros-Format | none | none | no | no |  |
| aros-SysMon | muimaster.library/MUI_DeleteCustomClass | none | no | no |  |
| aros-VMM | exec.library/RawPutChar | exec.library/RawPutChar | no | no |  |
| aros-Snoopy | none | none | no | opened: 29 Checkbox, 6 Button, 4 Group, 4 Slider, 1 Field |  |
| aros-About | muimaster.library/MUI_CreateCustomClass | aros.library/ArosInquireA | no | no |  |
| aros-Wanderer | intuition.library/EasyRequestArgs | intuition.library/EasyRequestArgs | no | no | DupLockFromFH on 0x6300: not an open file |
| aros-Info | none | none | no | no |  |
| aros-DiskInfo | muimaster.library/MUI_CreateCustomClass | none | no | no |  |
| aros-WBRename | none | none | no | no | DisposeObject: 0x0 is not a MUI object (or already disposed) |
| aros-prefs-Appearance | muimaster.library/MUI_CreateCustomClass | none | no | no | MUI_CreateCustomClass: the superclass "PrefsEditor.mcc" is not impleme |
| aros-prefs-Font | muimaster.library/MUI_CreateCustomClass | none | no | no | MUI_CreateCustomClass: the superclass "PrefsEditor.mcc" is not impleme |
| aros-prefs-Palette | muimaster.library/MUI_CreateCustomClass | none | no | no | MUI_CreateCustomClass: the superclass "PrefsEditor.mcc" is not impleme |
| aros-prefs-Input | muimaster.library/MUI_CreateCustomClass | none | no | no | MUI_CreateCustomClass: the superclass "PrefsEditor.mcc" is not impleme |
| aros-prefs-Time | none | none | no | no |  |
| aros-prefs-ScreenMode | muimaster.library/MUI_CreateCustomClass | none | no | no | MUI_CreateCustomClass: the superclass "PrefsEditor.mcc" is not impleme |
| aros-prefs-Zune | none | none | no | no |  |
| aros-prefs-Locale | none | none | no | no | OpenLibrary "codesets.library": LIBS:codesets.library has no RomTag in |
| aros-prefs-Editor | iffparse.library/AllocIFF | iffparse.library/AllocIFF | no | no |  |
| aros-prefs-Pointer | muimaster.library/MUI_CreateCustomClass | none | no | no | MUI_CreateCustomClass: the superclass "PrefsEditor.mcc" is not impleme |
| aros-prefs-IControl | muimaster.library/MUI_CreateCustomClass | none | no | no | MUI_CreateCustomClass: the superclass "PrefsEditor.mcc" is not impleme |
| aros-prefs-Wanderer | muimaster.library/MUI_CreateCustomClass | none | no | no | MUI_CreateCustomClass: the superclass "PrefsEditor.mcc" is not impleme |
| aros-prefs-ReqTools | muimaster.library/MUI_CreateCustomClass | none | no | no | MUI_CreateCustomClass: the superclass "PrefsEditor.mcc" is not impleme |
| aros-prefs-OpenURL | muimaster.library/MUI_RequestA | muimaster.library/MUI_RequestA | no | no | OpenLibrary "openurl.library": LIBS:openurl.library has no RomTag in i |
| aros-mui_notify1 | none | none | no | no |  |
| aros-mui_layout1 | none | none | no | opened:  |  |
| aros-mui_pages | none | none | no | opened: 5 Button, 2 Group, 1 Label |  |
| aros-mui_slider | none | none | no | opened: 1 Slider |  |
| aros-NList-Demo | muimaster.library/MUI_CreateCustomClass | none | no | no | MUI class "NList.mcc" is not implemented |
| aros-NListtree-Demo | none | none | no | no | MUI class "NListtree.mcc" is not implemented |
| aros-RequestString | none | none | no | opened: 1 Label, 1 Field |  |
| aros-MountHDF | none | none | no | no |  |

windows opened: 1 of 47 before, 9 of 47 after


What stops the programs that open no window now (all NAMED; none is a MUI-table gap):
* **Disk MCC classes** (`LIBS:Zune/*.mcc`, `MCC_Query`): the eleven Prefs editors ("Could not create Zune custom class `XEditor'": their
  superclass is `PrefsEditor.mcc`), `Clock.mcc`, `HotkeyString.mcc`, `NList*.mcc`, YAM/iGame's MCCs. Loading a disk class is OpenLibrary +
  `MCC_Query(0)` as guest calls from inside `MUI_NewObjectA` / `MUI_CreateCustomClass`: the next MUI step.
* **Libraries the volume lacks** (not MUI): `render.library` (iGame), `uuid.library` and partition libraries (HDToolBox, QuickPart, MountHDF,
  SysExplorer), `muiscreen.library` (Prefs/Zune), `codesets.library`, `openurl.library`, `dos64.library` v50 (the AROS-only 64-bit dos).
* **`exec.library` v50** (Format, SysMon, Prefs/Time refuse to start: `Requires exec.library v50, found v40`): AROS's exec ABI version; the
  engine reports AmigaOS 3.1 (40). Reporting 50 would make every program take AmigaOS 4 paths, so it is left to the exec lane. Also
  `exec/VNewRawDoFmt` behind it.
* Other lanes: `intuition/EasyRequestArgs` (Find, Wanderer), `aros/ArosInquireA` (About), `keymap/AskKeyMapDefault` (KeyShow),
  `commodities/BrokerCommand` (Exchange), `iffparse/AllocIFF` (Prefs/Editor), `MUI_RequestA` (Prefs/OpenURL), `exec/RawPutChar` (VMM),
  `locale/ParseDate` with a hook and `AllocDosObject` type 5 (YAM).
* mui_notify1 is not a window program: it runs the Notify class and prints; `tests/mui_real.rs` checks its output.

## The top rows the translation layer must cover first

Ranked by how many corpus binaries use the row (name evidence over 64 Amiga binaries, plus the runs; full ranking of 21 rows in
"The data" below). Layout-only classes (`Rectangle`, `Balance`) and no-UI libraries (commodities) are left out of the list: they
need no role.

**Amiga, in order:**

| # | table row | apps | role | note |
|---|---|---|---|---|
| 1 | MUI `Application` + `Window` (shell, title, close, menu strip, `Notify`) | 44 | window | every MUI app; needs the attribute list above (`Activate`, `AppWindow`, `SingleTask`, `UseCommodities`...) |
| 2 | MUI `Group` (H/V/virtual, frame, title, **PageMode**) | 43 | group | PageMode is the Register/prefs page layout |
| 3 | MUI `Text` + `MUI_MakeObjectA` (`MUIO_Label`, `Button`, `Checkmark`, `Cycle`, `PopButton`) | 33 | label / button / checkbox / choice | only `MUIO_Label` exists today |
| 4 | MUI `String` / `BetterString` | 26 | field | `String.mui` is refused by name in `RequestString` |
| 5 | MUI `List` / `Listview` / `NList` / `NListtree` / `IconList` | 23 | list | `NList` is an MCC: needs the custom-class host |
| 6 | ASL requesters + MUI `Popasl` | 22 | modal dialog | file/font/screenmode; a native picker, not a drawn one |
| 7 | MUI `Slider` / `Gauge` / `Scale` | 17 | slider / gauge | |
| 8 | **GadTools kinds** (`CreateGadgetA`; seen: TEXT, BUTTON, STRING, CYCLE, SLIDER, MX, INTEGER, LISTVIEW, PALETTE, GENERIC) | 15 | button / field / choice / slider / list | the classic apps (SnoopDos 9 buttons, DOpus, AWebCfg); AROS tools too |
| 9 | MUI `Register` (tabs) | 11 | choice (tabs) | |
| 10 | MUI `Image` / `Dtpic` / `IconImage` / `Pixmap` | 11 | image | |

Next five: MUI prefs-window family `PrefsEditor`/`SystemPrefsWindow` (11, window + groups, an MCC), BOOPSI system gadget/image
classes (`propgclass`, `sysiclass`, `buttongclass`, `strgclass`, `frameiclass`; 9), MUI `Scrollgroup`/`Virtgroup`/`Scrollbar` (9),
MUI menus `Menustrip`/`Menu`/`Menuitem` + GadTools menus (8), MUI `Cycle`/`Radio` (8); then `EasyRequest` (3 reached, 5 named),
`TextEditor` (3), and custom BOOPSI classes over a standard ancestor (DOpus, 2 apps but the most complex window in the corpus).

## The data

### Amiga census, one row per binary

| app | toolkit | ran | NEXT-NEEDED (first) | window UI calls | refused |
|---|---|---|---|---|---|
| amiagent | CLI | 15,281 insns | none | - | - |
| amimon | GadTools | 50,000,156 insns (budget, still running) | none | OpenWindowx1, AddControlx8 | - |
| amimon-mui | MUI | 50,000,154 insns (budget, still running) | none | OpenWindowx1, AddControlx8, SetMenuStripx1 | - |
| iGame | MUI | 553 insns | muimaster.library/MUI_CreateCustomClass | - | - |
| DirectoryOpus | dopus-own | 50,000,046 insns (budget, still running) | none | OpenWindowx4, PresentContentx60, SetMenuStripx3, AddControlx8 | - |
| dopus-viewfont | dopus-own | 9,422 insns | intuition.library/QueryOverscan | - | - |
| DOpusRT5 | CLI | 1,329 insns | none | - | - |
| YAM | MUI | 88,428 insns | none | - | dos.library/AllocDosObject AllocDosObject type 5 not modelled; locale.library/ParseDate P~ |
| AWeb | custom (ReAction layout.gadget) | 1,959 insns | none | - | - |
| AWebCfg | GadTools+BOOPSI | 908 insns | intuition.library/EasyRequestArgs | - | - |
| SnoopDos | GadTools+custom | 11,778 insns | none | - | - |
| aros-Clock | MUI | 3,055 insns | none | - | muimaster.library/MUI_NewObjectA MUI Window class: attribute 0x80428d2f is not implemente~ |
| aros-MultiView | GadTools+BOOPSI | 4,413 insns | intuition.library/WindowAction | - | - |
| aros-More | GadTools+BOOPSI | 3,226 insns | none | - | - |
| aros-Installer | MUI | 9,048 insns | none | - | dos.library/Close Close 0x0: not an open file; dos.library/Open Open NIL: is not modelled~ |
| aros-Calculator | MUI | 50,000,027 insns (budget, still running) | none | - | dos.library/Open Open NIL: is not modelled yet; intuition.library/SetAttrsA SetAttrsA: 0x~ |
| aros-Editor | GadTools+BOOPSI | 13,865 insns | iffparse.library/AllocIFF | - | dos.library/Close Close 0x0: not an open file; dos.library/Open Open NIL: is not modelled~ |
| aros-KeyShow | MUI | 1,347 insns | muimaster.library/MUI_CreateCustomClass | - | intuition.library/SetAttrsA SetAttrsA: 0x0 is not a BOOPSI object; muimaster.library/MUI_~ |
| aros-HDToolBox | MUI | 2,546 insns | none | - | - |
| aros-ScreenGrabber | MUI | 8,539 insns | muimaster.library/MUI_RequestA | - | dos.library/Close Close 0x0: not an open file; dos.library/Open Open NIL: is not modelled~ |
| aros-SysExplorer | MUI | 3,339 insns | none | - | - |
| aros-WiMP | MUI | 6,844 insns | none | - | muimaster.library/MUI_DisposeObject DisposeObject: 0x0 is not a MUI object (or already di~ |
| aros-Exchange | MUI | 2,971 insns | commodities.library/BrokerCommand | - | muimaster.library/MUI_DisposeObject DisposeObject: 0x0 is not a MUI object (or already di~ |
| aros-ASCIITable | MUI | 1,326 insns | muimaster.library/MUI_CreateCustomClass | - | muimaster.library/MUI_DisposeObject DisposeObject: 0x0 is not a MUI object (or already di~ |
| aros-FKey | MUI | 10,591 insns | none | - | dos.library/Close Close 0x0: not an open file; dos.library/Open Open NIL: is not modelled~ |
| aros-QuickPart | MUI | 9,818 insns | none | - | dos.library/Close Close 0x0: not an open file; dos.library/Open Open NIL: is not modelled~ |
| aros-AHIRecord | GadTools+BOOPSI | 12,836 insns | intuition.library/EasyRequestArgs | - | dos.library/Close Close 0x0: not an open file; dos.library/Open Open NIL: is not modelled~ |
| aros-BoingIconBar | GadTools+BOOPSI | 7,561 insns | none | - | dos.library/DupLockFromFH DupLockFromFH on 0x6300: not an open file; dos.library/DupLockF~ |
| aros-Find | MUI | 1,326 insns | muimaster.library/MUI_CreateCustomClass | - | muimaster.library/MUI_DisposeObject DisposeObject: 0x0 is not a MUI object (or already di~ |
| aros-Format | MUI | 3,381 insns | none | - | - |
| aros-SysMon | MUI | 3,133 insns | muimaster.library/MUI_DeleteCustomClass | - | - |
| aros-VMM | MUI | 8,067 insns | exec.library/RawPutChar | - | dos.library/Close Close 0x0: not an open file; dos.library/Open Open NIL: is not modelled~ |
| aros-Snoopy | MUI | 13,735 insns | none | - | dos.library/Close Close 0x0: not an open file; dos.library/Open Open NIL: is not modelled~ |
| aros-About | MUI | 1,326 insns | muimaster.library/MUI_CreateCustomClass | - | - |
| aros-Workbook | GadTools+BOOPSI | 3,286 insns | none | - | - |
| aros-Wanderer | MUI | 3,379 insns | intuition.library/EasyRequestArgs | - | dos.library/DupLockFromFH DupLockFromFH on 0x6300: not an open file; dos.library/DupLockF~ |
| aros-Info | MUI | 2,588 insns | none | - | - |
| aros-DiskInfo | MUI | 1,721 insns | muimaster.library/MUI_CreateCustomClass | - | - |
| aros-WBRename | MUI | 2,282 insns | none | - | muimaster.library/MUI_DisposeObject DisposeObject: 0x0 is not a MUI object (or already di~ |
| aros-prefs-Appearance | MUI-prefs | 6,693 insns | muimaster.library/MUI_CreateCustomClass | - | dos.library/Close Close 0x0: not an open file; dos.library/Open Open NIL: is not modelled~ |
| aros-prefs-Font | MUI-prefs | 6,672 insns | muimaster.library/MUI_CreateCustomClass | - | dos.library/Close Close 0x0: not an open file; dos.library/Open Open NIL: is not modelled~ |
| aros-prefs-Palette | MUI-prefs | 6,714 insns | muimaster.library/MUI_CreateCustomClass | - | dos.library/Close Close 0x0: not an open file; dos.library/Open Open NIL: is not modelled~ |
| aros-prefs-Input | MUI-prefs | 6,715 insns | muimaster.library/MUI_CreateCustomClass | - | dos.library/MatchFirst MatchFirst: AnchorPath 0x1ffdbe is not longword aligned; dos.libra~ |
| aros-prefs-Time | MUI-prefs | 3,391 insns | none | - | - |
| aros-prefs-ScreenMode | MUI-prefs | 6,714 insns | muimaster.library/MUI_CreateCustomClass | - | dos.library/Close Close 0x0: not an open file; dos.library/Open Open NIL: is not modelled~ |
| aros-prefs-Zune | MUI-prefs | 2,530 insns | none | - | - |
| aros-prefs-Locale | MUI-prefs | 9,825 insns | none | - | dos.library/Close Close 0x0: not an open file; dos.library/Open Open NIL: is not modelled~ |
| aros-prefs-Editor | MUI-prefs | 8,307 insns | iffparse.library/AllocIFF | - | dos.library/Close Close 0x0: not an open file; dos.library/Open Open NIL: is not modelled~ |
| aros-prefs-Pointer | MUI-prefs | 6,756 insns | muimaster.library/MUI_CreateCustomClass | - | dos.library/Close Close 0x0: not an open file; dos.library/Open Open NIL: is not modelled~ |
| aros-prefs-IControl | MUI-prefs | 6,693 insns | muimaster.library/MUI_CreateCustomClass | - | dos.library/Close Close 0x0: not an open file; dos.library/Open Open NIL: is not modelled~ |
| aros-prefs-Wanderer | MUI-prefs | 1,195 insns | muimaster.library/MUI_CreateCustomClass | - | muimaster.library/MUI_NewObjectA MUI Application class: attribute 0x8042a2c8 is not imple~ |
| aros-prefs-ReqTools | MUI-prefs | 6,651 insns | muimaster.library/MUI_CreateCustomClass | - | dos.library/Close Close 0x0: not an open file; dos.library/Open Open NIL: is not modelled~ |
| aros-prefs-OpenURL | MUI-prefs | 10,638 insns | muimaster.library/MUI_RequestA | - | dos.library/Close Close 0x0: not an open file; dos.library/Open Open NIL: is not modelled~ |
| aros-gtdemo | GadTools | 7,169 insns | exec.library/RawPutChar | - | dos.library/Close Close 0x0: not an open file; dos.library/Open Open NIL: is not modelled~ |
| aros-gtmultiselect | GadTools | 38,776 insns | none | - | dos.library/Close Close 0x0: not an open file; dos.library/Open Open NIL: is not modelled~ |
| aros-menudemo | GadTools | 7,099 insns (budget, still running) | none | OpenWindowx1, SetMenuStripx1 | dos.library/Open Open NIL: is not modelled yet |
| aros-mui_notify1 | MUI | 10,018 insns | none | - | dos.library/Close Close 0x0: not an open file; dos.library/Open Open NIL: is not modelled~ |
| aros-mui_layout1 | MUI | 9,051 insns | none | - | dos.library/Close Close 0x0: not an open file; dos.library/FPuts Write on 0x0: not an ope~ |
| aros-mui_pages | MUI | 9,159 insns | none | - | dos.library/Close Close 0x0: not an open file; dos.library/FPuts Write on 0x0: not an ope~ |
| aros-mui_slider | MUI | 8,850 insns | none | - | dos.library/Close Close 0x0: not an open file; dos.library/FPuts Write on 0x0: not an ope~ |
| aros-NList-Demo | MUI | 7,016 insns | muimaster.library/MUI_CreateCustomClass | - | dos.library/Close Close 0x0: not an open file; dos.library/Open Open NIL: is not modelled~ |
| aros-NListtree-Demo | MUI | 10,100 insns | none | - | dos.library/Close Close 0x0: not an open file; dos.library/Open Open NIL: is not modelled~ |
| aros-RequestString | MUI | 950 insns | none | - | muimaster.library/MUI_NewObjectA MUI Application class: attribute 0x80425ee5 is not imple~ |
| aros-MountHDF | MUI | 3,185 insns | none | - | - |

### Toolkit usage matrix, Amiga (name evidence from the hunk + what the census run created)

| app | GadTools | GadTools kinds (run time + static heuristic) | MUI classes named | BOOPSI system classes named | custom classes named | ASL | other system libs | disk/3rd-party libs |
|---|---|---|---|---|---|---|---|---|
| amiagent |  | - | - | - | - |  | cybergraphics | bsdsocket |
| amimon | GT | TEXT | - | - | - |  | - | - |
| amimon-mui |  | - | 7 | - | - |  | - | - |
| iGame |  | - | 18 | - | - |  | workbench | guigfx, lowlevel, render |
| DirectoryOpus | GT | BUTTON,INTEGER,MX,PALETTE | - | buttongclass, propgclass, strgclass, sysiclass | Nudopusgaugeclass, Nudopusiclass, Nudopusstrgclass | asl | commodities, cybergraphics, datatypes, workbench | Picasso96API, SysInfo, amigaguide, cgxsystem, cyberintuition |
| dopus-viewfont |  | - | - | - | - | asl | workbench | dopus5 |
| DOpusRT5 |  | - | - | - | - |  | - | dopus5 |
| YAM |  | - | 52 | - | subclass |  | cybergraphics, datatypes, iffparse, workbench | Nqrexxsyslib, amissl, amisslmaster, application, bamissl |
| AWeb | GT | BUTTON,GENERIC,INTEGER,LISTVIEW | - | buttongclass, gadgetclass, imageclass, pointerclass, propgclass, sysiclass | - | asl | commodities, datatypes, iffparse, workbench | AwebAmiSSL, AwebAmiTcp, Picasso96API, amissl, amisslmaster |
| AWebCfg | GT | BUTTON,SLIDER | - | sysiclass | - | asl | workbench | - |
| SnoopDos | GT | BUTTON,CYCLE,LISTVIEW,MX,SLIDER | - | buttongclass, propgclass, sysiclass | - | asl | commodities, workbench | amigaguide, ramlib-patch |
| aros-Clock |  | - | 3 | - | - |  | - | - |
| aros-MultiView | GT | - | - | buttongclass, icclass, modelclass, propgclass, sysiclass | datatypesclass | asl | datatypes, workbench | - |
| aros-More | GT | - | - | buttongclass, propgclass, sysiclass | - | asl | workbench | - |
| aros-Installer |  | - | 13 | - | - |  | datatypes, workbench | - |
| aros-Calculator |  | - | 4 | rootclass | - |  | - | - |
| aros-Editor | GT | - | - | propgclass, sysiclass | - | asl | iffparse, workbench | - |
| aros-KeyShow |  | - | 5 | - | - |  | commodities | - |
| aros-HDToolBox |  | - | 14 | frameiclass, gadgetclass | - |  | - | partition, uuid |
| aros-ScreenGrabber |  | - | 10 | - | - |  | cybergraphics, datatypes | - |
| aros-SysExplorer |  | - | 13 | - | hiddclass |  | - | aros, oop |
| aros-WiMP |  | - | 6 | - | - |  | - | - |
| aros-Exchange |  | - | 8 | - | - |  | commodities | - |
| aros-ASCIITable |  | - | 6 | - | - |  | commodities | - |
| aros-FKey |  | - | 10 | - | - |  | commodities | - |
| aros-QuickPart |  | - | 13 | - | - |  | - | expansion, oop, partition, uuid |
| aros-AHIRecord | GT | - | - | - | - | asl | cybergraphics | asyncio |
| aros-BoingIconBar | GT | - | - | - | - |  | datatypes, workbench | - |
| aros-Find |  | - | 10 | - | - |  | workbench | - |
| aros-Format |  | - | 9 | - | - |  | - | - |
| aros-SysMon |  | - | 16 | - | - |  | - | debug, oop |
| aros-VMM |  | - | 14 | - | - | asl | commodities | - |
| aros-Snoopy |  | - | 6 | - | - |  | - | - |
| aros-About |  | - | 9 | - | - |  | workbench | aros |
| aros-Workbook | GT | - | - | gadgetclass, groupgclass, propgclass, rootclass | - |  | workbench | - |
| aros-Wanderer |  | - | 15 | - | - |  | datatypes, iffparse, workbench | aros |
| aros-Info |  | - | 14 | - | - |  | workbench | - |
| aros-DiskInfo |  | - | 7 | - | - |  | - | - |
| aros-WBRename |  | - | 6 | - | - |  | workbench | - |
| aros-prefs-Appearance |  | - | 8 | - | - |  | - | decorator, decortheme |
| aros-prefs-Font |  | - | 6 | - | - |  | iffparse | - |
| aros-prefs-Palette |  | - | 10 | - | - |  | iffparse | - |
| aros-prefs-Input |  | - | 12 | - | - |  | commodities, iffparse | - |
| aros-prefs-Time |  | - | 8 | - | - |  | - | - |
| aros-prefs-ScreenMode |  | - | 11 | - | - |  | iffparse | - |
| aros-prefs-Zune |  | - | 27 | - | - | asl | - | muiscreen |
| aros-prefs-Locale |  | - | 9 | - | - |  | cybergraphics, iffparse | codesets |
| aros-prefs-Editor | GT | BUTTON,PALETTE | - | - | - | asl | iffparse | - |
| aros-prefs-Pointer |  | - | 10 | - | - |  | cybergraphics, datatypes, iffparse | - |
| aros-prefs-IControl |  | - | 8 | - | - |  | commodities, iffparse | - |
| aros-prefs-Wanderer |  | - | 11 | - | - |  | iffparse, workbench | - |
| aros-prefs-ReqTools |  | - | 7 | - | - |  | - | - |
| aros-prefs-OpenURL |  | - | 14 | - | - |  | - | openurl |
| aros-gtdemo | GT | BUTTON,CHECKBOX,INTEGER,LISTVIEW,MX,NUMBER,PALETTE,SCROLLER,SLIDER,STRING,TEXT | - | - | - |  | - | - |
| aros-gtmultiselect | GT | - | - | - | - |  | - | - |
| aros-menudemo | GT | - | - | - | - |  | - | - |
| aros-mui_notify1 |  | - | 1 | - | - |  | - | - |
| aros-mui_layout1 |  | - | 4 | - | - |  | - | - |
| aros-mui_pages |  | - | 4 | - | - |  | - | - |
| aros-mui_slider |  | - | 4 | - | - |  | - | - |
| aros-NList-Demo |  | - | 13 | - | - |  | - | - |
| aros-NListtree-Demo |  | - | 10 | - | - |  | - | - |
| aros-RequestString |  | - | 5 | - | - |  | - | - |
| aros-MountHDF |  | - | - | - | - |  | - | expansion |
### What the engine refuses most (count of apps; last run of each, first 6 refusals + the NEXT-NEEDED)

| apps | refusal |
|---|---|
| 30 | dos.library/Open: Open NIL: is not modelled yet |
| 26 | dos.library/Close: Close 0x0: not an open file |
| 16 | NEXT-NEEDED muimaster.library/MUI_CreateCustomClass |
| 10 | muimaster.library/MUI_NewObjectA: MUI Application class: attribute 0x8042a2c8 is not implemented |
| 8 | muimaster.library/MUI_NewObjectA: MUI class "SystemPrefsWindow.mcc" is not implemented |
| 6 | muimaster.library/MUI_MakeObjectA: MUI_MakeObjectA: object type 2 is not implemented (only MUIO_Label) |
| 6 | muimaster.library/MUI_DisposeObject: DisposeObject: <addr> is not a MUI object (or already disposed) |
| 4 | muimaster.library/MUI_MakeObjectA: MUI_MakeObjectA: object type 13 is not implemented (only MUIO_Label) |
| 3 | intuition.library/EasyRequestArgs |
| 3 | muimaster.library/MUI_NewObjectA: MUIA_Application_Window: the child object is NULL (its creation failed) |
| 3 | intuition.library/SetAttrsA: SetAttrsA: <addr> is not a BOOPSI object |
| 3 | muimaster.library/MUI_NewObjectA: MUIA_Window_RootObject: the child object is NULL (its creation failed) |
| 3 | muimaster.library/MUI_NewObjectA: MUI Group class: attribute MUIA_Frame is not implemented |
| 3 | dos.library/FPuts: Write on 0x0: not an open file |
| 2 | muimaster.library/MUI_NewObjectA: MUI Window class: attribute 0x804280cf is not implemented |
| 2 | iffparse.library/AllocIFF |
| 2 | muimaster.library/MUI_NewObjectA: MUI Application class: attribute 0x804235cb is not implemented |
| 2 | muimaster.library/MUI_MakeObjectA: MUI_MakeObjectA: object type 8 is not implemented (only MUIO_Label) |
| 2 | muimaster.library/MUI_RequestA |
| 2 | muimaster.library/MUI_MakeObjectA: MUI_MakeObjectA: object type 3 is not implemented (only MUIO_Label) |
| 2 | muimaster.library/MUI_NewObjectA: MUI Group class: attribute 0x80421a5f is not implemented |
| 2 | muimaster.library/MUI_MakeObjectA: MUI_MakeObjectA: object type 4 is not implemented (only MUIO_Label) |
| 2 | muimaster.library/MUI_NewObjectA: MUI Application class: attribute 0x8042c8d0 is not implemented |
| 2 | dos.library/DupLockFromFH: DupLockFromFH on 0x6340: not an open file |
| 2 | dos.library/DupLockFromFH: DupLockFromFH on 0x6300: not an open file |
| 2 | exec.library/RawPutChar |

### Aggregates over 64 Amiga binaries

Libraries named (count of apps): dos 64, intuition 63, exec 53, stdc 52, locale 47, muimaster 46, utility 38, icon 37, graphics 32, stdcio 31, posixc 31, workbench 19, gadtools 15, iffparse 13, asl 12, diskfont 10, commodities 10, layers 10, datatypes 9, keymap 8, cybergraphics 7, rexxsyslib 6, mathieeedoubbas 3, bsdsocket 3

MUI classes named (count of apps): Group.mui 44, Application.mui 44, Window.mui 34, Text.mui 34, Rectangle.mui 31, String.mui 25, List.mui 19, Listview.mui 18, Popasl.mui 12, Register.mui 11, SystemPrefsWindow.mcc 11, PrefsEditor.mcc 11, Gauge.mui 10, Slider.mui 10, Menuitem.mui 9, Menustrip.mui 8, Balance.mui 8, Menu.mui 7, Cycle.mui 7, Image.mui 7, Scrollgroup.mui 7, Virtgroup.mui 7, NListview.mcc 5, Popobject.mui 5, Scrollbar.mui 5, Area.mui 4, NFloattext.mcc 4, NList.mcc 4, Radio.mui 3, TextEditor.mcc 3

BOOPSI system classes named: sysiclass 7, propgclass 7, buttongclass 5, gadgetclass 3, rootclass 2, strgclass 1, pointerclass 1, imageclass 1, icclass 1, modelclass 1, frameiclass 1, groupgclass 1

GadTools kinds (apps using each; run time + static heuristic): BUTTON 6, INTEGER 3, MX 3, PALETTE 3, LISTVIEW 3, SLIDER 3, TEXT 2, GENERIC 1, CYCLE 1, NUMBER 1, STRING 1, SCROLLER 1, CHECKBOX 1

Custom BOOPSI classes made at run time (MakeClass name : super): dopusbuttongclass : gadgetclass, dopuscheckgclass : gadgetclass, dopusframeclass : gadgetclass, dopusgaugeclass : gadgetclass, dopusiclass : imageclass, dopuslistviewgclass : gadgetclass, dopuspalettegclass : gadgetclass, dopusstrgclass : strgclass, dopusviewgclass : gadgetclass, <private> : rootclass

Library functions called at run time by the binaries that reached them, by app count: intuition/NewObjectA 15, intuition/LockPubScreen 8, gadtools/GetVisualInfoA 6, muimaster/MUI_MakeObjectA 6, intuition/GetScreenDrawInfo 6, intuition/UnlockPubScreen 6, graphics/GetVPModeID 5, intuition/FreeScreenDrawInfo 5, MUI Text.mui 4, muimaster/MUI_NewObjectA 4, gadtools/FreeVisualInfo 4, asl/FreeAslRequest 4, graphics/TextLength 3, gadtools/CreateContext 3, intuition/OpenWindowTagList 3, intuition/MakeClass 3, asl/AllocAslRequest 3, gadtools/CreateMenusA 3, gadtools/LayoutMenusA 3, graphics/ReleasePen 3, intuition/DisposeObject 3, intuition/LockIBase 3, intuition/UnlockIBase 3, gadtools/CreateGadgetA 2

### Translation-table rows ranked by app count (Amiga, name evidence + census)

| # | apps | table row | role it feeds | e.g. |
|---|---|---|---|---|
| 1 | 44 | MUI Application + Window (shell, title, close, menus) | window | amimon-mui, iGame, YAM, aros-Clock ... |
| 2 | 43 | MUI Group (horizontal/vertical/virtual, frame, title) | group | amimon-mui, iGame, YAM, aros-Installer ... |
| 3 | 33 | MUI Text (labels, text buttons via MUIO_Label/Button) | label/button | amimon-mui, iGame, YAM, aros-Installer ... |
| 4 | 32 | MUI Balance / Rectangle spacing | (layout only) | iGame, YAM, aros-KeyShow, aros-HDToolBox ... |
| 5 | 26 | MUI String (text field) | field | iGame, YAM, aros-Installer, aros-HDToolBox ... |
| 6 | 23 | MUI List + Listview (rows, selection) incl. NList/NListtree | list | iGame, YAM, aros-Installer, aros-HDToolBox ... |
| 7 | 22 | ASL requesters (file/font/screenmode) | modal dialog | iGame, DirectoryOpus, dopus-viewfont, YAM ... |
| 8 | 17 | MUI Slider / Gauge / Scale | slider/gauge | YAM, aros-Installer, aros-ScreenGrabber, aros-QuickPart ... |
| 9 | 15 | GadTools gadget kinds (CreateGadgetA) | button/checkbox/choice/field/slider/list | amimon, DirectoryOpus, AWeb, AWebCfg ... |
| 10 | 11 | MUI Register (tabs) | choice (tabs) | YAM, aros-SysExplorer, aros-SysMon, aros-VMM ... |
| 11 | 11 | MUI Image / Dtpic / IconImage | image | iGame, YAM, aros-Installer, aros-QuickPart ... |
| 12 | 11 | MUI prefs window family (PrefsEditor/SystemPrefsWindow MCCs) | window + groups | aros-prefs-Appearance, aros-prefs-Font, aros-prefs-Palette, aros-prefs-Input ... |
| 13 | 10 | Commodities (Exchange/hotkeys; no UI) | (none) | DirectoryOpus, AWeb, SnoopDos, aros-KeyShow ... |
| 14 | 9 | BOOPSI system gadget/image classes (propg, strg, buttong, sysi, frame) | scrollbar/field/button/image | DirectoryOpus, AWeb, AWebCfg, SnoopDos ... |
| 15 | 9 | MUI Scrollgroup / Virtgroup / Scrollbar | scrollbar | YAM, aros-Installer, aros-SysExplorer, aros-QuickPart ... |
| 16 | 9 | datatypes (picture/text/sound viewers) | image/plane | DirectoryOpus, YAM, AWeb, aros-MultiView ... |
| 17 | 8 | MUI Menustrip / Menu / Menuitem | menu | amimon-mui, iGame, YAM, aros-HDToolBox ... |
| 18 | 8 | MUI Cycle / Radio (choice) | choice | iGame, YAM, aros-Installer, aros-HDToolBox ... |
| 19 | 5 | EasyRequest (system alert/confirm) | modal dialog | AWebCfg, aros-KeyShow, aros-AHIRecord, aros-Find ... |
| 20 | 3 | MUI TextEditor / TextEditor.mcp | multiline field | iGame, YAM, aros-Info |
| 21 | 3 | Custom BOOPSI classes (MakeClass; DOpus-style) | plane unless ancestor used | DirectoryOpus, dopus-viewfont, aros-Calculator |

### Mac census, one row per binary

| app | arch | ran | NEXT-NEEDED | Toolbox managers called |
|---|---|---|---|---|
| executor-go | 68k | 25474 instructions, halted=true, blocke~ | _GetCCursor ($AA1B) at pc=0x204f06 | toolbox: InitDialogs,InitFonts,InitGraf,InitWindows,REFUSED,TEInit; menu: InitMenus; event: FlushEvents |
| executor-icontest | 68k | 507 instructions, halted=true, blocked=~ | _NewCWindow ($AA45) at pc=0x20077a | toolbox: InitCursor,InitDialogs,InitFonts,InitGraf,InitWindows,REFUSED; menu: InitMenus; event: FlushEvents; ~ |
| retro68-Dialog-68k | 68k | 4083 instructions, halted=true, blocked~ | _GetNewDialog ($A97C) at pc=0x200a10 | toolbox: InitDialogs,InitFonts,InitGraf,InitWindows,REFUSED,TEInit; menu: InitMenus |
| retro68-FixedBenchmark-68k | 68k | 143738 instructions, halted=true, block~ | _SetToolBoxTrapAddress ($A647) at pc=0x20e43a | toolbox: GetTrapAddress,REFUSED |
| retro68-HelloWorld-68k | 68k | 118301 instructions, halted=true, block~ | _SetToolBoxTrapAddress ($A647) at pc=0x20afc0 | toolbox: GetTrapAddress,REFUSED |
| retro68-HelloWorldCpp-68k | 68k | 122180 instructions, halted=true, block~ | _SetToolBoxTrapAddress ($A647) at pc=0x22f210 | toolbox: GetTrapAddress,REFUSED |
| retro68-Launcher-68k | 68k | 124862 instructions, halted=true, block~ | _SetToolBoxTrapAddress ($A647) at pc=0x20b374 | toolbox: GetTrapAddress,REFUSED |
| retro68-Raytracer2-68k | 68k | 51554 instructions, halted=true, blocke~ | _SetToolBoxTrapAddress ($A647) at pc=0x202124 | toolbox: GetTrapAddress,REFUSED |
| retro68-Raytracer-68k | 68k | 9172 instructions, halted=true, blocked~ | _SetToolBoxTrapAddress ($A647) at pc=0x2014f4 | toolbox: GetTrapAddress,REFUSED |
| retro68-WDEFShell-68k | 68k | 7397 instructions, halted=true, blocked~ | _SetToolBoxTrapAddress ($A647) at pc=0x20144c | toolbox: GetTrapAddress,REFUSED |
| retro68-Application-ppc | ppc |  |  Library/beep (tvector): library Library is not provided by the engine | Other: GetVRefNum[NEXT-NEEDED],beep[NEXT-NEEDED],is_computer_on[NEXT-NEEDED]; Package/Misc: ExitToShell; File~ |
| retro68-Dialog-ppc | ppc |  | unimplemented trap import GetNewDialog: _GetNewDialog ($A97C) has no handler yet | QuickDraw: FrameRoundRect,InitCursor,InitGraf,InsetRect,PenSize; TextEdit: TEInit; Dialog: GetDialogItem[NEXT~ |
| retro68-FixedBenchmark-ppc | ppc |  |  InterfaceLib/GetFPos (tvector): _GetFPos ($A018) has no handler yet | File/Device: FSClose[NEXT-NEEDED],FSRead[NEXT-NEEDED],FSWrite[NEXT-NEEDED],FlushVol[NEXT-NEEDED],GetFPos[NEXT~ |
| retro68-HelloWorld-ppc | ppc |  | unimplemented trap import NGetTrapAddress: NGetTrapAddress is not a Toolbox or OS trap name in the engine's tables | File/Device: FSClose[NEXT-NEEDED],FSRead[NEXT-NEEDED],FSWrite[NEXT-NEEDED],FlushVol[NEXT-NEEDED],GetFPos[NEXT~ |
| retro68-HelloWorldCpp-ppc | ppc |  | unimplemented trap import NGetTrapAddress: NGetTrapAddress is not a Toolbox or OS trap name in the engine's tables | File/Device: FSClose[NEXT-NEEDED],FSRead[NEXT-NEEDED],FSWrite[NEXT-NEEDED],FlushVol[NEXT-NEEDED],GetFPos[NEXT~ |
| retro68-Raytracer2-ppc | ppc |  | unimplemented trap import Button: _Button ($A974) has no handler yet | QuickDraw: CopyBits,EraseRect,FrameRect,InitGraf,MoveTo,PaintRect; Font/Text: DrawString,InitFonts,StringWidt~ |
| retro68-Raytracer-ppc | ppc |  | unimplemented trap import Button: _Button ($A974) has no handler yet | Event: Button[NEXT-NEEDED],FlushEvents[NEXT-NEEDED],TickCount; Package/Misc: ExitToShell; File/Device: FSClos~ |
| retro68-WDEFShell-ppc | ppc |  | unimplemented trap import NewRoutineDescriptor: NewRoutineDescriptor is not a Toolbox or OS trap name in the engine's tables | QuickDraw: DiffRgn,DisposeRgn,FillRect,FillRgn,FillRoundRect,FrameOval; Window: BeginUpdate,DisposeWindow,Dra~ |
| systemless-showcase-68k | 68k | 46345 instructions, halted=true, blocke~ | _HFSDispatch ($A260) at pc=0x2077c8 | toolbox: GetOSTrapAddress,GetTrapAddress,REFUSED |

### Mac matrix: resource types (68k: static) and distinct Toolbox routines per manager

68k rows count A-line traps found in CODE resources (an upper bound: data that looks like a trap word counts, and the Retro68 runtime's own
libc file I/O inflates every Retro68 sample); PPC rows count the PEF's imports from InterfaceLib (exact). `Init*` and `TEInit` do not count.

| app | arch | WIND | ALRT | DLOG | DITL | CNTL | MENU | MBAR | CDEF | WDEF | LDEF | STR# | PICT | ICN# | TEXT | Window | Control | Dialog | Menu | List | TE | StdFile | QD | ColorQD | Font/Text | Event | Res | File | Mem | Misc |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| executor-go | 68k | 1 | 7 | 10 | 17 | 10 | 6 | 1 | 3 |  |  |  | 1 | 16 |  | 21 | 18 | 14 | 13 |  | 14 | 1 | 38 | 2 | 12 | 8 | 11 | 24 | 21 | 12 |
| executor-icontest | 68k |  |  |  |  |  |  |  |  |  |  |  |  | 11 |  | 2 |  |  |  |  |  |  | 3 | 1 |  | 1 | 3 |  | 4 | 1 |
| retro68-Dialog-68k | 68k |  |  | 1 | 1 |  |  |  |  |  |  |  |  |  |  |  | 2 | 5 |  |  |  |  | 3 |  |  | 1 | 2 |  | 6 | 4 |
| retro68-FixedBenchmark-68k | 68k |  |  |  |  |  |  |  |  |  |  |  |  |  |  | 11 | 7 | 3 | 4 |  | 3 |  | 24 | 15 | 9 | 5 | 10 | 29 | 16 | 24 |
| retro68-HelloWorld-68k | 68k |  |  |  |  |  |  |  |  |  |  |  |  |  |  | 11 | 3 |  |  |  | 2 |  | 23 | 12 | 8 | 5 | 8 | 16 | 15 | 23 |
| retro68-HelloWorldCpp-68k | 68k |  |  |  |  |  |  |  |  |  |  |  |  |  |  | 9 | 4 |  | 3 |  | 1 |  | 27 | 17 | 10 | 5 | 8 | 14 | 16 | 31 |
| retro68-Launcher-68k | 68k |  |  |  |  |  |  |  |  |  |  |  |  |  |  | 11 | 3 | 2 | 2 |  | 1 |  | 23 | 16 | 12 | 8 | 10 | 17 | 16 | 23 |
| retro68-Raytracer2-68k | 68k |  |  |  |  |  |  |  |  |  |  |  |  |  |  | 1 |  |  |  |  | 1 |  | 8 |  | 3 | 3 | 5 | 7 | 12 | 9 |
| retro68-Raytracer-68k | 68k |  |  |  |  |  |  |  |  |  |  |  |  |  |  | 1 |  |  |  |  |  |  | 8 |  | 3 | 3 | 3 |  | 10 | 8 |
| retro68-WDEFShell-68k | 68k | 1 |  |  |  |  | 3 | 1 |  | 2 |  | 1 |  |  | 1 | 10 |  |  | 10 |  | 1 |  | 20 |  | 2 | 5 | 3 | 2 | 10 | 9 |
| retro68-Application-ppc | ppc |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  | 6 | 2 | 1 |
| retro68-Dialog-ppc | ppc |  |  | 1 | 1 |  |  |  |  |  |  |  |  |  |  |  | 2 | 5 |  |  |  |  | 3 |  |  | 1 |  | 6 | 2 | 1 |
| retro68-FixedBenchmark-ppc | ppc |  |  |  |  |  |  |  |  |  |  |  |  |  |  | 10 |  |  |  |  |  |  | 10 |  | 5 | 5 |  | 9 | 5 | 2 |
| retro68-HelloWorld-ppc | ppc |  |  |  |  |  |  |  |  |  |  |  |  |  |  | 10 |  |  |  |  |  |  | 10 |  | 5 | 5 |  | 6 | 5 | 2 |
| retro68-HelloWorldCpp-ppc | ppc |  |  |  |  |  |  |  |  |  |  |  |  |  |  | 10 |  |  |  |  |  |  | 10 |  | 5 | 5 |  | 6 | 5 | 2 |
| retro68-Raytracer2-ppc | ppc |  |  |  |  |  |  |  |  |  |  |  |  |  |  | 1 |  |  |  |  |  |  | 8 |  | 3 | 3 |  | 6 | 5 | 1 |
| retro68-Raytracer-ppc | ppc |  |  |  |  |  |  |  |  |  |  |  |  |  |  | 1 |  |  |  |  |  |  | 8 |  | 4 | 3 |  | 6 | 3 | 1 |
| retro68-WDEFShell-ppc | ppc | 1 |  |  |  |  | 3 | 1 |  | 2 |  | 1 |  |  | 1 | 12 |  |  | 10 |  | 1 |  | 20 |  | 3 | 5 | 2 | 8 | 5 | 1 |
| systemless-showcase-68k | 68k | 1 | 1 | 1 | 2 | 1 | 9 | 1 |  |  |  |  |  |  |  | 15 | 8 | 5 | 13 | 1 | 17 | 1 | 51 | 19 | 11 | 10 | 15 | 1 | 6 | 10 |

Managers used beyond their Init* call, by app count (of 19 Mac apps): Memory 19, Package/Misc 19, QuickDraw 18, Event 18, Window 16, File/Device 16, Font/Text 15, Resource 12, TextEdit 9, Control 8, Menu 7, Color QD 7, Dialog 6, StandardFile 2, List 1


## The Mac corpus

19 classic Mac applications: 2 from the Executor tree (ARDI's own `go` launcher and `icontest`, MIT), and the Retro68
Samples built for BOTH architectures with the toolchain built here (Dialog, HelloWorld, HelloWorldCpp, Raytracer, Raytracer2,
FixedBenchmark, WDEFShell for 68k and PPC; Launcher for 68k; SharedLibrary/Application for PPC). Retro68 builds were produced by
`/data/build/retro68/build-all.sh` and `build-host.sh` (log paths in the sources table); they need Boost, which was missing
on this machine and was built locally (`/data/build/retro68/boost`), and texinfo, which was stubbed (only docs).

The richest app is `systemless`'s **Toolbox Showcase** (a fat application, 5,000 lines of C exercising Window/Menu/List/Resource/
TextEdit/StandardFile/Control/Dialog/Sound/Palette; GPL-3, used as test data only). Its source cannot be rebuilt here (Retro68's
open *Multiversal* interfaces lack `Controls.h`, `Lists.h`, `Palettes.h`, `QDOffscreen.h`, as its README says, and Apple's Universal
Interfaces cannot be redistributed), but its `.sit` stores the forks uncompressed, so `tools/corpus-extract-showcase.py` cuts
the 68k resource fork (`showcase.rsrc`) and the PPC PEF (`showcase.pef`) out of it; the PPC half is driven by
`macppc-engine/tests/real_app.rs` already. No other freely redistributable classic Mac application with an explicit licence turned
up (Macintosh Garden has none stated, so it was not used). **For Peter, if wanted: Apple's Universal Interfaces 3.4 in
`/data/refs/retro/Retro68/InterfacesAndLibraries/` would let Retro68 build the Showcase and any other Toolbox source.**

### Results

* **68k (mac68k-run --census)**. Both Executor apps and the Dialog sample stop at a Toolbox routine nobody has written:
  `_GetCCursor` (go, after the init sequence and `LoadSeg 6`), `_NewCWindow` (icontest), `_GetNewDialog` (Dialog).
  **Every other Retro68 sample, and the Showcase, first stopped at the same place: the startup's "does this trap exist?" probe,
  `_GetToolBoxTrapAddress` ($A746) / `_GetOSTrapAddress` ($A346)** (9 of 11 68k apps). That is now implemented
  (`traptable.rs`: the answer is a 4-byte stub that runs the same bridged trap, the `_Unimplemented` stub for an absent one, as a
  real ROM does); past it the Retro68 samples stop at `_SetToolBoxTrapAddress` ($A647: the runtime patches a trap) and the
  Showcase runs 46k instructions to `_HFSDispatch` ($A260, the File Manager).
* **PPC (macppc-run --census --run)**: every sample is instantiated and its `InterfaceLib` imports are bound or named
  (static census: 11 to 21 unbound per sample). The run stops: `NGetTrapAddress` (HelloWorld, HelloWorldCpp; the PPC InterfaceLib
  spells the same probe by that name, which the engine's trap-name table does not know), `GetNewDialog` (Dialog, whose
  first-listed unbound import is `ModalDialog`), `Button` (Raytracer, Raytracer2), `NewRoutineDescriptor` (WDEFShell: the
  Mixed Mode UPP builder), and the shared-library sample's `Library/beep` (a CFM shared library, which `macppc` cannot load yet).
* **Managers by app count (of 19)**: Memory and Package/Misc 19, QuickDraw 18, Event 18, Window 16, File/Device 16, Font/Text 15,
  Resource 12, TextEdit 9, **Control 8, Menu 7, Color QD 7, Dialog 6, StandardFile 2, List 1** (the matrix above has the per-app
  counts). The two rich apps (`go`: 10 CNTL, 10 DLOG, 17 DITL, 7 ALRT, 6 MENU, 3 own CDEFs; the Showcase: every manager) use all of
  them, which is the distribution doc 17 expects: Window + QuickDraw + Event carry every app, Control/Dialog/Menu carry the apps
  with a UI. The Retro68 samples are small and inflate Memory/File counts with their libc glue: weigh the two rich apps more.

### The top rows for the Mac side (by app count and by what a user sees)

1. **Window Manager** (`NewWindow`/`NewCWindow`/`GetNewWindow`, `BeginUpdate`/`EndUpdate`, `FindWindow`, `DragWindow`,
   `GrowWindow`, `TrackGoAway`): 16 of 19; a window is the unit of the projection. `_NewCWindow` is the next gap for icontest.
2. **QuickDraw drawing** (Rect/Oval/RoundRect/Rgn/Poly/Text, `CopyBits`, patterns, pen): 18 of 19: content planes replayed
   through the kit (doc 17 piece 5). `_GetCCursor`/colour is the next gap for go (Color QD: 7 apps).
3. **Event Manager** (`GetNextEvent`/`WaitNextEvent`, `Button`, `StillDown`, `TickCount`): 18 of 19, plus input in/exact guest out.
4. **Control Manager + standard CDEFs** (`NewControl`/`GetNewControl`, `TrackControl`, `HiliteControl`, `Get/SetControlValue`;
   push button, checkbox, radio, scrollbar via `CNTL`): 8 apps and the whole `go`/Dialog UI. Custom CDEFs (3 in `go`) are
   planes unless they inherit the standard part.
5. **Dialog Manager** (`GetNewDialog`/`ModalDialog`/`DrawDialog`/alerts, `DITL` item types: button, checkbox, radio, statText,
   editText, icon, pict, userItem, control): 6 apps. `ModalDialog` is the PPC Dialog sample's next gap and `_GetNewDialog` the 68k one.
6. **Menu Manager** (`MBAR`/`MENU`, `MenuSelect`, `MenuKey`, `AppendResMenu`, `HiliteMenu`): 7 apps; maps to the global menu bar.
7. **TextEdit** (`TENew`, `TEKey`, `TEClick`, `TEUpdate`, `TECut/Copy/Paste`): 9 (mostly the runtime's console); a `field`/multiline
   role for editText items and a plane for free TE.
8. **Font/Text** (`TextFont/Size/Face`, `DrawString`, `StringWidth`, `GetFontInfo`): 15: the kit's font engine at kit sizes.
9. **Resource Manager** (`GetResource`, `Get1Resource`, `ReleaseResource`, `LoadSeg`, `OpenResFile`): 12: the substrate for
   every `CNTL`/`DLOG`/`DITL`/`MENU`/`WIND` projection.
10. **List Manager and StandardFile** (`LNew`, `LAddRow`, `LClick`, `Pack0`/`Pack3`): 1 and 2 apps here (the Showcase uses a List, `go` and the Showcase `Pack3`),
    and every real document app uses both: they stay on the list because the corpus is thin on them, not because they are rare.

## Macintosh Garden apps (real shipped classic Mac software, 2026-10-18)

Peter authorised downloading from macintoshgarden.org for LOCAL TESTING ONLY. They are abandonware: "owner-authorised local test use, abandonware, never redistributed". Nothing here is committed: binaries and extracted resources live in `/data/build/corpus-bin/mac/<app>/` (URL, version and sha256 of every download in `/data/build/corpus-bin/CORPUS.tsv`). Downloaded archives are untrusted data: they were only ever unpacked with tools (machfs, unar, 7z), never executed on the host.

How they were fetched and unpacked (no sudo): the site serves an incomplete TLS chain, so curl was given the Sectigo intermediate fetched from the certificate's own AIA URL (verification stayed ON, no `-k`). Downloads are the token-signed links the page itself offers. `.zip`/`.dsk`/`.img`/`.toast` HFS images: `tools/mac-extract-hfs.py` (machfs in a venv; handles DiskCopy 4.2 headers) writes MacBinary II files. `.sit`/`.sea.bin`: `unar` (Fedora rpm unpacked into /data/build/rpms/root, run with LD_LIBRARY_PATH) writes AppleDouble `.rsrc` files. Both are read directly by retro-resfork.

Still unextracted, for Peter: **WriteNow 4.0.x**: both `WriteNow40.sit` and `writenow_4_02.zip` hold DiskCopy images of an installer (creator SIAP) whose payload is a proprietary installer archive that unar/lsar cannot open, and the 4.0.2 "updater" is only a patch. Needs a classic-Mac installer runner or a StuffIt Installer-Maker unpacker (nothing on vesper). WriteNow 2.0 and 2.2 are real 68k applications and ARE in the corpus. MFS volumes (WriteNow 1.07, MacPaint 1.5) cannot be read by machfs. BBEdit Lite pages list no download. Packages that would help: none required for what is here (`unar`, `p7zip` and python `machfs` are in use); a `hfsutils` or `hfsprogs` install would make HFS checks independent of machfs.

Tools: `tools/corpus-apps-mac-garden.tsv` (the app list; feed it to `corpus-census-mac.py`), `tools/mac-extract-hfs.py`, and `corpus-traps-mac.py --rank` (below). `census_corpus.rs` in `mac68k-engine` and `macppc-engine` pin each stop and SKIP loudly when the binaries are absent.

### Census (mac68k-run / macppc-run --census)

| app | arch | NEXT-NEEDED | managers reached before the stop |
|---|---|---|---|
| garden-writenow-2.0 | 68k | none (no unimplemented trap reached) | toolbox: InitFonts,InitGraf,InitWindows; event: EventAvail; res: GetResource |
| garden-writenow-2.2 | 68k | none (no unimplemented trap reached) | toolbox: InitFonts,InitGraf,InitWindows; event: EventAvail; res: GetResource |
| garden-writenow-installer-4.0 | 68k | _UnloadScrap ($A9FA) at pc=0x2027da | res: GetResource,ReleaseResource; toolbox: REFUSED |
| garden-writenow-updater-4.02 | 68k | _HFSDispatch ($A260) at pc=0x2039c8 | toolbox: GetTrapAddress,InitCursor,InitDialogs,InitFonts,InitGraf,InitWindows; menu: InitMenus; res: CurResFile |
| garden-macwrite-4.6 | 68k | _Pack2 ($A9E9) at pc=0x20900c | event: FlushEvents; res: GetResource; toolbox: InitDialogs,InitFonts,InitGraf,InitWindows,REFUSED,TEInit; menu: InitMenus; quickdraw: OpenPort |
| garden-macwrite-4.5 | 68k | _SetTrapAddress ($A047) at pc=0x2022e2 | event: FlushEvents; res: GetResource; toolbox: GetTrapAddress,InitDialogs,InitFonts,InitGraf,InitWindows,OpenPort |
| garden-macpaint-2.1F | 68k | _UnloadScrap ($A9FA) at pc=0x208752 | toolbox: REFUSED |
| garden-resedit-2.1.3 | 68k | _Alert ($A985) at pc=0x201aee | toolbox: GetTrapAddress,InitCursor,InitDialogs,InitFonts,InitGraf,InitWindows |
| garden-simpletext-1.4 | 68k | _HFSDispatch ($A260) at pc=0x2075d2 | toolbox: GetTrapAddress,REFUSED; res: CurResFile |
| garden-simpletext-1.3 | 68k | _SetToolBoxTrapAddress ($A647) at pc=0x203d90 | res: Get1Resource; toolbox: GetTrapAddress,REFUSED |
| garden-glider-pro | 68k | _SetToolBoxTrapAddress ($A647) at pc=0x205894 | res: GetResource,ReleaseResource; toolbox: GetTrapAddress,REFUSED |
| garden-glider-edit | 68k | _SetToolBoxTrapAddress ($A647) at pc=0x20154c | res: GetResource,ReleaseResource; toolbox: GetTrapAddress,REFUSED |
| garden-moving-van | 68k | _SetToolBoxTrapAddress ($A647) at pc=0x2007e4 | res: GetResource,ReleaseResource; toolbox: GetTrapAddress,REFUSED |
| garden-glider-pro-ppc | ppc | unimplemented trap import FlushEvents: _FlushEvents ($A032) is an OS trap with no C prototype in OS_SIGS yet | 155 unbound imports; run stops: see NEXT-NEEDED |

Findings the census exposed, and what happened to them (macfm lane, 2026-10-18):

* **MacWrite 4.5 `_OpenPort $8020a9fa` — FIXED: 24-bit addressing.** The classic Mac of that era has 24 address lines; applications keep flags in the high byte of pointers and handles. The machine now boots in 24-bit mode (`addrmode.rs`; `GuestMem` drops the top byte of EVERY access, the 68k bus and the host handlers alike; `_StripAddress` strips; `HLock`/`HPurge`/`HSetRBit`/`HSetState` mirror the lock/purge/resource bits into the master pointer's high byte, `_HGetState` and `**h` see them; `_SwapMMUMode` and the `MMU32Bit` global ($CB2) switch modes; `mac68k-run --32bit` / `Mac::set_address_mode(Bit32)` select 32-bit for a 32-bit-clean app). MacWrite's GrafPort pointer is now a good address; the 32-bit control still reproduces the old refusal.
* **WriteNow 2.0 / 2.2 ~10M bus faults — CAUSE FOUND AND FIXED (the same 24-bit issue).** Found with `mac68k-run --32bit --insns N --pcs N --dump` (the recent-PCs ring): after `GetResource('CODE',2)` the app's segment-call glue builds a RETURN ADDRESS, `move.l (a0),-(sp); addq.l #4,(sp); move.b #$80,(sp)`, i.e. it sets bit 31 of the return address as a flag, and returns through it with `RTR`. The 24-bit bus ignores that byte; with a 32-bit bus the PC became `$80208a32` (RAM is 16 MiB) and every fetch faulted (`pc` ran `$802091..` for 5M instructions). It was NOT a segment-loader gap. With 24-bit addressing both run on (to `_Pack6`, the International Utilities package).
* **Glider PRO** is a FAT application: its PEF ('cfrg') binds InterfaceLib imports (see the PPC row below) while its 68k half now stops at `_Pack7` (it passed `_SetToolBoxTrapAddress`).
* `_SetToolBoxTrapAddress`/`_SetTrapAddress` — IMPLEMENTED: a patched trap enters the guest routine as a `JSR` from the trapping instruction (`guestcall.rs`), which chains to the original through the `GetTrapAddress` stub; Pascal traps reached through a stub keep their frame (the return address sits on top of it). `_HFSDispatch`/`_FSDispatch`/`_HighLevelFSDispatch` and the whole File Manager (`fm/`, over the shared `retro-volume` tree), `_UnloadScrap` and the Scrap Manager (`scrap.rs`), `_Alert`/`_StopAlert`/`_NoteAlert`/`_CautionAlert` (`translate/alerts.rs`), `_Pack2` (`pack2.rs`, named refusals), `_SysEnvirons`, `_Gestalt`, `_DragWindow`, `_GetCursor`, `_PortSize`, `CountTypes`/`Count1Types`/`GetIndType`, `_SetResLoad`, `OpenResFile`/`CreateResFile`/`CloseResFile` landed too.
* **Open finding (next lane): ResEdit 2.1.3 and SimpleText 1.3** now run past every trap but stop at a SEGMENT LOADER refusal: they walk their own CODE resources (`Get1IxResource`, `GetResAttrs`, `ReleaseResource`) and the engine disposes a segment that is still resident (their `UnloadSeg` is accepted but segments stay resident), so the next `LoadSeg` reads freed memory. A real `UnloadSeg` (restore the jump-table entries, unlock the handle) is the fix.

Census, mac68k-run --census, before (971d664) and after this lane:

| app | NEXT-NEEDED before | NEXT-NEEDED after |
|---|---|---|
| WriteNow 2.0 | none: 10M bus faults | `_Pack6` ($A9ED) |
| WriteNow 2.2 | none: 10M bus faults | `_Pack6` ($A9ED) |
| WriteNow installer 4.0 | `_UnloadScrap` | `_GetFNum` ($A900) |
| WriteNow updater 4.02 | `_HFSDispatch` | `_Pack7` ($A9EE) |
| MacWrite 4.6 | `_Pack2` | `_SetCursor` ($A851) |
| MacWrite 4.5 | `_SetTrapAddress` (after a refused OpenPort) | `_SetCursor` ($A851) |
| MacPaint 2.1F | `_UnloadScrap` | `_SetWRefCon` ($A918) (88k instructions in) |
| ResEdit 2.1.3 | `_Alert` | none missing; LoadSeg refusal (see above) |
| SimpleText 1.4 | `_HFSDispatch` | `_OSDispatch` ($A88F) |
| SimpleText 1.3 | `_SetToolBoxTrapAddress` | none missing; LoadSeg refusal (see above) |
| Glider PRO (68k) | `_SetToolBoxTrapAddress` | `_Pack7` ($A9EE) |
| Glider Edit | `_SetToolBoxTrapAddress` | `_ParamText` ($A98B) |
| Moving Van | `_SetToolBoxTrapAddress` | `_GetNewCWindow` ($AA46) |
| Retro68 HelloWorld | `_SetToolBoxTrapAddress` | event loop: **opens its "Retro68 Console" window** |
| Retro68 Raytracer | `_SetToolBoxTrapAddress` | `_Button` ($A974) |
| Toolbox Showcase (68k) | `_HFSDispatch` | `_GetNewCWindow` ($AA46) |

### macwin lane (2026-10-19): the first Garden application opens its window

**MacWrite 4.6 opens "Untitled" on the engine** (document window with go-away and grow box, seven native menus, ruler and scroll bar drawn into
the content plane) and parks in `GetNextEvent`; `census_corpus.rs::macwrite_4_6_opens_its_window_with_menus_and_parks_in_its_event_loop` pins it.
MacPaint 2.1F also reaches its event loop (its palettes are created hidden, so no native window yet). Implemented, each with a host test and a
negative control: a real `_UnloadSeg` (`segment.rs`), `_SetCursor`/`_HideCursor`/`_ShowCursor`/`_ObscureCursor` (`cursor.rs`), `_SetWTitle`/`_GetWTitle`/
`_SetWRefCon`/`_GetWRefCon`/`_MoveWindow`/`_SizeWindow`/`_NewCWindow`/`_GetNewCWindow` (`winmgr.rs`), `_Pack6` International Utilities (`intl.rs`),
`_Pack7` Binary-Decimal (`pack7.rs`), one Package Manager convention (`package.rs`, also `_InitAllPacks`/`_InitPack`), `_GetFNum`/`_GetFontName`/
`_SetFScaleDisable`/`_SetFractEnable` (`fontnames.rs`), `_ParamText`/`_IsDialogEvent`/`_DialogSelect` (`translate/dialogs.rs`), `_OSDispatch` Process
Manager + Temporary Memory (`procmgr.rs`), `_CmpString`/`_RelString` (`strcmp.rs`), `_ReadDateTime`/`_SecondsToDate`/`_DateToSeconds` and a running
`Time` (`clock.rs`), `_SetOrigin` (`qd/origin.rs`), `_ForeColor`/`_BackColor`/`_ColorBit` (`qd/colorpen.rs`), `_SpaceExtra`, pictures `_OpenPicture`/
`_ClosePicture`/`_KillPicture`/`_PicComment`/`_DrawPicture` (`qd/picture.rs`), `_PurgeSpace`, `_GetString`, and `AddResMenu('FONT')` listing the System families.

Findings, in the order they were found (each is fixed or named):

* **`_UnloadSeg` was NOT what stopped ResEdit 2.1.3 and SimpleText 1.3.** Their `CODE` resources are **compressed** (resource attribute bit 0, header
  `$A89F6572`; ResEdit v8 with its own `dcmp` 0, SimpleText v9 with its own `dcmp` 3): the Resource Manager here never expands them, so `LoadSeg` read the
  compressed header as a segment header (`25970 entries at offset 43167` is the ASCII `"er"` of the signature). The next lane's task is Resource Manager
  decompression: run the file's own `dcmp` through the guest-call path (v8 dcmp 0/1/2: `dcmp(src, dst, work, len)` Pascal; v9 dcmp 3 has init / decompress /
  exit entry offsets in its first three words) or port a decompressor from ResourceDASM. Executor's `resOpen.cpp` only handles the v8 header.
* **DITL text items were shown without their first character** (every real dialog: "Sorry" -> "orry"). The item's length byte counts the text; there is no
  Pascal length inside. The hand-made fixtures wrapped their text in a Pascal string, so no test saw it.
* **`PBGetVInfo` overwrote 22 bytes past its 64-byte block** (the HFS-only `ioVSigWord`/`ioVFilCnt`/`ioVDirCnt` fields were written for every caller): MacWrite 4.5's
  frame held the block next to its saved A6 and return address and it returned through a cleared word. Now only `PBHGetVInfo` writes them.
* **Heap walkers**: MacWrite 4.6 counts free memory by walking `TheZone + 52` block by block, masking each size with the `Lo3Bytes` global ($31A) which was 0
  (an endless loop). `Lo3Bytes` is set, and the stub zone holds ONE free block spanning the heap (the engine's real allocations are host-side, not walkable).
* `GetItem` past the last item returns the empty string (the menu data's terminator), not a refusal: MacWrite counts a menu's items that way.
* **Open: MacWrite 4.5** returns through a 68000-style `TRAP #4` handler (`addq.w #2,sp; rts` assumes a 6-byte exception frame; the 68020 core pushes 8) and wanders
  into low memory. WriteNow 2.0 and Glider PRO (68k half) wander into address 0 the same way after a heap-block walk (they test block headers the engine does not have).
  Glider Edit and Moving Van need Color QuickDraw (`Gestalt 'qdrw'` undefined, then `_SetControlReference`): Glider Edit honestly shows its "needs Colour
  Quickdraw" alert; Moving Van runs to `_SetControlReference`.

Census, `mac68k-run --census --insns 50000000`, NEXT-NEEDED before this lane (the macfm table above) and after:

| app | NEXT-NEEDED before | NEXT-NEEDED after | window |
|---|---|---|---|
| MacWrite 4.6 | `_SetCursor` | **none: parked in `GetNextEvent`** | **opens "Untitled", 7 menus, ruler drawn** |
| MacWrite 4.5 | `_SetCursor` | none missing; wanders into address 0 (`TRAP #4` frame, see above) | none yet |
| MacPaint 2.1F | `_SetWRefCon` | **none: parked in `GetNextEvent`** | 5 windows created hidden |
| WriteNow 2.0 | `_Pack6` | none missing; wanders into address 0 (heap walk) | none yet |
| WriteNow 2.2 | `_Pack6` | `_VInstall` ($A033, vertical retrace) | none yet |
| WriteNow installer 4.0 | `_GetFNum` | `_GetAppParms` ($A9F5) | none yet |
| WriteNow updater 4.02 | `_Pack7` | none missing; wanders into address 0 | none yet |
| ResEdit 2.1.3 | LoadSeg refusal | LoadSeg refusal: compressed CODE (see above) | none yet |
| SimpleText 1.3 | LoadSeg refusal | LoadSeg refusal: compressed CODE | none yet |
| SimpleText 1.4 | `_OSDispatch` | `_Unimplemented` ($A89F) called from the stack area | none yet |
| Glider PRO (68k) | `_Pack7` | none missing; wanders into address 0 | none yet |
| Glider Edit | `_ParamText` | **none: runs; shows its "needs Colour Quickdraw" alert** | alert window (dialog) opens |
| Moving Van | `_GetNewCWindow` | `_SetControlReference` ($A95B) | its colour window opens |
| Toolbox Showcase (68k) | `_GetNewCWindow` | `_GetPalette` ($AA96) | |

### The next 10 traps by app count, whole Mac corpus (68k, static)

`tools/corpus-traps-mac.py --rank IMPLEMENTED.txt FILE...` counts, over every 68k application in the corpus (32 files: executor, the Retro68 samples, systemless-showcase and the Garden apps; PPC fragments have no CODE and are ranked by imports above), how many apps contain each trap in their CODE resources and the engine's table (`Mac::implemented()`) does not implement, compared by table KEY so aliases (`_GetOSTrapAddress`, `_NewPtrClear`) count as implemented. An upper bound: data that looks like a trap word counts; the Retro68 runtime's libc inflates the file calls.

| rank | apps | trap |
|---|---|---|
| 1 | 15 | _SizeWindow ($A91D) |
| 2 | 15 | _Pack7 ($A9EE) (NumToString / StringToNum) |
| 3 | 14 | _SetWTitle ($A91A) |
| 4 | 13 | _ScrollRect ($A8EF) |
| 5 | 13 | _GrowWindow ($A92B) |
| 6 | 13 | _MenuKey ($A93E) |
| 7 | 13 | _ParamText ($A98B) |
| 8 | 13 | _SystemClick ($A9B3) |
| 9 | 13 | _DebugStr ($ABFF) |
| 10 | 12 | _DrawPicture ($A8F6) |

(Re-ranked after the macfm lane over the 24 68k applications of `corpus-apps-mac*.tsv`, with `mac68k-run --implemented` as the table; the File Manager traps, `_DragWindow`, `_SysEnvirons` and `_SetTrapAddress` that headed the previous ranking are all implemented now, so the first ten above are what comes next. The previous ranking counted 32 files, so the counts are not comparable one to one.)



## MCC lane census (classes on disk, instance data, family dispose, self-drawing objects), 2026-10-19
`tools/corpus-mui.py` over the 47 MUI programs, before (merge base d10413b) and after. Columns: windows / native controls / planes / named refusals.

| program | before | after | next missing function | first named refusal left |
|---|---|---|---|---|
| aros-Clock | 0 / 0 / 0 / 3 | 1 / 0 / 1 / 1 | (see the mui3 table below) | unimplemented |
| aros-Find | 0 / 0 / 0 / 6 | 0 / 0 / 0 / 5 | intuition.library/EasyRequestArgs | AllocDosObject type 5 not modelled |
| aros-WBRename | 0 / 0 / 0 / 1 | 0 / 0 / 0 / 0 | none |  |
| aros-prefs-Appearance | 0 / 0 / 0 / 1 | 1 / 31 / 1 / 0 | none |  |
| aros-prefs-Font | 0 / 0 / 0 / 1 | 1 / 14 / 0 / 0 | none |  |
| aros-prefs-Palette | 0 / 0 / 0 / 1 | 0 / 0 / 0 / 3 | none | MUI Group class: attribute MUIA_Palette_Entries is not implemented |
| aros-prefs-Input | 0 / 0 / 0 / 1 | 0 / 0 / 0 / 2 | none | MatchFirst: AnchorPath 0x1ffdbe is not longword aligned |
| aros-prefs-ScreenMode | 0 / 0 / 0 / 1 | 0 / 0 / 0 / 1 | graphics.library/NextDisplayInfo | unimplemented |
| aros-prefs-Locale | 0 / 0 / 0 / 1 | 0 / 0 / 0 / 0 | none |  |
| aros-prefs-Pointer | 0 / 0 / 0 / 1 | 1 / 12 / 1 / 0 | none |  |
| aros-prefs-IControl | 0 / 0 / 0 / 1 | 0 / 0 / 0 / 4 | none | MUI class "ChunkyImage.mui" is not implemented |
| aros-prefs-Wanderer | 0 / 0 / 0 / 1 | 1 / 12 / 0 / 3 | none | MUI_CreateCustomClass: the superclass "Imagedisplay.mui" is not implem |
| aros-prefs-ReqTools | 0 / 0 / 0 / 1 | 1 / 32 / 0 / 1 | none | MUI Group class: attribute 0x90470501 is not implemented |
| aros-prefs-OpenURL | 0 / 0 / 0 / 2 | 0 / 0 / 0 / 1 | iffparse.library/AllocIFF | unimplemented |
| aros-NList-Demo | 0 / 0 / 0 / 7 | 0 / 0 / 0 / 5 | none | AllocDosObject type 5 not modelled |
| aros-NListtree-Demo | 0 / 0 / 0 / 5 | 0 / 0 / 0 / 4 | none | AllocDosObject type 5 not modelled |
| **all 47** | 9 / 115 / 0 / 47 | 15 / 216 / 3 / 43 | | |

Programs not listed did not change. The Prefs editors open because `PrefsEditor.mcc` / `PrefsWindow.mcc` / `SystemPrefsWindow.mcc` load from `SYS:Classes/Zune` (docs/MUI-NATIVE.md); the planes are the Appearance and Pointer previews and the Clock face (its hands need `graphics.library/InitArea`: AreaDraw polygons). The NList demos now load `NList.mcc` and stop on `dos/AllocDosObject` type 5 and `Bodychunk.mui`.
## Events, area fills, own-dispatcher sets, image classes (mui3 lane), 2026-10-19
`tools/corpus-mui.py` over the 47 MUI programs, before (push-mcc) and after. Columns: windows / native controls / planes / distinct named refusals.

| program | before | after | next missing function | first named refusal left |
|---|---|---|---|---|
| YAM | 0 / 0 / 0 / 1 | 0 / 0 / 0 / 1 | intuition.library/EasyRequestArgs | unimplemented |
| aros-Clock | 1 / 0 / 1 / 1 | 1 / 0 / 13 / 0 | none |  |
| aros-FKey | 0 / 0 / 0 / 4 | 0 / 0 / 0 / 0 | none |  |
| aros-Find | 0 / 0 / 0 / 5 | 1 / 16 / 0 / 2 | none | MUI_CreateCustomClass: the superclass "Semaphore.mui" is not implemented |
| aros-prefs-Palette | 0 / 0 / 0 / 3 | 0 / 0 / 0 / 1 | utility.library/CloneTagItems | unimplemented |
| aros-prefs-Input | 0 / 0 / 0 / 2 | 0 / 0 / 0 / 3 | muimaster.library/MUI_ObtainPen | BitMap 0x0: BytesPerRow 41 Rows 8212 Depth 0 is not a usable bitmap (chunky 1..=8 bits) |
| aros-prefs-ScreenMode | 0 / 0 / 0 / 1 | 0 / 0 / 0 / 3 | graphics.library/BestModeIDA | GetDisplayInfoData: record kind 0x80003000 is not modelled (only DTAG_DISP) |
| aros-prefs-IControl | 0 / 0 / 0 / 4 | 0 / 0 / 0 / 3 | muimaster.library/MUI_ObtainPen | BitMap 0x0: BytesPerRow 40 Rows 62660 Depth 0 is not a usable bitmap (chunky 1..=8 bits) |
| aros-prefs-Wanderer | 1 / 12 / 0 / 3 | 1 / 12 / 0 / 1 | none | MUI Application class: method 0x00000109 is not implemented |
| aros-prefs-ReqTools | 1 / 32 / 0 / 1 | 1 / 32 / 0 / 0 | none |  |
| aros-NList-Demo | 0 / 0 / 0 / 5 | 2 / 23 / 0 / 1 | none | MUI Application class: MUIM_Application_Load is not implemented |
| aros-NListtree-Demo | 0 / 0 / 0 / 4 | 1 / 30 / 0 / 0 | none |  |
| **all 47** | 15 / 216 / 3 / 43 | 19 / 285 / 15 / 24 | | |

Clock now draws its face and hands (AreaEnd fills into an off-screen bitmap, blitted) and ticks on the virtual clock (MUIIHNF_TIMER, 1000 ms); the Pointer preview's MUIM_Window_AddEventHandler node receives native pointer events. Programs not listed did not change.

## Pens, display modes, images, settings (mui4 lane), 2026-10-19
`tools/corpus-mui.py` over the 47 MUI programs, before (push-mui3) and after. Columns: windows / native controls / planes / distinct named refusals. The census volume now
assigns `LIBS:` the way AROS's Startup-Sequence does (`SYS:Libs`, then `SYS:Classes`, where `colorwheel.gadget` lives): without it the Palette editor's Coloradjust
class jumped through a NULL colorwheel base.

| program | before | after | next missing function | first named refusal left |
|---|---|---|---|---|
| aros-Find | 1 / 16 / 0 / 2 | 1 / 16 / 0 / 0 | none |  |
| aros-prefs-Palette | 0 / 0 / 0 / 1 | 1 / 7 / 0 / 0 | none |  |
| aros-prefs-Input | 0 / 0 / 0 / 3 | 1 / 22 / 0 / 0 | none |  |
| aros-prefs-ScreenMode | 0 / 0 / 0 / 3 | 1 / 48 / 0 / 0 | none |  |
| aros-prefs-IControl | 0 / 0 / 0 / 3 | 1 / 15 / 0 / 0 | none |  |
| aros-prefs-Wanderer | 1 / 12 / 0 / 1 | 1 / 12 / 0 / 0 | none |  |
| aros-NList-Demo | 2 / 23 / 0 / 1 | 2 / 23 / 2 / 0 | none |  |
| **all 47** | 19 / 285 / 15 / 24 | 23 / 377 / 17 / 10 | | |

Programs not listed did not change. What each step was: the Input and IControl editors' BetterString objects ask `MUI_ObtainPen` in Setup (and read
`_screen(obj)` before Area's Setup, so `mad_RenderInfo` had to be set first); ScreenMode asks `BestModeIDA` and `DTAG_DIMS/NAME`, and its own `nnset` calls
recursed until `MUIA_NoNotify` silenced a whole call; Palette needed the `CloneTagItems` family, the Slider's `MUIA_Group_Horiz` tag and `colorwheel.gadget` on
`LIBS:`; Wanderer prefs adds a window to its live application (`OM_ADDMEMBER`); Find's `Process.mui` is a class over `Semaphore.mui`; the NList demo
loads its settings (`MUIM_Application_Load`) and shows two Bodychunk pictures (the two planes). IControl's ChunkyImages sit on a register page that is not
the open one, so its planes appear when the page is shown. What still stops programs is not MUI (the refusals left are `EasyRequestArgs`, `AskKeyMapDefault`,
`BrokerCommand`, `RawPutChar`, `ArosInquireA`, `AllocIFF`, and two `DupLockFromFH` in Wanderer).

### macnext lane (2026-10-19): census walls taken one by one

`MAC_INSNS=50000000`, mac-plus profile; "after" runs with `MAC_IDLE=300` (300 virtual 60 Hz ticks passed while the app waits for an event;
without ticks the stops below the line are the same except that MacPaint / Moving Van stay blocked in their first event call). "Window" =
a native window opened through GuestUi.

| app | stop before | stop after | window |
|---|---|---|---|
| ResEdit 2.1.3 | `_OpenRFPerm` | event loop, modal alert (ModalDialog filterProc named) | yes (alert; its prefs file is empty, see below) |
| SimpleText 1.3 | `_AliasDispatch` (FindFolder) | `oapp` handler runs, Untitled window created, then `_TEStyleNew` | no (TextEdit) |
| SimpleText 1.4 | `_Unimplemented` (false "serious error": stale ResErr) | `oapp` handler runs, then `_TEStyleNew` | no (TextEdit) |
| Toolbox Showcase (68k, PPC) | `_GetPalette` | `_TENew` (PPC: `SetPalette` import) | window created |
| WriteNow 2.2 | `_VInstall` | `_SetDialogItem` (past `_FontMetrics`) | yes |
| WriteNow 2.0 | `_AVLTreeDispatch` (stray jump: Pack7 stack bug) | `_SetDialogItem` (past `_FontMetrics`) | yes |
| WriteNow installer 4.0 | `_GetAppParms` | event loop | yes |
| Moving Van | `_SetControlReference` | event loop (PICT v2 and `SetCRefCon(NIL)` list scroll bars refused by name) | yes |
| MacPaint 2.1F | event loop, windows hidden (no null event) | personalization dialog (its first-run) | yes (dialog) |
| Glider PRO / Glider Edit | `_AVLTreeDispatch` / alert | "Need Color QuickDraw" alert: honest absence kept | yes |
| MacWrite 4.5 / 4.6 | event loop | event loop (4.5 under mac-ii still takes the 68000 TRAP #4 frame) | yes |

Findings. (1) File Manager refnum 2 was the application's own resource-fork refnum: the first `Close` of a program's file unloaded its
code (ResEdit LoadSeg 22, Glider PRO). (2) `_Pack7` is a register package. (3) A real Mac returns null events forever; here the host clock
drives them. (4) The stale `ResErr` after a successful `GetNewMBar` made SimpleText 1.4 raise its own "serious error". (5) ResEdit opens its
own file by `CurApName` (now on the volume) and then declares its (empty, just created) preferences file damaged: AddResource /
WriteResource are not modelled yet, so it does not write the defaults a first run would. (6) MacWrite 4.5 under mac-plus: its ~132
non-fatal refusals are `GetDialogItem(NIL, ...)` after an activate event for the "Clipboard" window (segment 5): the program holds a NIL
DialogPtr; the cause (a GetNewDialog/NewDialog it should have made earlier, `NewDialog` is not implemented and is not reached by the
census) was NOT found in the time.

## MUI_RequestA, the Application's commodity, the Editor's exit (mui5 lane), 2026-10-19
`tools/corpus-mui.py` (47 MUI programs) and `tools/corpus-census.py` (all 64), before (push-amilibs 1e283ee) and after. Columns: windows / native controls / planes / distinct named refusals.

| program | before | after | what changed |
|---|---|---|---|
| aros-VMM | 0 / 0 / 0 / 2, next `MUI_RequestA` | 1 / 21 / 0 / 6, none | its requester class is on the seam; it opens its window (the 6 refusals are new MUI gaps it now reaches: `MUIM_Window_SetCycleChain`, `MUIM_List_Clear` on its Listview, `MUIA_List_Active`, `MUIM_SetAsString`, `MUIM_Application_Input`) |
| aros-FKey | 0 / 0 / 0 / 0 | 1 / 9 / 0 / 0 | its Application now has a broker (`MUIA_Application_Broker` / `BrokerPort`), so it gets past "Can't create CxBroker object!" and opens |
| aros-Exchange | 0 / 0 / 0 / 0, "Can't create CxBroker object!" | runs its main loop (hidden: `CX_POPUP` defaults to NO), none | `Broker` + `BrokerPort` of its Application, then `CopyBrokerList` / `FreeBrokerList` |
| aros-prefs-Editor | 0 / 0 / 0 / 1 | 1 / 0 / 0 / 2, next `gadtools.library/DrawBevelBoxA` | opens its window (named refusals: GadTools `PALETTE_KIND`, `DrawBevelBoxA`) |
| aros-Editor (GadTools) | ends with "Main interface failed to open!" | opens its window with menu strip, prop and arrows; next `intuition.library/NewModifyProp` | **cause: `IntuitionBase->ActiveScreen` was NULL** |
| aros-WiMP | 1 / 15 / 0 / 0 | 1 / 15 / 0 / 0 | unchanged, but only after a fix: once `FirstScreen` named the screen, WiMP listed it and its List construct hook got `A2 = 0` instead of the pool (`MUIA_List_Pool` absent: Zune makes one) |
| **all 47** | 24 windows / 491 controls / 17 planes / 7 | 27 / 521 / 17 / 12 | |

The all-64 census counts programs whose run reaches no unimplemented function: 61 before, 60 after (VMM joins; the Editor pair moves on to its next real gap, so the number goes down while the programs go further).

The Editor's exit was not silent: the line is `Main interface failed to open!` (the census tool prints the log only from the first unimplemented call, and none was reached).
Jano's `setup()` takes its screen from `prefs.parent`, which `set_default_prefs` copies from `IntuitionBase->ActiveScreen`; the engine's `IntuitionBase` had no ActiveScreen/FirstScreen, so `setup()` returned 1 before any library call.

## 2026-10-19: Gute Form font un-clipped; Mac `_FontMetrics`

* `tools/gen-gute-form-font.py` mapped strip row `yy` from scratch row `BASE - baseline + yy`, one row too low, so every ascender and capital
  lost its top ink row ('T' and 'F' had no bar). It now maps from `BASE - asc + yy` (row `tf_Baseline` = the last ink row above the baseline) and asserts
  that no capital A-Z or plain ascender (b d f h k l t) loses an ink row against the rasteriser's own output. `retro-theme` tests decode the strip
  (ascenders span row 0 to the baseline row, capitals are 8-9 rows, T/F/E/Z have a bar). Only one pinned hash moved
  (`amiga68k-engine/tests/gfx_window.rs`, the "AB" frame; its glyph-ink assertions, computed from the font itself, held).
  Still clipped, by design (Y_SIZE stays 11 = 9 above the baseline + 2 below): the accented capitals 192..=221 lose 2-4 top rows (their accents), and `$ % / \ ¾ Ø å ì í î ù ú û` by one row, `ç ý` lose a row of descender.
* `_FontMetrics` ($A835): `FMetricRec` (ascent, descent, leading, widMax as Fixed from the same record `GetFontInfo` uses; `wTabHandle`) and the
  Font Manager's 1072-byte global width table (`qd/fontmetrics.rs`; `tabData` is the Fixed `CharWidth` of each character; `WidthPtr` $B10 and
  `WidthTabHandle` $B2A name it). The stand-in font means `aFID` = the requested id, scale factors 256, `usedFam` false, `tabFont`/`fHand` NIL.
  `CharWidth`/`StringWidth` ignore `spExtra` and soft-style extra (an existing stand-in limit), so `sExtra` is stored and added to the space entry as the real table
  does, but the two agree only while `spExtra` = 0.

Census (68k, mac-plus and mac-ii, `--idle 300`, identical on both machines):

| app | before | after |
|---|---|---|
| WriteNow 2.0 | `_FontMetrics` | `_SetDialogItem` ($A98E) at pc=0x21307c, 32354 instructions |
| WriteNow 2.2 | `_FontMetrics` | `_SetDialogItem` ($A98E) at pc=0x21569c, 3313444 instructions |

## NewModifyProp, DrawBevelBoxA, the Listview's List, SetAsString, Application_Input (mui6 lane), 2026-10-19
`tools/corpus-mui.py` (47 MUI programs) and `tools/corpus-census.py` (all 64), before (merged-macfm e1965da) and after. Columns: windows / native controls / planes / distinct named refusals.

| program | before | after | what changed |
|---|---|---|---|
| aros-VMM | 1 / 21 / 0 / 6 | 1 / 21 / 0 / 0, next none | the six refusals are gone: `MUIM_Window_SetCycleChain` (recorded), a Listview's `MUIM_List_Clear` / `InsertSingle` / `MUIA_List_Active` (passed to its List), `MUIM_SetAsString`, `MUIM_Application_Input` |
| aros-prefs-Editor | 1 / 0 / 0 / 2, next `gadtools.library/DrawBevelBoxA` | 1 / 0 / 1 / 1, next none | `DrawBevelBoxA` is a native `Bevel` op (one plane); the remaining refusal is GadTools `PALETTE_KIND` (no swatch role) |
| aros-Editor (GadTools) | next `intuition.library/NewModifyProp` | next none | `NewModifyProp` moves the native ScrollBar |
| **all 47** | 28 / 521 / 17 / 12 | 28 / 521 / 18 / 5 | |

All-64 census: 62 programs reach no unimplemented function (60 after mui5); the two others (`dopus-viewfont` `QueryOverscan`, `aros-MultiView` `WindowAction`) are unchanged.
Findings. (1) Zune's window.c has no `MUIM_Window_SetCycleChain` (it falls to rootclass and answers 0); the engine records the program's order and the seam cannot carry it (no focus order in guestui-abi). (2) Listview.mui passes the List methods, the List attributes and the notifications on them to its List (listview.c:126-266), which is what VMM's refusals were. (3) `MUIM_Application_Input` has no `MUIA_` counterpart; it is `NewInput` with all signals taken as fired.

## Color QuickDraw on the Macintosh II profile (cqd lane), 2026-10-19
`tools/corpus-census-mac.py` over `corpus-apps-mac.tsv` and `corpus-apps-mac-garden.tsv` (`MAC_IDLE=300`, default profile mac-ii), runners built from the parent
commit (e1965da) and from this branch. What Glider PRO tests, found by tracing its startup (a disassembly of its CODE segment at the SysEnvirons call,
not a guess): `SysEnvirons(2, &rec)` and the byte at offset 9, `hasColorQD`. Not Gestalt `'qd  '`, not `ROM85`; it also reads `systemVersion` (>= $0605, >= $0700)
and probes `_Unimplemented` ($A89F) against `$A0AD`/`$A1AD`. Glider Edit asks Gestalt `'qdrw'` (`gestaltHasColor`), then `'fs  '`, then `'stdf'`.
The three answers (SysEnvirons, Gestalt `'qd  '`/`'qdrw'`, low-memory `ROM85`) now come from the machine profile; mac-plus and mac-se still say no.

| app | before | after | next wall |
|---|---|---|---|
| Glider PRO (68k) | alert "Need Color Quickdraw" (error 12), 146082 insns | past the colour test; asks `GetMainDevice`, `HasDepth` 1/4/8, `AESetInteractionAllowed`, plays its 12-frame animated cursor (`GetCCursor`/`SetCCursor`), Gestalt `'fold'` (honest "no"), then dies on its Preferences file: alert "A File Error Loading/Saving Preferences" (-120), 147100 insns | File Manager: `PBHOpenDF("Glider Prefs")` with vRefNum = the working directory its own `PBOpenWD` made and `ioDirID` 0x1f is `dirNFErr` here (not diagnosed further: not a Color QuickDraw matter) |
| Glider Edit | alert "Colour Quickdraw (system 7.0 or higher)" | passes `'qdrw'` and `'fs  '` (now bit 1, `gestaltHasFSSpecCalls`: the FSp calls exist), alert "the new filesystem" from `'stdf'` | Gestalt `'stdf'` (Standard File 3.0 attributes): Standard File (`_Pack3`) is not implemented, so there is no honest answer |
| Toolbox Showcase (68k) | `_TENew` | `_TENew` | unchanged: TextEdit |
| Toolbox Showcase (PowerPC) | `SetPalette` import (the known gap: the PowerPC name of `NSetPalette`, an alias now) | `TENew` | TextEdit |
| Executor `go` | `_GetCCursor` | `_SetDialogItem` | Dialog Manager |
| Executor `icontest` | `_CopyMask` | no unimplemented trap reached | (`CopyMask` is modelled) |
| Glider PRO (PowerPC) | `FlushEvents` import | `FlushEvents` import; 185 of 312 imports bound (was 167): the colour ones (`GetMainDevice`, `RGBForeColor`, `GetCCursor`, `OpenCPort`...), `SetPortBits`, `HasDepth`, `SetDepth`, `CopyMask` | the OS-trap C prototypes (`FlushEvents` ...) |

No other census row changed (the 33 programs of the two lists keep the same stop or final alert; SimpleText 1.4's `_TEStyleNew` is the same stop at a shifted address; run lengths differ by a few instructions).
Glider's own offscreen port is `OpenCPort` + `SetPortBits`/the PixMap fields + `CopyBits`/`CopyMask` (its PowerPC imports show `OpenCPort`, `CloseCPort`, no `NewGWorld`), which is
why the work is the colour PORT and the indexed pixel maps and not GWorlds (not implemented, not reached).

What was built (`crates/mac68k-engine/src/cqd/`, the drawing in `src/qd/`): the structures in guest memory at their documented offsets (`CGrafPort`, `PixMap`,
`ColorTable`, `GDevice`, `PixPat`, `CCrsr`), the standard 8-bit table (checked entry by entry against Pomme's `clut8`), the main device (8 bits deep, standard table), the Color
Manager (`Color2Index`, `Index2Color`, `RealColor`, `InvertColor`, `GetCTable`, `GetCTSeed`, `QDError`), `OpenCPort`/`InitCPort`/`CloseCPort`/`SetPortPix`, colour windows
(`NewCWindow`/`GetNewCWindow`: native frame, 8-bit canvas shown to the host as exact RGB pixels, never theme roles; a colour-table change repaints the window),
`RGBForeColor`/`RGBBackColor`/`GetForeColor`/`GetBackColor` and the classic `ForeColor`/`BackColor` on a colour port, `HasDepth`/`SetDepth`, `CopyBits` between depths and tables, `CopyMask`.
The ops are the SAME ones as before, made depth-aware: the pen is a pair of pen numbers, the Boolean transfer modes act on pixel values (`qd::draw::PenOp`), `retro-raster` gained
`stencil_with`. Refused by name: direct (16/32-bit) pixel maps, colour patterns (`patType` 1/2), arithmetic modes 32..39, `SetDepth` to another depth, `Index2Color` outside the table, a scaling `CopyMask`, `SetPortBits` on a CGrafPort, `NewCWindow` on a machine without Color QuickDraw.
Heap addresses of an app that calls `InitGraf` on mac-ii move by about 2.3 KB (the main device's records), which is why two tests that pinned a handle address now pin its form.

## One driver interface for the three engines (engine lane), 2026-10-19
`retro_common::GuestMachine` is the single surface a host app drives an Amiga, a Mac 68k or a Mac PowerPC program through
(`start_file(bytes, name)`, `deliver`, `step`, `advance_clock_to(virtual_us)`, `next_timer_deadline`, `ui`, `volume`, `drain_log`,
`drain_refused`, `set_stop_on_unimplemented`); the clock is virtual microseconds for all three, and the Mac engines convert to their 60 Hz
tick at one documented boundary (`ticks_at` / `us_at_tick` in `mac68k_engine::clock`). Tests: `crates/retro-common/tests/guest_machine.rs`
drives every engine only through the trait (fixtures, amimon, and the corpus apps below, which skip when `/data/build/corpus-bin` is absent).
Real applications reaching their window through the trait. Census: `macppc-run --census --run` output over the 10 PowerPC corpus apps (Retro68 x8, Glider PRO, Toolbox Showcase) is byte-identical before (58e4459) and after (the runners never advance the PowerPC clock); `mac68k-run` is untouched:

| platform | app | through the trait |
|---|---|---|
| Amiga | amimon (committed fixture) | GadTools window, parks on its timer, native close ends it |
| Mac 68k (mac-ii) | MacWrite 4.6 | "Untitled" document window + 7-menu strip, parks in GetNextEvent |
| Mac PowerPC | Retro68 HelloWorld | "Retro68 Console" window, then `NGetTrapAddress` by name (unchanged) |

One behaviour change underneath: a PowerPC `WaitNextEvent` with a sleep was parked until an event and never woke on time (the sleep was not
honoured); it now times out when the host's clock reaches the sleep's deadline, as the 68k engine's call does, so a PowerPC app's idle work runs.

## 2026-10-19: a host's MUI layout profile (group band, text measurer, label gap, spacing, window inset)

`mui::layout::Metrics` (ruling 17 piece 7, still a measured host profile, all-zero = Zune's own sizes) gained six fields: `text_w` (the host's
own width of a text, `None` = the screen font), `group_head` and `group_inset` (a titled group's contents start `head + inset` under its top edge
and `inset` inside its other three sides), `label_gap` (room right of a label that labels a control), `spacing` (the host's default neighbour gap,
a guest's own `MUIA_Group_*Spacing` still wins) and `win_pad`. Census: the AROS Snoopy window (corpus `aros-m68k/sys/System/Snoopy`) with the old
four-field kit profile versus the full profile, host-measured by the RetroCompat.app test; no corpus app's Zune-default layout changes (the
workspace corpus tests with a default profile are unchanged: 864 passed, 0 failed, debug and release). New corpus test:
`a_host_group_profile_keeps_every_group_member_inside_the_panel_and_labels_with_their_controls` (skips when the corpus is absent).
