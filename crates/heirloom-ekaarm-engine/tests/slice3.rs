//! The CONE / Avkon / file server / store / active object natives of slice 3, on synthetic processes (no corpus): each native is
//! called by name through the same dispatch the guest's import stubs use, with its arguments in guest memory.
use heirloom_ekaarm_engine::cpu::Memory;
use heirloom_ekaarm_engine::layout::{CODE_BASE, EXIT_SVC, STUB_BASE, STUB_SVC_BASE};
use heirloom_ekaarm_engine::natives::{self, NativeOutcome};
use heirloom_ekaarm_engine::{loader, Outcome, Process};

const BX_LR: u32 = 0xE12F_FF1E;

fn words(w: &[u32]) -> Vec<u8> {
    w.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// A bare process whose import stubs are `names`; stub `i` is `names[i]`.
fn proc(names: &[&str]) -> Process {
    let mut all: Vec<&str> = names.to_vec();
    all.extend(["_ZN4User6AllocZEi", "_ZN4User4FreeEPv"]);
    let mut p = loader::bare(&all).unwrap();
    p.uid3 = 0x1028_28d6;
    p
}

/// Call the native named `name` with r0-r3 = `args`; return what it did and r0.
fn call(p: &mut Process, name: &str, args: &[u32]) -> (NativeOutcome, u32) {
    let idx = p.stubs.iter().position(|s| s.name.as_deref() == Some(name)).unwrap_or_else(|| panic!("{name} is not a stub of this process"));
    for (i, a) in args.iter().enumerate() {
        p.cpu.regs[i] = *a;
    }
    let o = natives::call(p, idx);
    (o, p.cpu.regs[0])
}

fn ok(p: &mut Process, name: &str, args: &[u32]) -> u32 {
    match call(p, name, args) {
        (NativeOutcome::Return, r0) => r0,
        (o, _) => panic!("{name}: {o:?}"),
    }
}

fn refused(p: &mut Process, name: &str, args: &[u32]) -> String {
    match call(p, name, args) {
        (NativeOutcome::Refused(m), _) => m,
        (o, _) => panic!("{name}: expected a refusal, got {o:?}"),
    }
}

fn alloc(p: &mut Process, n: u32) -> u32 {
    ok(p, "_ZN4User6AllocZEi", &[n])
}

fn rd(p: &mut Process, a: u32) -> u32 {
    p.mem.read_u32(a).unwrap()
}

fn wr(p: &mut Process, a: u32, v: u32) {
    use heirloom_ekaarm_engine::cpu::Memory;
    p.mem.write_u32(a, v).unwrap();
}

/// A `TPtrC16` (type 1) at a fresh heap block pointing at `text`'s UTF-16 copy.
fn tptrc16(p: &mut Process, text: &str) -> u32 {
    let units: Vec<u16> = text.encode_utf16().collect();
    let data = alloc(p, units.len() as u32 * 2 + 2);
    for (i, u) in units.iter().enumerate() {
        use heirloom_ekaarm_engine::cpu::Memory;
        p.mem.write_u16(data + i as u32 * 2, *u).unwrap();
    }
    let d = alloc(p, 8);
    wr(p, d, units.len() as u32 | 1 << 28);
    wr(p, d + 4, data);
    d
}

#[test]
fn engine_stubs_dispatch_by_name_and_unverified_vtable_slots_refuse_by_name() {
    let mut p = proc(&["_ZImported"]);
    let first = p.engine_stub("_ZEngineMade").unwrap();
    assert_eq!(p.engine_stub("_ZEngineMade").unwrap(), first, "one stub per name");
    assert_eq!(p.engine_stub("_ZImported").unwrap(), STUB_BASE, "an image's own import of that name is reused");
    let vptr = p.build_vtable("CThing", &[Some("_ZImported"), None, Some("_ZEngineMade")]).unwrap();
    assert_eq!(rd(&mut p, vptr), STUB_BASE);
    assert_eq!(rd(&mut p, vptr + 8), first);
    let unnamed = rd(&mut p, vptr + 4);
    let idx = ((unnamed - STUB_BASE) / 8) as usize;
    assert_eq!(p.stubs[idx].name.as_deref(), Some("CThing::vslot1"));
    // the stub is real guest code: SVC STUB_SVC_BASE+idx, then BX LR, and calling it reaches the named dispatch
    assert_eq!(rd(&mut p, unnamed), 0xEF00_0000 | (STUB_SVC_BASE + idx as u32));
    match natives::call(&mut p, idx) {
        NativeOutcome::Refused(m) => assert!(m.starts_with("NEXT-NEEDED") && m.contains("CThing::vslot1"), "{m}"),
        o => panic!("{o:?}"),
    }
}

#[test]
fn host_state_is_tied_to_the_allocation_generation_not_just_the_address() {
    let names = ["_ZN24CRemConInterfaceSelector4NewLEv", "_ZN24CRemConInterfaceSelector11OpenTargetLEv", "_ZN4User4FreeEPv", "_ZN4User6AllocZEi"];
    let mut p = proc(&names);
    let sel = ok(&mut p, names[0], &[]);
    ok(&mut p, names[1], &[sel]);
    // the guest frees the selector and gets the same address back for something else: the old host state must not be reachable
    ok(&mut p, names[2], &[sel]);
    let again = ok(&mut p, "_ZN4User6AllocZEi", &[16]);
    assert_eq!(again, sel, "the heap reuses the address");
    let m = refused(&mut p, names[1], &[again]);
    assert!(m.contains("not a live interface selector"), "{m}");
}

#[test]
fn remcon_target_is_registered_with_its_selector_and_needs_a_live_one() {
    let (sel_new, target_new, open) = ("_ZN24CRemConInterfaceSelector4NewLEv", "_ZN20CRemConCoreApiTarget4NewLER24CRemConInterfaceSelectorR28MRemConCoreApiTargetObserver", "_ZN24CRemConInterfaceSelector11OpenTargetLEv");
    let mut p = proc(&[sel_new, target_new, open]);
    let sel = ok(&mut p, sel_new, &[]);
    let t = ok(&mut p, target_new, &[sel, 0x1234]);
    assert_ne!(t, 0);
    ok(&mut p, open, &[sel]);
    assert!(refused(&mut p, target_new, &[0x0900_0000, 0]).contains("not a live interface selector"));
}

#[test]
fn cleanup_stack_pop_and_destroy_frees_tany_items_and_refuses_an_unbalanced_pop() {
    let (push_any, pop_destroy, pop_destroy_n, pop) = ("_ZN12CleanupStack5PushLEPv", "_ZN12CleanupStack13PopAndDestroyEv", "_ZN12CleanupStack13PopAndDestroyEi", "_ZN12CleanupStack3PopEv");
    let mut p = proc(&[push_any, pop_destroy, pop_destroy_n, pop, "_ZN4User6AllocZEi"]);
    let (a, b) = (alloc(&mut p, 16), alloc(&mut p, 16));
    ok(&mut p, push_any, &[a]);
    ok(&mut p, push_any, &[b]);
    ok(&mut p, pop_destroy, &[]);
    assert_eq!(p.heap.as_ref().unwrap().len_of(b), None, "the top item was freed");
    assert!(p.heap.as_ref().unwrap().len_of(a).is_some(), "the one below stays");
    assert!(refused(&mut p, pop_destroy_n, &[2]).contains("PopAndDestroy of 2 with 1"));
    ok(&mut p, pop_destroy_n, &[1]);
    assert_eq!(p.heap.as_ref().unwrap().len_of(a), None);
    assert!(refused(&mut p, pop, &[]).contains("cleanup stack"), "popping an empty stack is refused");
}

#[test]
fn trap_brackets_nest_and_an_unbalanced_one_is_refused() {
    let (mark, unmark, push_any) = ("_ZN4User16MarkCleanupStackEv", "_ZN4User18UnMarkCleanupStackEP12TTrapHandler", "_ZN12CleanupStack5PushLEPv");
    let mut p = proc(&[mark, unmark, push_any]);
    let h = ok(&mut p, mark, &[]);
    let h2 = ok(&mut p, mark, &[]);
    assert_eq!(h, h2, "one handler object per thread");
    ok(&mut p, unmark, &[h]);
    ok(&mut p, unmark, &[h]);
    assert!(refused(&mut p, unmark, &[h]).contains("no TRAP bracket is open"));
    ok(&mut p, mark, &[]);
    ok(&mut p, push_any, &[0x10]);
    assert!(refused(&mut p, unmark, &[h]).contains("holds 1 items at the end of a TRAP opened at 0"));
    assert!(refused(&mut p, unmark, &[0x7777]).contains("not the trap handler"));
}

#[test]
fn descriptor_copy_is_bounded_by_the_maximum_length_and_text_stays_utf16() {
    let copy = "_ZN6TDes164CopyERK7TDesC16";
    let mut p = proc(&[copy, "_ZN10TBufBase16C1Ei"]);
    let src = tptrc16(&mut p, "Příklad");
    let buf = alloc(&mut p, 8 + 20);
    ok(&mut p, "_ZN10TBufBase16C1Ei", &[buf, 10]); // a TBuf16<10>: iMaxLength 10, data at +8
    ok(&mut p, copy, &[buf, src]);
    assert_eq!(rd(&mut p, buf) & 0x0FFF_FFFF, 7, "iLength");
    use heirloom_ekaarm_engine::cpu::Memory;
    assert_eq!(p.mem.read_u16(buf + 8 + 2).unwrap(), 'ř' as u16, "elements are UTF-16 in guest memory");
    let long = tptrc16(&mut p, "01234567890");
    assert!(refused(&mut p, copy, &[buf, long]).contains("11 elements into a maximum length of 10"));
}

#[test]
fn private_path_is_created_in_the_in_memory_drive_only_through_a_connected_session() {
    let (connect, create, private) = ("_ZN3RFs7ConnectEi", "_ZN3RFs17CreatePrivatePathEi", "_ZN3RFs19SetSessionToPrivateEi");
    let mut p = proc(&[connect, create, private]);
    p.set_drive(heirloom_ekaarm_engine::drive::Drive::new());
    let fs = alloc(&mut p, 8);
    assert!(refused(&mut p, create, &[fs, 2]).contains("not connected"), "an RFs that was never connected");
    assert_eq!(ok(&mut p, connect, &[fs, 0xFFFF_FFFF]), 0);
    assert_eq!(ok(&mut p, create, &[fs, 2]), 0, "KErrNone: created");
    assert_eq!(ok(&mut p, create, &[fs, 2]) as i32, -11, "KErrAlreadyExists");
    assert!(p.host.drive.as_ref().unwrap().exists("\\private\\102828d6"));
    assert_eq!(ok(&mut p, private, &[fs, 2]), 0);
}

#[test]
fn dictionary_store_creates_an_empty_file_reports_nothing_present_and_refuses_an_existing_store() {
    let (connect, private, open, present) = ("_ZN3RFs7ConnectEi", "_ZN3RFs19SetSessionToPrivateEi", "_ZN20CDictionaryFileStore6OpenLCER3RFsRK7TDesC164TUid", "_ZNK16CDictionaryStore10IsPresentLE4TUid");
    let dtor = "_ZN20CDictionaryFileStoreD0Ev";
    let mut p = proc(&[connect, private, open, present, dtor, "_ZN12CleanupStack13PopAndDestroyEv"]);
    p.set_drive(heirloom_ekaarm_engine::drive::Drive::new());
    let fs = alloc(&mut p, 8);
    ok(&mut p, connect, &[fs, 0xFFFF_FFFF]);
    ok(&mut p, private, &[fs, 2]);
    let name = tptrc16(&mut p, "filebrowser.ini");
    let store = ok(&mut p, open, &[fs, name, 0x1028_28d6]);
    assert!(p.host.drive.as_ref().unwrap().exists("C:\\private\\102828d6\\filebrowser.ini"), "created relative to the session path");
    assert_eq!(ok(&mut p, present, &[store, 0x1234]), 0);
    // the engine-built object has a virtual destructor the cleanup stack can call (slot 1 = deleting)
    let vptr = rd(&mut p, store);
    let d0 = rd(&mut p, vptr + 4);
    assert_eq!(p.stubs[((d0 - STUB_BASE) / 8) as usize].name.as_deref(), Some(dtor));
    ok(&mut p, "_ZN12CleanupStack13PopAndDestroyEv", &[]);
    // a store file that already has content is a format this engine does not read: refused, never treated as empty
    p.host.drive.as_mut().unwrap().put_file("C:\\private\\102828d6\\old.ini", vec![1, 2, 3]).unwrap();
    let old = tptrc16(&mut p, "old.ini");
    assert!(refused(&mut p, open, &[fs, old, 0x1028_28d6]).contains("not implemented"));
    // and an RFs that is not connected is refused
    let bad = alloc(&mut p, 8);
    assert!(refused(&mut p, open, &[bad, name, 0]).contains("not connected"));
}

#[test]
fn active_objects_are_queued_by_priority_and_report_added_through_their_links() {
    let (ctor, add, deque, set_active) = ("_ZN7CActiveC2Ei", "_ZN16CActiveScheduler3AddEP7CActive", "_ZN7CActive5DequeEv", "_ZN7CActive9SetActiveEv");
    let mut p = proc(&[ctor, add, deque, set_active]);
    let [a, b, c] = [alloc(&mut p, 28), alloc(&mut p, 28), alloc(&mut p, 28)];
    for (o, prio) in [(a, 0u32), (b, 10), (c, 0)] {
        ok(&mut p, ctor, &[o, prio]);
        assert_eq!(rd(&mut p, o + 12), 0, "IsAdded() is false before Add");
        assert_eq!(rd(&mut p, o + 20), prio, "Priority()");
    }
    for o in [a, b, c] {
        ok(&mut p, add, &[o]);
        assert_ne!(rd(&mut p, o + 12), 0, "IsAdded()");
    }
    assert!(refused(&mut p, add, &[a]).contains("already added"));
    // order: b (10) first, then a, then c (equal priorities keep arrival order): walk the links
    let head = rd(&mut p, c + 12); // c is last, so its next is the sentinel
    let mut order = vec![];
    let mut at = rd(&mut p, head);
    while at != head { order.push(at - 12); at = rd(&mut p, at); }
    assert_eq!(order, vec![b, a, c]);
    ok(&mut p, deque, &[a]);
    assert_eq!(rd(&mut p, a + 12), 0, "Deque clears the links");
    let mut order = vec![];
    let mut at = rd(&mut p, head);
    while at != head { order.push(at - 12); at = rd(&mut p, at); }
    assert_eq!(order, vec![b, c]);
    ok(&mut p, set_active, &[b]);
    assert_eq!(rd(&mut p, b + 8) & 1, 1, "IsActive()");
}

#[test]
fn a_view_is_registered_by_the_uid_its_own_id_returns_and_activation_needs_a_known_view() {
    let (add, activate, default_view) = ("_ZN13CAknViewAppUi8AddViewLEP8CAknView", "_ZN13CAknViewAppUi17ActivateLocalViewLE4TUid", "_ZN9CCoeAppUi15SetDefaultViewLERK8MCoeView");
    let mut p = proc(&[add, activate, default_view]);
    // guest code at CODE_BASE: SVC AddViewL; SVC exit.  Id() at +0x20 returns 7.
    p.map_code(CODE_BASE, &words(&[0xEF00_0000 | STUB_SVC_BASE, 0xEF00_0000 | EXIT_SVC, 0, 0, 0, 0, 0, 0, 0xE3A0_0007, BX_LR])).unwrap();
    let appui = alloc(&mut p, 0x98);
    let view = alloc(&mut p, 0x50);
    let vtable = alloc(&mut p, 0x80);
    wr(&mut p, view, vtable);
    wr(&mut p, vtable + 3 * 4, CODE_BASE + 0x20); // slot 3: CAknView::Id
    p.cpu.regs[..2].copy_from_slice(&[appui, view]);
    p.cpu.set_pc_interworking(CODE_BASE);
    assert!(matches!(p.run(1_000), Outcome::Exited(_)));
    assert!(p.log.iter().any(|l| l.contains("has Id 0x7")), "{:?}", p.log);
    assert_eq!(p.callback_depth(), 0);
    // a known uid is queued; an unknown one leaves KErrNotFound, which needs the unwinder: a named refusal
    assert_eq!(ok(&mut p, activate, &[appui, 7]), appui, "queued (r0 untouched)");
    assert!(refused(&mut p, activate, &[appui, 99]).contains("no view with Id 0x63"));
    // SetDefaultViewL takes the MCoeView subobject (+4) of a registered view
    ok(&mut p, default_view, &[appui, view + 4]);
    assert!(refused(&mut p, default_view, &[appui, view + 0x40]).contains("not a registered view"));
}

#[test]
fn fs_private_path_bafl_utils_get_dir_and_volume_free_space() {
    let names = [
        "_ZN3RFs7ConnectEi",
        "_ZN3RFs11PrivatePathER6TDes16",
        "_ZN9BaflUtils17EnsurePathExistsLER3RFsRK7TDesC16",
        "_ZN9BaflUtils12FolderExistsER3RFsRK7TDesC16",
        "_ZN9BaflUtils10FileExistsERK3RFsRK7TDesC16",
        "_ZNK3RFs6GetDirERK7TDesC16jjRP4CDir",
        "_ZNK4CDir5CountEv",
        "_ZNK4CDirixEi",
        "_ZN4CDirD0Ev",
        "_ZN11TVolumeInfoC1Ev",
        "_ZNK3RFs6VolumeER11TVolumeInfoi",
    ];
    let mut p = proc(&names);
    let mut drive = heirloom_ekaarm_engine::drive::Drive::new();
    drive.put_file("c:\\data\\notes\\hello.txt", b"hi".to_vec()).unwrap();
    p.set_drive(drive);

    let rfs = alloc(&mut p, 8);
    // NEGATIVE CONTROL: disconnected RFs must refuse PrivatePath and GetDir.
    let buf = alloc(&mut p, 520);
    wr(&mut p, buf, 3 << 28); // EBuf (TBuf16)
    wr(&mut p, buf + 4, 256);
    assert!(refused(&mut p, "_ZN3RFs11PrivatePathER6TDes16", &[rfs, buf]).contains("not connected"));

    assert_eq!(ok(&mut p, "_ZN3RFs7ConnectEi", &[rfs, 0]), 0);
    assert_eq!(ok(&mut p, "_ZN3RFs11PrivatePathER6TDes16", &[rfs, buf]), 0);
    let len = rd(&mut p, buf) & 0x0fff_ffff;
    let units: Vec<u16> = (0..len).map(|i| p.mem.read_u16(buf + 8 + i * 2).unwrap()).collect();
    assert_eq!(String::from_utf16_lossy(&units), "\\private\\102828d6\\");

    // BaflUtils: FolderExists / EnsurePathExistsL / FileExists
    let new_file = tptrc16(&mut p, "c:\\data\\fresh\\item.cfg");
    let new_dir = tptrc16(&mut p, "c:\\data\\fresh");
    let existing_file = tptrc16(&mut p, "c:\\data\\notes\\hello.txt");
    assert_eq!(ok(&mut p, "_ZN9BaflUtils12FolderExistsER3RFsRK7TDesC16", &[rfs, new_dir]), 0);
    ok(&mut p, "_ZN9BaflUtils17EnsurePathExistsLER3RFsRK7TDesC16", &[rfs, new_file]);
    assert_eq!(ok(&mut p, "_ZN9BaflUtils12FolderExistsER3RFsRK7TDesC16", &[rfs, new_dir]), 1);
    assert_eq!(ok(&mut p, "_ZN9BaflUtils10FileExistsERK3RFsRK7TDesC16", &[rfs, existing_file]), 1);
    assert_eq!(ok(&mut p, "_ZN9BaflUtils10FileExistsERK3RFsRK7TDesC16", &[rfs, new_file]), 0);

    // RFs::GetDir + CDir::{Count, operator[], ~CDir}
    let dir_pat = tptrc16(&mut p, "c:\\data\\notes\\*");
    let cdir_slot = alloc(&mut p, 4);
    let sp = p.cpu.regs[13];
    wr(&mut p, sp, cdir_slot);
    assert_eq!(ok(&mut p, "_ZNK3RFs6GetDirERK7TDesC16jjRP4CDir", &[rfs, dir_pat, 0, 0]), 0);
    let cdir = rd(&mut p, cdir_slot);
    assert_ne!(cdir, 0);
    assert_eq!(ok(&mut p, "_ZNK4CDir5CountEv", &[cdir]), 1);
    let entry = ok(&mut p, "_ZNK4CDirixEi", &[cdir, 0]);
    let name_len = rd(&mut p, entry + 28) & 0x0fff_ffff;
    let name_units: Vec<u16> = (0..name_len).map(|i| p.mem.read_u16(entry + 32 + i * 2).unwrap()).collect();
    assert_eq!(String::from_utf16_lossy(&name_units), "hello.txt");
    // NEGATIVE CONTROL: out-of-bounds CDir index refuses.
    assert!(refused(&mut p, "_ZNK4CDirixEi", &[cdir, 1]).contains("out of bounds"));
    ok(&mut p, "_ZN4CDirD0Ev", &[cdir]);
    assert!(refused(&mut p, "_ZNK4CDir5CountEv", &[cdir]).contains("not a CDir"));

    // RFs::Volume: C: (drive 2) is writable with non-zero free space; Z: (drive 25) is write-protected with 0 free.
    let vol = alloc(&mut p, 600);
    ok(&mut p, "_ZN11TVolumeInfoC1Ev", &[vol]);
    assert_eq!(ok(&mut p, "_ZNK3RFs6VolumeER11TVolumeInfoi", &[rfs, vol, 2]), 0);
    assert_eq!(rd(&mut p, vol + 12), 0, "C: is writable (media_att == 0)");
    assert!(rd(&mut p, vol + 32) > 0, "C: reports free space");
    assert_eq!(ok(&mut p, "_ZNK3RFs6VolumeER11TVolumeInfoi", &[rfs, vol, 25]), 0);
    assert_ne!(rd(&mut p, vol + 12), 0, "Z: is write-protected ROM");
    assert_eq!(rd(&mut p, vol + 32), 0, "Z: has 0 free bytes");
}

#[test]
fn des_left_secure_id_des_array_primary_vtable_and_fullscreen_exit() {
    let names = [
        "_ZNK7TDesC164LeftEi",
        "_ZNK8RProcess8SecureIdEv",
        "_ZN16CDesC16ArrayFlatC1Ei",
        "_ZN12CDesC16Array7AppendLERK7TDesC16",
        "_ZN9CEikonEnv8InfoWinLERK7TDesC16S2_",
        "_ZN13CAknAppUiBase16SetFullScreenAppEi",
        "_ZNK13CAknAppUiBase15IsFullScreenAppEv",
        "_ZN13CAknAppUiBase4ExitEv",
    ];
    let mut p = proc(&names);

    // TDesC16::Left(TInt) with ARM EABI sret in r0
    let src = tptrc16(&mut p, "SymbianBelle");
    let sret = alloc(&mut p, 8);
    ok(&mut p, "_ZNK7TDesC164LeftEi", &[sret, src, 7]);
    assert_eq!(rd(&mut p, sret) & 0x0fff_ffff, 7);
    let ptr = rd(&mut p, sret + 4);
    let got: Vec<u16> = (0..7).map(|i| p.mem.read_u16(ptr + i * 2).unwrap()).collect();
    assert_eq!(String::from_utf16_lossy(&got), "Symbian");
    // Clamps to source length and negative length yields 0.
    ok(&mut p, "_ZNK7TDesC164LeftEi", &[sret, src, 999]);
    assert_eq!(rd(&mut p, sret) & 0x0fff_ffff, 12);
    ok(&mut p, "_ZNK7TDesC164LeftEi", &[sret, src, (-5i32) as u32]);
    assert_eq!(rd(&mut p, sret) & 0x0fff_ffff, 0);

    // RProcess::SecureId returns uid3
    assert_eq!(ok(&mut p, "_ZNK8RProcess8SecureIdEv", &[0]), p.uid3);

    // CDesC16ArrayFlat primary vtable populates slots 3 (MdcaCount) and 4 (MdcaPoint)
    let arr = alloc(&mut p, 0x20);
    ok(&mut p, "_ZN16CDesC16ArrayFlatC1Ei", &[arr, 4]);
    let item = tptrc16(&mut p, "Task1");
    ok(&mut p, "_ZN12CDesC16Array7AppendLERK7TDesC16", &[arr, item]);
    let vptr = rd(&mut p, arr);
    let count_stub = rd(&mut p, vptr + 3 * 4);
    let point_stub = rd(&mut p, vptr + 4 * 4);
    assert_ne!(count_stub, 0, "primary vtable slot 3 (MdcaCount) must be non-null");
    assert_ne!(point_stub, 0, "primary vtable slot 4 (MdcaPoint) must be non-null");

    // Static CEikonEnv::InfoWinL reads r0 and r1
    let l1 = tptrc16(&mut p, "Hello");
    let l2 = tptrc16(&mut p, "World");
    ok(&mut p, "_ZN9CEikonEnv8InfoWinLERK7TDesC16S2_", &[l1, l2]);
    assert!(p.log.iter().any(|l| l.contains("InfoWinL \"Hello\" / \"World\"")), "{:?}", p.log);

    // Full-screen toggle + Exit outcome
    assert_eq!(ok(&mut p, "_ZNK13CAknAppUiBase15IsFullScreenAppEv", &[0]), 0);
    ok(&mut p, "_ZN13CAknAppUiBase16SetFullScreenAppEi", &[0, 1]);
    assert_eq!(ok(&mut p, "_ZNK13CAknAppUiBase15IsFullScreenAppEv", &[0]), 1);
    assert!(matches!(call(&mut p, "_ZN13CAknAppUiBase4ExitEv", &[0]), (NativeOutcome::Exit(0), _)));
}

#[test]
fn modal_message_query_dialog_projects_request_and_resumes_on_answer() {
    use heirloom_ui_abi::{RecordingGuestUi, UiCall, UiObjectRef};
    let names = [
        "_ZN22CAknMessageQueryDialogC1Ev",
        "_ZN10CEikDialog9PrepareLCEi",
        "_ZNK15CAknQueryDialog12QueryHeadingEv",
        "_ZN20CAknPopupHeadingPane8SetTextLE7TPtrC16",
        "_ZN22CAknMessageQueryDialog15SetMessageTextLERK7TDesC16",
        "_ZN10CEikDialog5RunLDEv",
    ];
    let mut p = proc(&names);
    p.host.env = Some(alloc(&mut p, 0x100));
    p.set_ui(RecordingGuestUi::new());
    let dlg = alloc(&mut p, 0xe0);
    assert_eq!(ok(&mut p, "_ZN22CAknMessageQueryDialogC1Ev", &[dlg]), dlg);
    ok(&mut p, "_ZN10CEikDialog9PrepareLCEi", &[dlg, 0x1001]);
    let hp = ok(&mut p, "_ZNK15CAknQueryDialog12QueryHeadingEv", &[dlg]);
    assert_ne!(hp, 0);
    let title_des = tptrc16(&mut p, "About App");
    let (tw0, tw1) = (rd(&mut p, title_des), rd(&mut p, title_des + 4));
    ok(&mut p, "_ZN20CAknPopupHeadingPane8SetTextLE7TPtrC16", &[hp, tw0, tw1]);
    let body_des = tptrc16(&mut p, "Version 1.0");
    ok(&mut p, "_ZN22CAknMessageQueryDialog15SetMessageTextLERK7TDesC16", &[dlg, body_des]);
    let (out, _) = call(&mut p, "_ZN10CEikDialog5RunLDEv", &[dlg]);
    assert!(matches!(out, NativeOutcome::Blocked), "RunLD must park waiting for host response: {out:?}");
    let (req_ref, _) = p.nat.pending_request.expect("pending_request set");
    let ui = p.ui_as::<RecordingGuestUi>().unwrap();
    assert!(ui.calls.iter().any(|c| matches!(c, UiCall::Request(_, m) if m.title == "About App" && m.text == "Version 1.0")));
    // NEGATIVE CONTROL: mismatched RequestRef is refused and keeps dialog parked.
    let wrong = UiObjectRef { id: req_ref.id + 99, generation: 1 };
    assert!(p.deliver_request_response(wrong, Some(0)).is_err());
    assert!(p.nat.pending_request.is_some());
    // Answering OK pops cleanup stack, frees dialog, and sets r0 = EEikBidOk (1).
    p.deliver_request_response(req_ref, Some(0)).unwrap();
    assert!(p.nat.pending_request.is_none());
    assert_eq!(p.cpu.regs[0], 1);
    assert!(p.nat.cleanup.is_empty());
}

#[test]
fn get_dir_splits_files_and_dirs_with_tparse_ptrc_and_descriptors() {
    let names = [
        "_ZN3RFs7ConnectEi",
        "_ZN11RHandleBase5CloseEv",
        "_ZNK3RFs6GetDirERK7TDesC16jjRP4CDirS5_",
        "_ZNK4CDir5CountEv",
        "_ZNK4CDirixEi",
        "_ZN4CDirD0Ev",
        "_ZN10TParsePtrCC1ERK7TDesC16",
        "_ZNK10TParseBase3ExtEv",
        "_ZNK7TDesC165RightEi",
        "_ZNK7TDesC163MidEii",
        "_ZNK7TDesC1613LocateReverseE5TChar",
        "_ZNK7TDesC165FindFERKS_",
        "_ZNK7TDesC168CompareCERKS_",
        "_ZN6TDes166CopyLCERK7TDesC16",
        "_ZN6TDes164TrimEv",
    ];
    let mut p = proc(&names);
    let mut drive = heirloom_ekaarm_engine::drive::Drive::new();
    drive.put_file("c:\\root\\sub\\nested.txt", b"x".to_vec()).unwrap();
    drive.put_file("c:\\root\\demo.SIS", b"abc".to_vec()).unwrap();
    p.set_drive(drive);

    let rfs = alloc(&mut p, 8);
    assert_eq!(ok(&mut p, "_ZN3RFs7ConnectEi", &[rfs, 0]), 0);
    let pat = tptrc16(&mut p, "c:\\root\\*");
    let fslot = alloc(&mut p, 4);
    let dslot = alloc(&mut p, 4);
    let sp = p.cpu.regs[13];
    wr(&mut p, sp, fslot);
    wr(&mut p, sp + 4, dslot);
    assert_eq!(ok(&mut p, "_ZNK3RFs6GetDirERK7TDesC16jjRP4CDirS5_", &[rfs, pat, 0, 0]), 0);
    let (fdir, ddir) = (rd(&mut p, fslot), rd(&mut p, dslot));
    assert_eq!(ok(&mut p, "_ZNK4CDir5CountEv", &[fdir]), 1);
    assert_eq!(ok(&mut p, "_ZNK4CDir5CountEv", &[ddir]), 1);
    let dent = ok(&mut p, "_ZNK4CDirixEi", &[ddir, 0]);
    assert_eq!(rd(&mut p, dent) & 0x10, 0x10, "directory entry must carry KEntryAttDir");
    let fent = ok(&mut p, "_ZNK4CDirixEi", &[fdir, 0]);
    assert_eq!(rd(&mut p, fent + 4), 3, "file entry size must match");
    ok(&mut p, "_ZN4CDirD0Ev", &[fdir]);
    ok(&mut p, "_ZN4CDirD0Ev", &[ddir]);
    ok(&mut p, "_ZN11RHandleBase5CloseEv", &[rfs]);
    assert_eq!(rd(&mut p, rfs), 0, "RHandleBase::Close zeroes iHandle");

    // TParsePtrC::Ext on a 28-byte object + TDes16::CopyLC / Trim / Right / Mid / FindF / CompareC
    let path_des = tptrc16(&mut p, "  C:\\root\\DEMO.SIS  ");
    let buf = alloc(&mut p, 128);
    wr(&mut p, buf, 3 << 28);
    wr(&mut p, buf + 4, 50);
    ok(&mut p, "_ZN6TDes166CopyLCERK7TDesC16", &[buf, path_des]);
    ok(&mut p, "_ZN6TDes164TrimEv", &[buf]);
    let pp = alloc(&mut p, 0x20);
    ok(&mut p, "_ZN10TParsePtrCC1ERK7TDesC16", &[pp, buf]);
    let sret = alloc(&mut p, 8);
    ok(&mut p, "_ZNK10TParseBase3ExtEv", &[sret, pp]);
    let ext_cmp = tptrc16(&mut p, ".SIS");
    assert_eq!(ok(&mut p, "_ZNK7TDesC168CompareCERKS_", &[sret, ext_cmp]), 0);
    assert_eq!(ok(&mut p, "_ZNK7TDesC1613LocateReverseE5TChar", &[buf, b'\\' as u32]), 7);
    let needle = tptrc16(&mut p, "DEMO");
    assert_eq!(ok(&mut p, "_ZNK7TDesC165FindFERKS_", &[buf, needle]), 8);
    ok(&mut p, "_ZNK7TDesC165RightEi", &[sret, buf, 4]);
    assert_eq!(rd(&mut p, sret) & 0x0fff_ffff, 4);
    // NEGATIVE CONTROL: Mid past length is refused by name.
    assert!(refused(&mut p, "_ZNK7TDesC163MidEii", &[sret, buf, 99, 1]).contains("outside length"));
}
