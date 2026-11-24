//! Witness tests for declarative in-plane chrome bindings (`HEIRLOOM-APP-CHROME-NEEDS-A-BINDING`):
//! custom in-plane toolbar/ruler chrome stays raw when unbound or when the resource digest mismatches,
//! and projects native `DrawOp::Face` + `DrawOp::Icon` (with active state) only when the exact
//! `(res_type, res_id, length, fnv1a64)` matches.
use heirloom_bind::{BandRule, ButtonRule, PlaneChromeBinding, PlaneChromeIdentity};
use heirloom_fmt_resfork::writer::ForkBuilder;
use heirloom_mac68k_engine::pascal::Rect;
use heirloom_mac68k_engine::qd::draw::paint;
use heirloom_mac68k_engine::qd::port::PAT_COPY;
use heirloom_mac68k_engine::qd::shapes::rect as qrect;
use heirloom_mac68k_engine::toolbox::WindowSpec;
use heirloom_mac68k_engine::Mac;
use heirloom_raster::Rect as RRect;
use heirloom_ui_abi::{DrawOp, GuestRect, IconRole, RecordingGuestUi, UiCall};

/// QuickDraw `patXor` transfer mode (invert).
const PAT_XOR: u16 = 10;

fn run_with_binding(code_bytes: &[u8], binding: Option<PlaneChromeBinding>) -> Vec<DrawOp> {
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    let fb = ForkBuilder::new().add(b"CODE", 1, None, code_bytes);
    let fork = heirloom_fmt_resfork::ResFork::parse(fb.build()).unwrap();
    m.rm.open_fork(2, fork);
    if let Some(b) = binding {
        m.add_plane_chrome_binding(b);
    }
    let win = m
        .make_window(WindowSpec {
            storage: 0,
            rect: Rect { top: 40, left: 40, bottom: 240, right: 540 },
            title: "Doc".into(),
            visible: true,
            proc_id: 0,
            behind: u32::MAX,
            go_away: true,
            ref_con: 0,
        })
        .unwrap();
    paint(&mut m, win, &qrect(RRect::new(0, 0, 487, 46)), PAT_COPY, &[0; 8]).unwrap();
    // Active button inversion in (193, 29, 28, 12)
    paint(&mut m, win, &qrect(RRect::new(193, 29, 221, 41)), PAT_XOR, &[0xFF; 8]).unwrap();
    // Bottom-right grow box
    paint(&mut m, win, &qrect(RRect::new(485, 185, 500, 200)), PAT_COPY, &[0xFF; 8]).unwrap();
    heirloom_mac68k_engine::qd::plane::flush(&mut m);
    m.ui
        .calls
        .iter()
        .rev()
        .find_map(|c| match c {
            UiCall::PresentPlane(_, p) => Some(p.ops.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

fn sample_binding(code_bytes: &[u8], hash: u64) -> PlaneChromeBinding {
    PlaneChromeBinding {
        id: "ruler-test".into(),
        identity: PlaneChromeIdentity::Resource {
            res_type: *b"CODE",
            res_id: 1,
            length: code_bytes.len() as u32,
            fnv1a64: hash,
        },
        present_if_text: None,
        bands: vec![BandRule {
            rect: GuestRect { left: 0, top: 0, width: 487, height: 46 },
            remap_1bit_pixels: true,
        }],
        buttons: vec![
            ButtonRule {
                rect: GuestRect { left: 191, top: 26, width: 33, height: 18 },
                clear_rect: GuestRect { left: 190, top: 25, width: 35, height: 20 },
                active_rect: Some(GuestRect { left: 193, top: 29, width: 28, height: 12 }),
                label: "1\u{00d7}".into(),
                icon: None,
            },
            ButtonRule {
                rect: GuestRect { left: 327, top: 26, width: 33, height: 18 },
                clear_rect: GuestRect { left: 326, top: 25, width: 35, height: 20 },
                active_rect: Some(GuestRect { left: 329, top: 29, width: 28, height: 12 }),
                label: " ".into(),
                icon: Some(IconRole::AlignLeft),
            },
        ],
        suppress_bottom_right: Some((22, 22)),
        window_range: None,
        pixel_guards: vec![],
        palette_rules: vec![],
    }
}

#[test]
fn plane_chrome_binding_applies_only_on_exact_resource_match() {
    let code = [0x4Eu8, 0x56, 0x00, 0x00, 0x70, 0x01, 0x4E, 0x5E, 0x4E, 0x75];

    // 1. Unbound: no Face/Icon ops emitted.
    let unbound = run_with_binding(&code, None);
    assert!(!unbound.iter().any(|o| matches!(o, DrawOp::Face { .. } | DrawOp::Icon { .. })));

    // 2. Wrong digest (negative control): untouched.
    let wrong = run_with_binding(&code, Some(sample_binding(&code, 0xDEAD_BEEF)));
    assert!(!wrong.iter().any(|o| matches!(o, DrawOp::Face { .. } | DrawOp::Icon { .. })));

    // 3. Exact match: emits active Face("1×"), inactive Face + Icon(AlignLeft), suppresses bottom-right box.
    let exact = run_with_binding(&code, Some(sample_binding(&code, heirloom_bind::wdef::fnv1a64(&code))));
    assert!(exact.iter().any(|o| matches!(o, DrawOp::Face { label, pressed: true, .. } if label == "1\u{00d7}")));
    assert!(exact.iter().any(|o| matches!(o, DrawOp::Icon { role: IconRole::AlignLeft, .. })));
    assert!(
        !exact.iter().any(|o| matches!(o, DrawOp::Fill { rect, .. } if rect.left >= 480 && rect.top >= 180)),
        "bottom-right grow box should be suppressed"
    );
}
