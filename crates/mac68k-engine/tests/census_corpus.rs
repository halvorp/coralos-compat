//! The corpus lane's 68k census on REAL applications (not committed: /data/build/corpus-bin; docs/CORPUS.md). A missing binary
//! SKIPS loudly. Each test pins how far the app gets TODAY and moves on deliberately when a trap lands.
use guestui_abi::RecordingGuestUi;
use mac68k_engine::Mac;

fn census(rel: &str) -> Option<Mac<RecordingGuestUi>> { census_in(rel, mac68k_engine::addrmode::AddrMode::Bit24) }

/// [`census`] on the named machine profile.
fn census_on(rel: &str, machine: &str) -> Option<Mac<RecordingGuestUi>> {
    let path = format!("/data/build/corpus-bin/{rel}");
    let file = std::fs::read(&path).ok().or_else(|| { eprintln!("SKIPPED: corpus binary {path} not present (docs/CORPUS.md)"); None })?;
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    m.stop_on_unimplemented = true;
    m.set_machine(machine).unwrap();
    m.launch(&file).unwrap();
    m.run_loop(50_000_000).unwrap();
    Some(m)
}

fn census_in(rel: &str, mode: mac68k_engine::addrmode::AddrMode) -> Option<Mac<RecordingGuestUi>> {
    let path = format!("/data/build/corpus-bin/{rel}");
    let file = std::fs::read(&path).ok().or_else(|| { eprintln!("SKIPPED: corpus binary {path} not present (docs/CORPUS.md)"); None })?;
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    m.stop_on_unimplemented = true;
    m.set_address_mode(mode).unwrap();
    m.launch(&file).unwrap();
    m.run_loop(50_000_000).unwrap();
    Some(m)
}

#[test]
fn retro68_68k_samples_get_through_their_startup_and_the_trap_patch_and_stop_at_the_next_missing_trap() {
    // The Retro68 runtime asks GetToolTrapAddress/GetOSTrapAddress what exists (traptable.rs) and then PATCHES a Toolbox trap
    // (_SetToolBoxTrapAddress, now modelled: the patch routine runs and chains to the original). HelloWorld then runs to its
    // event loop and OPENS ITS WINDOW; Raytracer polls `Button` and renders until the budget is spent (Button/GetMouse are modelled
    // now: it used to stop at _Button); Dialog sets its user item's draw procedure with SetDialogItem, which `ModalDialog` now calls (the
    // Dialog Manager's user items, translate/useritems.rs), and waits in ModalDialog with nothing refused.
    for app in ["Raytracer", "Dialog"] {
        let Some(m) = census(&format!("mac-retro68/68k/{app}.bin")) else { return };
        assert!(m.refused.is_empty(), "{app}: {:?}", m.refused);
        if app != "Dialog" { assert!(m.log.iter().any(|l| l.contains("GetTrapAddress")), "{app}: the startup probe was answered: {:#?}", m.log); }
    }
    let Some(m) = census("mac-retro68/68k/HelloWorld.bin") else { return };
    assert!(m.refused.is_empty(), "HelloWorld reaches its event loop with no refusal: {:?}", m.refused);
    assert!(m.log.iter().any(|l| l.contains("SetTrapAddress Toolbox")), "the runtime's trap patch was installed: {:#?}", m.log);
    assert!(m.ui.calls.iter().any(|c| format!("{c:?}").contains("OpenWindow")), "HelloWorld opened its window");
    // NEGATIVE CONTROL: before traptable.rs the same samples stopped AT the probe, so no GetTrapAddress line existed.
    assert!(!m.refused.iter().any(|r| r.contains("GetToolBoxTrapAddress") || r.contains("GetOSTrapAddress")), "{:?}", m.refused);
}

