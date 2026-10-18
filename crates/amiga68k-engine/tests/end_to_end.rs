use amiga68k_engine::fixture::build_hunk;
use amiga68k_engine::Amiga;
use guestui_abi::*;

fn run(file: &[u8]) -> (Amiga<RecordingGuestUi>, amiga68k_engine::RunReport) {
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    let r = a.run_hunk(file, 100_000).expect("load+run");
    (a, r)
}

#[test]
fn bridged_sequence_reaches_guestui() {
    let (a, r) = run(&build_hunk(None));
    assert_eq!(a.log, vec![
        "[exec] OpenLibrary name=\"intuition.library\" ver=0 -> 0x2000",
        "[intuition] OpenWindow left=10 top=20 w=320 h=200 title=\"Hello\" -> 0x200000",
        "[intuition] SetMenuStrip handle=0x200000 menus=1",
        "[intuition] CloseWindow 0x200000",
        "[exec] CloseLibrary intuition.library base=0x2000",
    ]);
    assert!(a.refused.is_empty());
    assert_eq!(a.mem.read_u32(r.loaded.bases[2]).unwrap(), 0x200000, "OpenWindow d0 (the guest struct Window) stored via reloc'd BSS address");
    assert_eq!(a.heap.live_blocks(), 0, "CloseWindow gave the struct Window back");
    assert_eq!(r.stats.bus_faults, 0);
    assert!(r.stats.halted && r.stats.insns > 10 && r.stats.insns < 200, "{}", r.stats.insns);
    assert_eq!(a.libs.iter().find(|l| l.name == "intuition.library").unwrap().open_count, 0);

    let win = UiObjectRef { id: 1, generation: 1 };
    let item = |label: &str, key: char, i: usize| MenuItem {
        label: label.into(), enabled: true, shortcut: Some(key), check: None, sub_items: vec![],
        token: CallbackToken(0 | (i as u64) << 5 | 0x1F << 11),
    };
    assert_eq!(a.ui.calls, vec![
        UiCall::OpenWindow(WindowModel {
            title: "Hello".into(), rect: GuestRect { left: 10, top: 20, width: 320, height: 200 },
            role: WindowRole::Document, closable: true, resizable: true, token: CallbackToken(0x200000),
        }),
        UiCall::SetMenuStrip(win, MenuModel { menus: vec![Menu {
            title: "Project".into(), enabled: true, items: vec![item("Open", 'O', 0), item("Quit", 'Q', 1)],
        }]}),
        UiCall::CloseWindow(win),
    ]);
}

#[test]
fn out_of_range_title_pointer_is_refused_by_name() {
    let (a, r) = run(&build_hunk(Some(0xFFFF_0000)));
    assert_eq!(a.refused[0], "intuition.library/OpenWindow: NewWindow.Title: pointer 0xffff0000 outside guest RAM");
    assert!(a.log[1].starts_with("[intuition.library] OpenWindow REFUSED"));
    assert_eq!(a.mem.read_u32(r.loaded.bases[2]).unwrap(), 0, "refused call returns NULL");
    assert_eq!(r.stats.bus_faults, 0, "the bridge refused it before the guest could fault");
    assert!(a.ui.calls.is_empty(), "nothing reached guestui");
    // the program then passes NULL to SetMenuStrip/CloseWindow: refused by name too, still no crash
    assert!(a.refused.iter().any(|e| e.starts_with("intuition.library/SetMenuStrip")));
    assert!(a.refused.iter().any(|e| e.starts_with("intuition.library/CloseWindow")));
    assert!(r.stats.halted);
}

#[test]
fn unimplemented_lvo_is_a_named_refusal() {
    use amiga68k_engine::machine::GRAPHICS_BASE;
    use retro_hunk::writer::{be16, be32, build, Hunk};
    // movea.l #GRAPHICS_BASE,a6 ; jsr -330(a6) (graphics Flood: not implemented) ; rts
    let mut c = vec![];
    be16(&mut c, 0x2C7C); be32(&mut c, GRAPHICS_BASE);
    be16(&mut c, 0x4EAE); be16(&mut c, 0xFEB6);
    be16(&mut c, 0x4E75);
    let (a, _) = run(&build(&[Hunk::code(c, vec![])], None));
    assert_eq!(a.refused, vec!["graphics.library/Flood: unimplemented".to_string()]);
    assert_eq!(a.log, vec!["[graphics.library] Flood REFUSED: unimplemented".to_string()]);
}

#[test]
fn census_stops_at_the_first_unimplemented_call_and_names_it() {
    use amiga68k_engine::machine::GRAPHICS_BASE;
    use retro_hunk::writer::{be16, be32, build, Hunk};
    // movea.l #GRAPHICS_BASE,a6 ; jsr -330(a6) (Flood, unimplemented) ; jsr -312(a6) (BltPattern, unimplemented) ; rts
    let mut c = vec![];
    be16(&mut c, 0x2C7C); be32(&mut c, GRAPHICS_BASE);
    be16(&mut c, 0x4EAE); be16(&mut c, 0xFEB6);
    be16(&mut c, 0x4EAE); be16(&mut c, 0xFEC8);
    be16(&mut c, 0x4E75);
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    a.stop_on_unimplemented = true;
    let r = a.run_hunk(&build(&[Hunk::code(c.clone(), vec![])], None), 10_000).unwrap();
    assert!(r.stats.halted);
    assert_eq!(a.refused, vec!["graphics.library/Flood: unimplemented".to_string()], "stopped at the FIRST one");
    // CONTROL: without census mode the same program continues past Flood and reaches BltPattern.
    let mut b = Amiga::new(RecordingGuestUi::new()).unwrap();
    b.run_hunk(&build(&[Hunk::code(c, vec![])], None), 10_000).unwrap();
    assert_eq!(b.refused.len(), 2, "{:?}", b.refused);
    assert!(b.refused[1].starts_with("graphics.library/BltPattern"), "{:?}", b.refused);
}
