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
