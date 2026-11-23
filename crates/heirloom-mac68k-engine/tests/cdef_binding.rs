//! Witness tests for declarative custom `CDEF` bindings (`HEIRLOOM-APP-CHROME-NEEDS-A-BINDING`):
//! a custom `procID` (`CDEF` resource ID = `procID >> 4`) stays in the content plane when unbound
//! or when the resource digest mismatches, and projects as the declared native kit control when
//! its exact `(res_id, length, fnv1a64)` matches a loaded `ControlDefBinding`.
use heirloom_bind::{ControlDefBinding, ControlDefIdentity, ControlDefKind};
use heirloom_fmt_resfork::writer::ForkBuilder;
use heirloom_mac68k_engine::pascal::Rect;
use heirloom_mac68k_engine::toolbox::WindowSpec;
use heirloom_mac68k_engine::Mac;
use heirloom_ui_abi::{ControlKind, RecordingGuestUi, UiCall};

fn make_mac_with_cdef(cdef_bytes: &[u8]) -> (Mac<RecordingGuestUi>, u32) {
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    let fb = ForkBuilder::new().add(b"CDEF", 2, None, cdef_bytes);
    let fork = heirloom_fmt_resfork::ResFork::parse(fb.build()).unwrap();
    m.rm.open_fork(2, fork);
    let win = m.make_window(WindowSpec {
        storage: 0,
        rect: Rect { top: 40, left: 40, bottom: 240, right: 340 },
        title: "Doc".into(),
        visible: true,
        proc_id: 0,
        behind: u32::MAX,
        go_away: true,
        ref_con: 0,
    }).unwrap();
    (m, win)
}

#[test]
fn custom_cdef_projects_natively_only_when_exact_binding_matches() {
    let cdef = [0x4Eu8, 0x56, 0x00, 0x00, 0x70, 0x00, 0x4E, 0x5E, 0x4E, 0x75];
    let rect = Rect { top: 0, left: 284, bottom: 184, right: 300 };

    // 1. Unbound: falls back to the content plane (no native AddControl).
    let (mut m, win) = make_mac_with_cdef(&cdef);
    m.create_control(win, rect, "p01", true, 0, 0, 100, 32, 0).unwrap();
    assert!(!m.ui.calls.iter().any(|c| matches!(c, UiCall::AddControl(..))), "unbound CDEF 2 must not project natively");

    // 2. Mismatched digest (negative control): still unbound.
    let (mut m, win) = make_mac_with_cdef(&cdef);
    m.add_control_def_binding(ControlDefBinding {
        id: "wrong-hash".into(),
        identity: ControlDefIdentity::Resource { res_id: 2, length: cdef.len() as u32, fnv1a64: 0xDEAD_BEEF },
        kind: ControlDefKind::ScrollBar,
    });
    m.create_control(win, rect, "p01", true, 0, 0, 100, 32, 0).unwrap();
    assert!(!m.ui.calls.iter().any(|c| matches!(c, UiCall::AddControl(..))), "mismatched hash must not project natively");

    // 3. Exact (res_id, length, fnv1a64) binding: projects as native ScrollBar!
    let (mut m, win) = make_mac_with_cdef(&cdef);
    m.add_control_def_binding(ControlDefBinding {
        id: "exact-cdef2".into(),
        identity: ControlDefIdentity::Resource {
            res_id: 2,
            length: cdef.len() as u32,
            fnv1a64: heirloom_bind::wdef::fnv1a64(&cdef),
        },
        kind: ControlDefKind::ScrollBar,
    });
    m.create_control(win, rect, "p01", true, 10, 0, 100, 32, 0).unwrap();
    let added = m.ui.calls.iter().find_map(|c| match c {
        UiCall::AddControl(_, ctl) => Some(ctl.clone()),
        _ => None,
    });
    let ctl = added.expect("bound CDEF 2 must project as a native control");
    assert!(matches!(ctl.kind, ControlKind::ScrollBar { vertical: true, total: 101, visible: 1, top: 10 }), "got {:?}", ctl.kind);
}
