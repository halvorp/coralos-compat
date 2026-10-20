//! The procID table (WDEF slice 1, docs/consults/2026-10-20-wdef.md): a custom window definition is refused by name before
//! anything is allocated or shown; the system WDEFs keep their native roles, each by its own row.
use guestui_abi::*;
use mac68k_engine::pascal::Rect;
use mac68k_engine::toolbox::WindowSpec;
use mac68k_engine::Mac;

fn spec(proc_id: u16) -> WindowSpec {
    WindowSpec { storage: 0, rect: Rect { top: 100, left: 50, bottom: 300, right: 250 }, title: "W".into(), visible: true, proc_id, behind: 0xFFFF_FFFF, go_away: true, ref_con: 0 }
}

fn opened(m: &Mac<RecordingGuestUi>) -> Vec<WindowModel> {
    m.ui.calls.iter().filter_map(|c| if let UiCall::OpenWindow(w) = c { Some(w.clone()) } else { None }).collect()
}

#[test]
fn a_custom_wdef_is_refused_by_name_and_leaves_nothing_behind() {
    for p in [2048u16, 2064, 128 * 16 + 3, 0x7FF0] {
        let mut m = Mac::new(RecordingGuestUi::default()).unwrap();
        let e = m.make_window(spec(p)).unwrap_err();
        assert!(e.0.contains("WDEF-UNSUPPORTED"), "procID {p}: {}", e.0);
        assert!(e.0.contains(&format!("WDEF {}", p >> 4)), "procID {p}: {}", e.0);
        assert_eq!((m.mm.live_ptrs(), m.windows.len(), m.z.len(), opened(&m).len()), (0, 0, 0, 0), "procID {p}");
    }
}

#[test]
fn the_system_wdefs_keep_their_native_roles() {
    for (p, role, resizable) in [
        (0u16, WindowRole::Document, true),
        (8, WindowRole::Document, true),
        (4, WindowRole::Document, false),
        (12, WindowRole::Document, false),
        (1, WindowRole::Dialog, false),
        (2, WindowRole::Dialog, false),
        (3, WindowRole::Dialog, false),
        (5, WindowRole::Dialog, false),
        (16, WindowRole::Document, false),
        (16 + 6, WindowRole::Document, false),
    ] {
        let mut m = Mac::new(RecordingGuestUi::default()).unwrap();
        m.make_window(spec(p)).unwrap();
        let o = opened(&m);
        assert_eq!(o.len(), 1, "procID {p}");
        assert!(o[0].role == role, "procID {p}");
        assert_eq!(o[0].resizable, resizable, "procID {p}");
    }
}

#[test]
fn the_rounded_window_is_its_own_row_not_procid_0_by_its_low_bits() {
    let mut a = Mac::new(RecordingGuestUi::default()).unwrap();
    a.make_window(spec(0)).unwrap();
    let oa = opened(&a);
    let mut b = Mac::new(RecordingGuestUi::default()).unwrap();
    b.make_window(spec(16)).unwrap();
    let ob = opened(&b);
    assert_eq!(oa.len(), 1, "procID 0");
    assert_eq!(ob.len(), 1, "procID 16");
    assert!(oa[0].resizable, "procID 0 has a grow box");
    assert!(!ob[0].resizable, "procID 16 is the rounded document window, no grow box");
}
