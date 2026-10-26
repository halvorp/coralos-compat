//! The guest UI wire is transparent: one UI session made directly on a `GuestUi` and made through `ProxyUi` -> frames ->
//! `WireHost` records the same calls, ids and events translate both ways, and a host refusal comes back with its call token.

use heirloom_ui_abi::wire::loopback::Loopback;
use heirloom_ui_abi::wire::proxy::{ProxyUi, WireTransport};
use heirloom_ui_abi::*;

fn rect(left: i32, top: i32, width: i32, height: i32) -> GuestRect {
    GuestRect { left, top, width, height }
}

fn window(title: &str, token: u64) -> WindowModel {
    WindowModel {
        title: title.into(),
        rect: rect(0, 0, 320, 200),
        role: WindowRole::Document,
        closable: true,
        resizable: true,
        token: CallbackToken(token),
    }
}

fn control(kind: ControlKind, token: u64) -> ControlModel {
    ControlModel {
        kind,
        label: "control".into(),
        rect: rect(1, 1, 40, 20),
        enabled: true,
        token: CallbackToken(token),
    }
}

fn request_model() -> RequestModel {
    RequestModel {
        title: "Question".into(),
        text: "Continue?".into(),
        buttons: vec!["OK".into()],
        default: Some(0),
        cancel: Some(0),
        kind: RequestKind::default(),
    }
}

fn audio_model() -> AudioStreamModel {
    AudioStreamModel {
        rate_hz: 22_054,
        channels: 1,
        format: AudioFormat::U8,
        label: "wire-test".into(),
    }
}

/// Drive one complete session, chaining every object reference returned by the
/// UI under test. The third control is deliberately not used after creation:
/// the refusal test refuses only its `add_control`, without cascading into
/// stale-control refusals later in the session.
fn script(ui: &mut dyn GuestUi) {
    // Not in the session: `icon_image` (the named limit in proxy.rs) and audio (never on this wire).
    let w1 = ui.open_window(window("first", 101)).unwrap();
    let w2 = ui.open_window(window("second", 102)).unwrap();

    let c1 = ui
        .add_control(w1, control(ControlKind::Checkbox { checked: false }, 111))
        .unwrap();
    let c2 = ui
        .add_control(w1, control(ControlKind::ToggleButton { on: true }, 112))
        .unwrap();
    let _c3 = ui
        .add_control(
            w2,
            control(ControlKind::Slider { min: 0, max: 10, value: 3 }, 113),
        )
        .unwrap();

    ui.set_control(w1, c1, ControlValue::Checked(true)).unwrap();
    ui.set_control(w1, c2, ControlValue::Checked(false)).unwrap();
    ui.set_control_rect(w1, c1, rect(2, 3, 60, 24)).unwrap();
    ui.set_control_enabled(w1, c1, false).unwrap();

    ui.set_window_title(w1, "retitled".into()).unwrap();
    ui.set_menu_strip(w1, MenuModel::default()).unwrap();
    ui.clear_menu_strip(w1).unwrap();
    ui.hide_window(w1).unwrap();
    ui.show_window(w1).unwrap();

    let request = ui.request(Some(w1), request_model()).unwrap();
    ui.close_request(request).unwrap();

    let pixels = [0x00ff_4000u32; 16];
    ui.present_content(w1, rect(0, 0, 4, 4), 4, 4, &pixels)
        .unwrap();
    ui.present_plane(
        w1,
        Plane {
            id: 7,
            rect: rect(0, 0, 4, 4),
            ops: vec![DrawOp::Fill {
                rect: rect(0, 0, 4, 4),
                pen_role: PenRole::Background,
            }],
        },
    )
    .unwrap();

    ui.remove_control(w1, c1).unwrap();
    ui.close_window(w1).unwrap();
    ui.close_window(w2).unwrap();

    // No answers or native input are queued by this session. In the refusal
    // test this also drains the pending refusal frame into `take_refusals`.
    assert_eq!(ui.poll_event(), None);
}

#[test]
fn direct_and_loopback_record_identical_calls() {
    let mut direct = RecordingGuestUi::new();
    script(&mut direct);

    let mut proxy = ProxyUi::new(Loopback::new(RecordingGuestUi::new()));
    script(&mut proxy);

    assert!(proxy.take_refusals().is_empty());
    assert_eq!(proxy.bad_frames(), 0);
    assert_eq!(&direct.calls, &proxy.transport().ui().calls);
}

#[test]
fn swallowing_the_fifth_send_breaks_equivalence() {
    let mut direct = RecordingGuestUi::new();
    script(&mut direct);

    let transport = SwallowFifthSend {
        inner: Loopback::new(RecordingGuestUi::new()),
        send_count: 0,
    };
    let mut proxy = ProxyUi::new(transport);
    script(&mut proxy);

    assert!(proxy.transport().send_count >= 5);
    assert_ne!(&direct.calls, &proxy.transport().inner.ui().calls);
}