#[test]
fn the_toolbox_showcase_fat_app_runs_past_the_file_manager_to_the_colour_window_manager() {
    // systemless's Toolbox Showcase (68k CODE in the resource fork, cut out by tools/corpus-extract-showcase.py): it used to stop at
    // the HFS dispatcher (_HFSDispatch $A260); the File Manager exists now and _GetNewCWindow is modelled now (winmgr.rs); the Palette Manager
    // answers (palette.rs); plain TextEdit exists now (textedit/), so it creates its text edit record (_TENew) and the next wall is the
    // STYLED one (_TEStyleNew).
    let Some(m) = census("systemless-showcase/showcase.rsrc") else { return };
    assert!(m.log.iter().any(|l| l.starts_with("[textedit] TENew")), "the showcase's TENew is answered: {:#?}", m.log);
    assert_eq!(m.refused, vec!["unimplemented trap _MeasureText ($A837)".to_string()], "styled TextEdit exists now (textedit/styled.rs), so the showcase creates its styled record and the next wall is the Font Manager's MeasureText: {:#?}", m.log);
    assert!(m.log.iter().any(|l| l.starts_with("[textedit] TEStyleNew")), "TEStyleNew is answered");
}

#[test]
fn macintosh_garden_apps_stop_at_the_first_missing_trap_and_each_stop_is_named() {
    // Real shipped classic Mac applications (Macintosh Garden, owner-authorised local test use; docs/CORPUS.md, tools/corpus-apps-mac-garden.tsv).
    // Each pins how far the app gets TODAY; when a trap lands the stop moves on and the row is updated deliberately.
    for (app, stop) in [
        ("mac/glider-pro/fx/Utilities_Moving Van.bin", "DrawPicture 0x"), // past SetControlReference: its PICT is a version-2 picture
        ("mac/simpletext/ux/SimpleText.rsrc", "unimplemented trap _Pack14 ($A830)"), // 1.4: past Pack7 (register package), ResErr after GetNewMBar, Pack8, the Finder's oapp event, and styled TextEdit (TEStyleNew, TEAutoView, TEDispatch): the next wall is Pack14
    ] {
        let Some(m) = census(app) else { continue };
        assert!(m.refused.last().is_some_and(|r| r.contains(stop)), "{app}: {:?}\n{:#?}", m.refused.last(), m.log);
    }
    // WriteNow 2.0 sets its ruler dialog's twelve user items (SetDialogItem), has DrawDialog call the draw procedure (which composes its picture
    // offscreen through SetPortBits on the dialog's own port, CopyBits it back, and returns through a flagged address), and runs on into its
    // event loop: no missing trap at all. 2.2 gets as far as 2.0 and then jumps into low memory (the next wall, not a Toolbox routine).
    if let Some(m) = census("mac/writenow/fx/z20_0/WriteNow 2.0.bin") {
        assert!(m.refused.iter().all(|r| !r.contains("unimplemented")), "{:?}", m.refused);
        for needle in ["[dialog] DrawDialog", "user item 1 of", "SetPortBits", "[quickdraw] CopyBits"] { assert!(m.log.iter().any(|l| l.contains(needle)), "{needle}: {:#?}", m.log.len()); }
        assert!(!m.refused.iter().any(|r| r.contains("SetPBits")), "SetPortBits on a window's port is honoured now: {:?}", m.refused);
    }
    if let Some(m) = census("mac/writenow/fx/z22_1/WriteNow 2.2.bin") {
        assert!(m.refused.iter().all(|r| !r.contains("unimplemented")), "no Toolbox routine is missing for 2.2 either: {:?}", m.refused);
        assert!(m.refused.iter().any(|r| r.starts_with("DrawString: port") && r.contains("bounds (0,0,0,")), "the next wall: the offscreen BitMap it measures font names into has no rows: {:?}", m.refused.last());
    }
    // The WriteNow 4.0 installer now gets through GetAppParms and DrawDialog to its event loop: no missing trap at all.
    if let Some(m) = census("mac/writenow/fx40_1/Install WriteNow 4.0.bin") {
        assert!(m.refused.iter().all(|r| !r.contains("unimplemented")), "{:?}", m.refused);
        assert!(m.log.iter().any(|l| l.contains("[segment] GetAppParms")), "{:#?}", m.log);
    }
    // "Walls" that are not a missing Toolbox routine: past everything modelled the program JUMPS TO ADDRESS 0 and the first A-line word it
    // meets there is the "trap" the census reports. Findings (docs/CORPUS.md): MacWrite 4.5 returns through a 68000-style TRAP #4
    // handler frame (`addq.w #2,sp; rts` after a TRAP: a 68020 frame is two bytes longer); WriteNow 2.0 and Glider PRO walk heap blocks.
    // What is pinned is the fact that matters, that the stop is in low memory and not at a Toolbox routine.
    for app in ["mac/macwrite/u45/MacWrite.rsrc"] {
        let Some(m) = census(app) else { continue };
        let (op, pc) = m.unimplemented_at.last().copied().expect("a stop");
        // Where the stray stop lands depends on what the garbage after the bad return happens to contain (address 0 or the middle of an
        // unloaded segment), so only its being a stop that no sane program would call is pinned: an A-line word from the AVL/Drag
        // dispatch range ($AA80, $AD05) at an odd or low-memory pc, never one of the traps this lane implemented.
        assert!(matches!(op, 0xAA80 | 0xAD05) && (pc < 0x1000 || pc % 2 == 1), "{app}: the stop at {op:#06x} (pc {pc:#x}) is a stray one, not a Toolbox call: {:?}", m.refused.last());
        if app.contains("Glider") { continue; } // fat app: its 68k half stops in its own code before any of those
        assert!(m.log.iter().any(|l| l.contains("[menu] DrawMenuBar") || l.contains("GetNewWindow") || l.contains("[pack6]") || l.contains("[font]") || l.contains("[clock]")), "{app}: it got past the trap walls this lane cleared");
    }
    // Glider PRO needs Color QuickDraw and SAYS SO in its own alert on a machine without it: the Plus profile gets the program's real,
    // documented answer (it tests SysEnvirons' hasColorQD, byte 9 of the record, nothing else). The default Macintosh II has Color
    // QuickDraw, so it goes on: past its colour/depth checks, through the animated cursor, to the Preferences folder, where it stops
    // at a File Manager answer (OpenDF of "Glider Prefs" under a working directory is dirNFErr), which is the next wall.
    let Some(m) = census_on("mac/glider-pro/fx/Glider PRO\u{2122}.bin", "mac-plus") else { return };
    assert!(m.log.iter().any(|l| l.contains("[dialog] ParamText [\"Need Color Quickdraw\"")), "{:#?}", m.log);
    let Some(m) = census("mac/glider-pro/fx/Glider PRO\u{2122}.bin") else { return };
    assert!(!m.log.iter().any(|l| l.contains("Need Color Quickdraw")), "the Macintosh II has Color QuickDraw: {:#?}", m.log);
    assert!(m.log.iter().any(|l| l.contains("[quickdraw] GetMainDevice")) && m.log.iter().any(|l| l.contains("[palette] HasDepth")), "it asked for the main device and its depths");
    assert!(m.log.iter().any(|l| l.contains("[toolbox] SetCCursor")), "its animated cursor ran");
    // The Preferences file: SysEnvirons' sysVRefNum is the System Folder's working directory, Glider finds the Preferences folder in it
    // (GetCatInfo), opens a working directory on it, and asks for "Glider Prefs" with PBOpenDF through FSDispatch ($A060: an IOParam block with
    // NO ioDirID: it used to read the stack garbage after the block as a dirID and get dirNFErr -120). A first run has no such file: fnfErr.
    assert!(m.log.iter().any(|l| l.contains("[file] open data fork \"Glider Prefs\"") && l.ends_with("-> -43")), "{:#?}", m.log);
    assert!(!m.log.iter().any(|l| l.contains("-> -120")), "no dirNFErr: {:#?}", m.log);
    // the Sound Manager answers (sound/): GetDefaultOutputVolume is its first call; the next wall is the Resource Manager's MaxSizeRsrc
    assert!(m.log.iter().any(|l| l.starts_with("[sound] GetDefaultOutputVolume")), "its volume question is answered: {:#?}", m.log);
    // MaxSizeRsrc answers, the System file's I-beam cursor exists (GetCursor 1: its absence was Glider's "Couldn't Load a Resource"), the
    // sounds load, the game gets as far as its title pictures (PICT v2: refused by name) and the palette (_SetEntries)
    assert!(m.log.iter().any(|l| l.starts_with("[res] MaxSizeRsrc")) && m.log.iter().any(|l| l.starts_with("[res] GetCursor 1 -> 0x") && !l.contains("-> 0x0 ")), "{:#?}", m.log);
    assert!(!m.log.iter().any(|l| l.contains("Couldn't Load a Resource")), "the load-failure alert is gone");
    assert!(m.refused.last().is_some_and(|r| r.contains("_SetEntries ($AA3F)")), "the next wall: {:?}", m.refused.last());
    // ResEdit 2.1.3 and SimpleText 1.3 have COMPRESSED CODE resources (resource attribute bit 0, header $A89F6572, a 'dcmp' in the file
    // itself). The Resource Manager runs that 'dcmp' IN THE GUEST (compressed.rs); before it did, a LoadSeg read the compressed header as a
    // segment header ("25970 entries at jump-table offset 43167") and the app died there.
    for (app, stop) in [("mac/resedit/u213/resedit_2.1.3.sea_/ResEdit.rsrc", ""), ("mac/simpletext/u13/SimpleText 1.3.rsrc", "unimplemented trap _Pack14 ($A830)")] {
        let Some(m) = census(app) else { continue };
        // ResEdit finds itself (SetVol with GetVol's bare volume name restores the default directory) and accepts its fresh preferences file (a
        // new resource fork is the 286 bytes CreateResFile writes), so it no longer declares itself or its preferences damaged: nothing refused.
        if stop.is_empty() { assert!(m.refused.is_empty(), "{app}: {:?}", m.refused); } else { assert!(m.refused.last().is_some_and(|r| r.contains(stop)), "{app}: {:?}", m.refused); }
        assert!(!m.refused.iter().any(|r| r.contains("LoadSeg") || r.contains("cannot decompress")), "{app}: {:?}", m.refused);
        let expanded = m.log.iter().filter(|l| l.contains("[res] decompress")).count();
        assert!(expanded >= 3, "{app}: {expanded} decompressions ran in the guest");
        let path = format!("/data/build/corpus-bin/{app}");
        let fork = retro_resfork::ResFork::parse(retro_resfork::open(&std::fs::read(&path).unwrap()).map(|o| std::fs::read(&path).unwrap()[o.rsrc].to_vec()).unwrap()).unwrap();
        let compressed = (0..fork.len()).filter(|&i| fork.entry(i).is_some_and(|e| e.ty == *b"CODE" && e.attrs & 1 != 0 && fork.data(i).is_some_and(|d| d.starts_with(&[0xA8, 0x9F, b'e', b'r'])))).count();
        assert!(compressed >= 8, "{app}: {compressed} compressed CODE resources");
    }
    // NEGATIVE CONTROL: an app whose CODE is not compressed gets past the same segment loader.
    if let Some(file) = std::fs::read("/data/build/corpus-bin/mac/macwrite/u45/MacWrite.rsrc").ok() {
        let fork = retro_resfork::ResFork::parse(file[retro_resfork::open(&file).unwrap().rsrc].to_vec()).unwrap();
        assert!((0..fork.len()).all(|i| fork.entry(i).is_none_or(|e| e.ty != *b"CODE" || e.attrs & 1 == 0)));
    }
    // MacWrite 4.5 no longer records the OpenPort refusal: its flagged GrafPort pointer (0x8020a9xx) is a good address on a 24-bit machine.
    let Some(m) = census("mac/macwrite/u45/MacWrite.rsrc") else { return };
    assert!(!m.refused.iter().any(|r| r.starts_with("OpenPort")), "{:?}", m.refused);
    assert!(m.log.iter().any(|l| l.strip_prefix("[quickdraw] OpenPort 0x").is_some_and(|h| h.len() == 6 && h.starts_with("20"))), "OpenPort was called with the stripped pointer (no flag byte): {:#?}", m.log);
    // MacWrite 4.5 under the Plus: ~130 `GetDialogItem(NIL, ...)` calls (and as many HiliteControl / GetDialogItemText on a garbage handle) after the
    // activate event for its Clipboard window. THE CAUSE: its event loop calls `IsDialogEvent` with `subq.w #2,sp` (the result slot is NOT cleared) and
    // tests the WORD it gets back with `tst.w (sp)+`; the engine wrote only the Boolean's high byte, so the stack garbage in the low byte made a FALSE
    // answer look TRUE and the program went on to its dialog handler with no dialog. A Boolean result now clears the slot's low byte (pascal.rs).
    if let Some(m) = census_on("mac/macwrite/u45/MacWrite.rsrc", "mac-plus") {
        assert!(m.log.iter().any(|l| l.contains("[dialog] IsDialogEvent what 8 -> false")), "the activate event for the Clipboard window is not a dialog event: {:#?}", m.log.len());
        assert!(!m.refused.iter().any(|r| r.contains("GetDialogItem") || r.contains("HiliteControl") || r.contains("GetDialogItemText")), "{:?}", &m.refused[..m.refused.len().min(5)]);
    }
    // NEGATIVE CONTROL: on a 32-bit machine the same pointer is outside RAM, as before the 24-bit work.
    let Some(m) = census_in("mac/macwrite/u45/MacWrite.rsrc", mac68k_engine::addrmode::AddrMode::Bit32) else { return };
    assert!(m.refused.iter().any(|r| r.starts_with("OpenPort: GrafPort: pointer")), "{:?}", m.refused);
}

#[test]
fn writenow_2_x_ran_into_a_bus_fault_loop_because_it_jumps_through_a_flagged_return_address() {
    // FINDING (docs/CORPUS.md): WriteNow 2.0 and 2.2 burned the whole 5M-instruction budget with ~10M bus faults after
    // GetResource('CODE', 2). THE CAUSE (found with the recent-PCs ring): its segment-call glue builds a RETURN ADDRESS and sets the high
    // byte to $80 (`move.l (a0),-(sp); addq.l #4,(sp); move.b #$80,(sp)`), then returns through it with `RTR`; the 24-bit bus of the machine
    // it was written for ignores that byte. On a 32-bit bus the PC became $80xxxxxx (RAM is 16 MiB) and every fetch faulted.
    for app in ["mac/writenow/fx/z20_0/WriteNow 2.0.bin", "mac/writenow/fx/z22_1/WriteNow 2.2.bin"] {
        let Some(m) = census(app) else { continue };
        assert!(m.log.iter().any(|l| l.contains("GetResource 'CODE' 2")), "{app}: {:#?}", m.log);
        // past the International Utilities package (modelled in intl.rs) with no Toolbox routine missing
        assert!(m.unimplemented_at.is_empty(), "{app}: {:?}", m.refused.last());
        assert!(m.log.iter().any(|l| l.contains("[pack6]") || l.contains("[font] SetFScaleDisable") || l.contains("[font]")), "{app}: it reached Pack6 / the Font Manager: {:#?}", m.log);
        // NEGATIVE CONTROL: the 32-bit machine reproduces the old symptom (no trap reached; the budget spent on bus faults).
        let Some(c) = census_in(app, mac68k_engine::addrmode::AddrMode::Bit32) else { continue };
        assert!(c.refused.is_empty() && c.unimplemented_at.is_empty(), "{app}: the 32-bit run never reaches a trap, it stays in the fault loop: {:?}", c.refused);
    }
}