#[test]
fn host_events_return_with_the_engine_window_id() {
    let mut proxy = ProxyUi::new(Loopback::new(RecordingGuestUi::new()));

    // Allocate a host request object first. The subsequent window is therefore
    // host id 2 but engine window id 1, making the reverse translation visible.
    let _request = proxy.request(None, request_model()).unwrap();
    let engine_win = proxy.open_window(window("events", 201)).unwrap();
    let _engine_control = proxy
        .add_control(
            engine_win,
            control(ControlKind::Checkbox { checked: false }, 202),
        )
        .unwrap();

    let (host_win, _, _) = proxy
        .transport()
        .ui()
        .find_control(|model| model.token == CallbackToken(202))
        .expect("the host recorded the control");
    assert_ne!(host_win, engine_win);

    let token = CallbackToken(201);
    proxy
        .transport_mut()
        .ui_mut()
        .events
        .push_back(GuestUiEvent::CloseRequested { win: host_win, token });

    assert_eq!(
        proxy.poll_event(),
        Some(GuestUiEvent::CloseRequested { win: engine_win, token })
    );
    assert_eq!(proxy.poll_event(), None);
}

#[test]
fn a_host_refusal_returns_its_one_based_call_token() {
    let mut host = RecordingGuestUi::new();
    host.refuse_kinds.push("Slider");

    let mut proxy = ProxyUi::new(Loopback::new(host));
    script(&mut proxy);

    let refusals = proxy.take_refusals();
    assert_eq!(refusals.len(), 1);
    // OpenWindow, OpenWindow, AddControl, AddControl, then the refused Slider
    // AddControl. The preceding canvas lease is later and is not a UiCall.
    assert_eq!(refusals[0].0, 5);
    assert!(matches!(refusals[0].1.to_ui(), UiRefusal::Unsupported(_)));
}

#[test]
fn audio_never_rides_the_ui_wire() {
    // the engine plays through its own Aura session (23-ENGINE-SPLIT.md:256): the proxy refuses by name and sends nothing
    let mut proxy = ProxyUi::new(Loopback::new(RecordingGuestUi::new()));
    assert!(matches!(proxy.open_audio_stream(audio_model()), Err(UiRefusal::Unsupported(_))));
    assert!(proxy.transport().ui().calls.is_empty());
}

#[test]
fn bad_magic_is_counted_without_panicking() {
    let transport = BadFrameTransport {
        bad_frame: Some(vec![0; 8]),
    };
    let mut proxy = ProxyUi::new(transport);

    assert_eq!(proxy.poll_event(), None);
    assert_eq!(proxy.bad_frames(), 1);
    assert_eq!(proxy.poll_event(), None);
    assert_eq!(proxy.bad_frames(), 1);
}

struct SwallowFifthSend<T: WireTransport> {
    inner: T,
    send_count: u32,
}

impl<T: WireTransport> WireTransport for SwallowFifthSend<T> {
    fn send(&mut self, frame: &[u8]) {
        self.send_count += 1;
        if self.send_count != 5 {
            self.inner.send(frame);
        }
    }

    fn ask(&mut self, frame: &[u8]) -> Vec<u8> {
        self.inner.ask(frame)
    }

    fn recv(&mut self) -> Option<Vec<u8>> {
        self.inner.recv()
    }

    fn lease_canvas(
        &mut self,
        win: UiObjectRef,
        width: i32,
        height: i32,
        pixels: &[u32],
    ) -> u32 {
        self.inner.lease_canvas(win, width, height, pixels)
    }
}

struct BadFrameTransport {
    bad_frame: Option<Vec<u8>>,
}

impl WireTransport for BadFrameTransport {
    fn send(&mut self, _frame: &[u8]) {}

    fn ask(&mut self, _frame: &[u8]) -> Vec<u8> {
        Vec::new()
    }

    fn recv(&mut self) -> Option<Vec<u8>> {
        self.bad_frame.take()
    }

    fn lease_canvas(
        &mut self,
        _win: UiObjectRef,
        _width: i32,
        _height: i32,
        _pixels: &[u32],
    ) -> u32 {
        0
    }
}
#[test]
fn a_call_longer_than_a_frame_is_refused_and_ids_stay_in_step() {
    let mut proxy = ProxyUi::new(Loopback::new(RecordingGuestUi::new()));
    let big = "x".repeat(heirloom_ui_abi::wire::MAX_FRAME as usize + 1);
    assert!(matches!(proxy.open_window(window(&big, 1)), Err(UiRefusal::Unsupported(_))));
    assert!(proxy.transport().ui().calls.is_empty(), "nothing was sent");
    let w = proxy.open_window(window("next", 2)).unwrap();
    assert_eq!(w.id, 1, "the refused call minted no id");
}