#[test]
fn macwrite_4_6_opens_its_window_with_menus_and_parks_in_its_event_loop() {
    // THE GOAL of the macwin lane: a real Macintosh Garden application opens its window on the engine. MacWrite 4.6 launches, builds
    // its seven menus (the Font menu from the System families), opens "Untitled" (WIND 301, a document window with go-away and grow),
    // draws its ruler and scroll bar into the content plane, handles the first update event and blocks in GetNextEvent.
    use guestui_abi::{UiCall, WindowRole};
    let Some(file) = std::fs::read("/data/build/corpus-bin/mac/macwrite/fx46/MacWrite.bin").ok().or_else(|| { eprintln!("SKIPPED: MacWrite 4.6 not present"); None }) else { return };
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    m.stop_on_unimplemented = true;
    m.launch(&file).unwrap();
    let r = m.run_loop(5_000_000).unwrap();
    assert!(r.stats.blocked && !r.stats.halted, "parked in its event loop: {:?} {:#?}", r.stats, m.log);
    assert!(m.unimplemented_at.is_empty(), "no unimplemented trap was reached: {:?}", m.unimplemented_at);
    let opens: Vec<_> = m.ui.calls.iter().filter_map(|c| match c { UiCall::OpenWindow(w) => Some(w.clone()), _ => None }).collect();
    assert_eq!(opens.len(), 1, "{:?}", m.ui.calls);
    assert_eq!((opens[0].title.as_str(), opens[0].role, opens[0].closable, opens[0].resizable), ("Untitled", WindowRole::Document, true, true));
    let strip = m.ui.calls.iter().rev().find_map(|c| match c { UiCall::SetMenuStrip(_, s) => Some(s.clone()), _ => None }).unwrap();
    assert_eq!(strip.menus.iter().map(|x| x.title.as_str()).collect::<Vec<_>>(), ["Apple", "File", "Edit", "Search", "Format", "Font", "Style"]);
    let font = strip.menus.iter().find(|x| x.title == "Font").unwrap();
    assert!(font.items.iter().any(|i| i.label == "Geneva") && font.items.len() == 15, "the Font menu lists the System families: {:?}", font.items.len());
    assert!(m.ui.calls.iter().any(|c| matches!(c, UiCall::PresentPlane(_, p) if p.ops.len() > 3)), "its ruler and scroll bar were drawn into the content plane");
    // the refusals it leaves are NAMED and are exactly the known ones: PBLockRange (HFSDispatch selector $10) on the document file, and
    // the ruler dialog code asking a NIL dialog for its items
    assert!(m.refused.iter().all(|r| r.contains("HFSDispatch selector 0x10") || r.contains("is not a dialog") || r.contains("is not a control") || r.contains("is not a dialog item handle") || r.contains("0x3c0000") || r.contains("EmptyRgn")), "{:?}", m.refused);
    // NEGATIVE CONTROL: before this lane the same binary stopped at _SetCursor with no window at all.
    assert!(m.log.iter().any(|l| l.contains("[toolbox] SetCursor")), "SetCursor is modelled: {:#?}", &m.log[..m.log.len().min(5)]);
}

#[test]
fn macpaint_2_1f_reaches_its_event_loop_and_its_palettes_own_wdef_is_refused_by_name() {
    // MacPaint 2.1F builds a "Diagnostic" window (procID 16, the system rounded WDEF, visible = FALSE) and four Tool / Pattern /
    // Spot / FatBits palettes with procID 544 = WDEF 34, a 724-byte 68k window definition MacPaint ships in its own resource fork.
    // WDEF slice 1 (docs/consults/2026-10-20-wdef.md): a program's own WDEF has no native presentation contract, so each palette
    // is refused by name and NewWindow returns NIL; before the slice they were silently created as document windows. The app
    // still reaches GetNextEvent; the only other refusals are what a NIL window leads to (MoveWindow of it, text calls on a NIL port).
    let Some(mut m) = census("mac/macpaint/u21/MacPaint 2.1F.rsrc") else { return };
    assert!(m.unimplemented_at.is_empty(), "{:?}", m.refused);
    assert!(m.ev.waiting.is_some(), "blocked in GetNextEvent");
    assert_eq!(m.get_resource(*b"WDEF", 34, false).ok().and_then(|h| m.handle_bytes(h)).map(|b| b.len()), Some(724), "WDEF 34 is MacPaint's own");
    let wdef = m.refused.iter().filter(|r| r.contains("procID 544 selects WDEF 34") && r.contains("[WDEF-UNSUPPORTED]")).count();
    assert_eq!(wdef, 4, "the four palettes, each refused by name: {:?}", m.refused);
    assert!(m.refused.iter().all(|r| r.contains("[WDEF-UNSUPPORTED]") || r.contains("0x0 is not an open window") || r.contains("thePort is NIL")), "{:?}", m.refused);
    // the system rounded window is not refused, and nothing was silently opened in a palette's place
    assert_eq!(m.windows.len(), 1, "only the Diagnostic window exists");
    assert!(m.windows.values().all(|w| w.proc_id == 16 && w.native.is_none()), "Diagnostic, created hidden");
    assert!(m.log.iter().any(|l| l.contains("NewWindow \"Diagnostic\"") && l.contains("proc=16")));
}
