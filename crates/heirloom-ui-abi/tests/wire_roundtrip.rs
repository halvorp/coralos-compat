use heirloom_ui_abi::wire::{WireRefusal, 
    decode, encode_call, encode_event, encode_lease, encode_refused, Frame, WireError, MAX_FRAME,
};
use heirloom_ui_abi::*;

fn object(id: u32) -> UiObjectRef {
    UiObjectRef { id, generation: id + 10 }
}

fn rect() -> GuestRect {
    GuestRect { left: -2, top: 3, width: 40, height: 30 }
}

fn window() -> WindowModel {
    WindowModel {
        title: "Wire window".into(),
        rect: rect(),
        role: WindowRole::Dialog,
        closable: true,
        resizable: false,
        token: CallbackToken(0x1122_3344_5566_7788),
    }
}

fn control(kind: ControlKind) -> ControlModel {
    ControlModel {
        kind,
        label: "control label".into(),
        rect: rect(),
        enabled: true,
        token: CallbackToken(91),
    }
}

fn menu() -> MenuModel {
    MenuModel {
        menus: vec![Menu {
            title: "File".into(),
            enabled: true,
            items: vec![MenuItem {
                label: "Open".into(),
                enabled: true,
                shortcut: Some('ö'),
                check: Some(false),
                sub_items: vec![MenuItem {
                    label: "Recent".into(),
                    enabled: false,
                    shortcut: None,
                    check: None,
                    sub_items: vec![],
                    token: CallbackToken(7),
                }],
                token: CallbackToken(6),
            }],
        }],
    }
}

fn plane() -> Plane {
    Plane {
        id: 17,
        rect: rect(),
        ops: vec![
            DrawOp::Fill { rect: rect(), pen_role: PenRole::Surface },
            DrawOp::Text {
                at: GuestPoint { x: 4, y: 8 },
                string: "plane text".into(),
                font_role: FontRole::Mono,
                px: 14,
                width: 120,
                pen_role: PenRole::Text,
                bg_role: Some(PenRole::Background),
            },
            DrawOp::Line {
                a: GuestPoint { x: 0, y: 0 },
                b: GuestPoint { x: 9, y: 11 },
                pen_role: PenRole::Accent,
            },
            DrawOp::Bevel { rect: rect(), raised: true },
            DrawOp::Icon { rect: rect(), role: IconRole::Drawer },
            DrawOp::Pixels {
                rect: GuestRect { left: 0, top: 0, width: 1, height: 2 },
                rgb: vec![0x0011_2233, 0x0044_5566],
            },
        ],
    }
}

fn request() -> RequestModel {
    RequestModel {
        title: "Question".into(),
        text: "Continue?".into(),
        buttons: vec!["Yes".into(), "No".into()],
        default: Some(0),
        cancel: Some(1),
        kind: RequestKind::Caution,
    }
}

fn audio(format: AudioFormat) -> AudioStreamModel {
    AudioStreamModel {
        rate_hz: 44_100,
        channels: 2,
        format,
        label: "wire audio".into(),
    }
}

fn calls() -> Vec<UiCall> {
    let kinds = vec![
        ControlKind::Button,
        ControlKind::Checkbox { checked: true },
        ControlKind::String { text: "value".into(), max_len: 20 },
        ControlKind::Slider { min: -4, max: 9, value: 2 },
        ControlKind::Label { text: "label".into(), framed: true },
        ControlKind::ToggleButton { on: true },
        ControlKind::ScrollBar { vertical: false, total: 100, visible: 10, top: 20 },
        ControlKind::Field {
            text: "-12".into(),
            max_len: 8,
            editable: true,
            integer: true,
        },
        ControlKind::Group { title: "group".into(), style: GroupStyle::Raised },
        ControlKind::ArrowButton { dir: ArrowDir::Left },
        ControlKind::Hotspot,
        ControlKind::Choice {
            style: ChoiceStyle::Radio,
            labels: vec!["one".into(), "two".into()],
            selected: 1,
        },
        ControlKind::List {
            columns: vec!["name".into(), "value".into()],
            rows: vec![vec!["a".into(), "b".into()]],
            selected: Some(0),
            top: 0,
        },
        ControlKind::Gauge { value: 3, max: 10 },
        ControlKind::Swatches { colors: vec![1, 2, 3], selected: 2 },
    ];
    let values = vec![
        ControlValue::Pressed,
        ControlValue::Checked(true),
        ControlValue::Text("new text".into()),
        ControlValue::Level(-7),
        ControlValue::Scroll { total: 50, visible: 5, top: 9 },
        ControlValue::Choice(4),
        ControlValue::Rows {
            rows: vec![vec!["row".into()]],
            selected: None,
            top: 0,
        },
        ControlValue::Cancelled,
    ];
    let mut result = vec![
        UiCall::OpenWindow(window()),
        UiCall::CloseWindow(object(1)),
        UiCall::SetWindowTitle(object(2), "new title".into()),
        UiCall::HideWindow(object(3)),
        UiCall::ShowWindow(object(4)),
        UiCall::SetMenuStrip(object(5), menu()),
        UiCall::ClearMenuStrip(object(6)),
        UiCall::RemoveControl(object(7), object(8)),
        UiCall::SetControlRect(object(9), object(10), rect()),
        UiCall::SetControlEnabled(object(11), object(12), false),
        UiCall::PresentContent {
            win: object(13),
            damage: rect(),
            width: 320,
            height: 200,
            hash: 0xfedc_ba98_7654_3210,
        },
        UiCall::PresentPlane(object(14), plane()),
        UiCall::Request(Some(object(15)), request()),
        UiCall::CloseRequest(object(16)),
        UiCall::OpenAudio(audio(AudioFormat::U8)),
        UiCall::OpenAudio(audio(AudioFormat::S16Le)),
        UiCall::QueueAudio(object(17), 123_456),
        UiCall::QueueAudioLooped(object(18), 4096, 12, 900),
        UiCall::FlushAudio(object(19)),
        UiCall::PauseAudio(object(20), true),
        UiCall::SetAudioVolume(object(21), 0x100, 0x80),
        UiCall::CloseAudio(object(22)),
    ];
    result.extend(kinds.into_iter().map(|kind| UiCall::AddControl(object(30), control(kind))));
    result.extend(
        values
            .into_iter()
            .map(|value| UiCall::SetControl(object(31), object(32), value)),
    );
    result
}

fn events() -> Vec<GuestUiEvent> {
    let intents = vec![
        Intent::Activate,
        Intent::SetValue(ControlValue::Checked(false)),
        Intent::SelectRow(8),
        Intent::ScrollTo(14),
        Intent::CommitText("committed".into()),
        Intent::Press,
        Intent::Release { inside: false },
        Intent::Drag(ControlValue::Level(6)),
    ];
    let mut result = vec![
        GuestUiEvent::CloseRequested { win: object(1), token: CallbackToken(2) },
        GuestUiEvent::MenuCommand { win: object(3), token: CallbackToken(4) },
        GuestUiEvent::Resized { win: object(5), rect: rect() },
        GuestUiEvent::Pointer {
            win: object(6),
            x: -4,
            y: 8,
            button: PointerButton::Primary,
            phase: PointerPhase::Down,
            clicks: 2,
        },
        GuestUiEvent::Pointer {
            win: object(6),
            x: 0,
            y: 1,
            button: PointerButton::None,
            phase: PointerPhase::Move,
            clicks: 0,
        },
        GuestUiEvent::Pointer {
            win: object(6),
            x: 2,
            y: 3,
            button: PointerButton::Secondary,
            phase: PointerPhase::Up,
            clicks: 0,
        },
        GuestUiEvent::Key {
            win: object(7),
            code: 65,
            text: "A".into(),
            modifiers: Modifiers(Modifiers::SHIFT | Modifiers::CTRL),
            down: true,
        },
        GuestUiEvent::RequestAnswered { req: object(8), button: 1 },
    ];
    result.extend(intents.into_iter().map(|intent| GuestUiEvent::Intent {
        win: object(9),
        control: object(10),
        token: CallbackToken(11),
        generation: 12,
        intent,
    }));
    result
}

#[test]
fn every_call_shape_roundtrips() {
    for call in calls() {
        let mut bytes = vec![];
        encode_call(&call, &mut bytes).unwrap();
        let (decoded, consumed) = decode(&bytes).unwrap();
        assert_eq!(consumed, bytes.len());
        assert_eq!(decoded, Frame::Call(call));
    }
}

#[test]
fn every_event_shape_roundtrips() {
    for event in events() {
        let mut bytes = vec![];
        encode_event(&event, &mut bytes).unwrap();
        let (decoded, consumed) = decode(&bytes).unwrap();
        assert_eq!(consumed, bytes.len());
        assert_eq!(decoded, Frame::Event(event));
    }
}

#[test]
fn refusals_and_leases_roundtrip() {
    let refusals = [
        UiRefusal::UnknownObject(object(1)),
        UiRefusal::StaleGeneration(object(2)),
        UiRefusal::InvalidGeometry("bad wire geometry"),
        UiRefusal::Unsupported("unsupported wire operation"),
    ];
    for why in refusals {
        let mut bytes = vec![];
        encode_refused(CallbackToken(99), &why, &mut bytes).unwrap();
        assert_eq!(
            decode(&bytes).unwrap(),
            (Frame::Refused { token: CallbackToken(99), why: WireRefusal::from_ui(&why) }, bytes.len())
        );
        // The host's text arrives whole (owned), and maps back onto the same kind of UiRefusal engine-side.
        assert_eq!(core::mem::discriminant(&WireRefusal::from_ui(&why).to_ui()), core::mem::discriminant(&why));
    }
    let mut bytes = vec![];
    encode_lease(object(44), 0x8000, 7, &mut bytes).unwrap();
    assert_eq!(
        decode(&bytes).unwrap(),
        (
            Frame::Lease { window: object(44), region: 0x8000, generation: 7 },
            bytes.len()
        )
    );
}

#[test]
fn concatenated_frames_report_their_exact_consumption() {
    let first_call = UiCall::CloseWindow(object(1));
    let second_call = UiCall::SetWindowTitle(object(2), "second".into());
    let mut first = vec![];
    let mut second = vec![];
    encode_call(&first_call, &mut first).unwrap();
    encode_call(&second_call, &mut second).unwrap();
    let first_len = first.len();
    first.extend_from_slice(&second);
    let (frame, consumed) = decode(&first).unwrap();
    assert_eq!(frame, Frame::Call(first_call));
    assert_eq!(consumed, first_len);
    assert_eq!(decode(&first[consumed..]).unwrap().0, Frame::Call(second_call));
}

#[test]
fn every_proper_prefix_is_truncated() {
    let mut bytes = vec![];
    encode_call(&UiCall::OpenWindow(window()), &mut bytes).unwrap();
    for cut in 0..bytes.len() {
        let error = decode(&bytes[..cut]).unwrap_err();
        assert_eq!(error.name(), "Truncated", "prefix length {cut}: {error:?}");
    }
}

#[test]
fn malformed_headers_and_tags_are_refused_by_name() {
    let mut valid = vec![];
    encode_call(&UiCall::CloseWindow(object(1)), &mut valid).unwrap();

    let mut bad = valid.clone();
    bad[0] ^= 0xff;
    assert_eq!(decode(&bad), Err(WireError::BadMagic));

    let mut bad = valid.clone();
    bad[2] = 99;
    assert_eq!(decode(&bad), Err(WireError::BadVersion(99)));

    let mut bad = valid.clone();
    bad[3] = 5;
    assert_eq!(decode(&bad), Err(WireError::UnknownKind(5)));

    let mut bad = valid;
    bad[8] = 0xff;
    assert_eq!(
        decode(&bad),
        Err(WireError::UnknownTag { what: "UiCall", tag: 0xff })
    );

    let mut too_long = vec![0x49, 0x55, 1, 1, 0, 0, 0, 0];
    too_long[4..8].copy_from_slice(&(MAX_FRAME + 1).to_le_bytes());
    assert_eq!(
        decode(&too_long),
        Err(WireError::TooLong { what: "frame", len: MAX_FRAME + 1 })
    );
}

#[test]
fn invalid_utf8_trailing_payload_and_impossible_counts_are_refused() {
    let mut bad_utf8 = vec![];
    encode_call(&UiCall::SetWindowTitle(object(1), "x".into()), &mut bad_utf8).unwrap();
    *bad_utf8.last_mut().unwrap() = 0xff;
    assert_eq!(decode(&bad_utf8), Err(WireError::BadUtf8("string")));

    let mut trailing = vec![];
    encode_call(&UiCall::CloseWindow(object(1)), &mut trailing).unwrap();
    let old_len = u32::from_le_bytes(trailing[4..8].try_into().unwrap());
    trailing[4..8].copy_from_slice(&(old_len + 1).to_le_bytes());
    trailing.push(0);
    assert_eq!(decode(&trailing), Err(WireError::TrailingBytes));

    let mut huge_count = vec![];
    encode_call(
        &UiCall::SetMenuStrip(object(1), MenuModel { menus: vec![] }),
        &mut huge_count,
    ).unwrap();
    // The payload of SetMenuStrip with no menus ENDS with the menus' u32 count (0): prove that, then make it 2^32-1.
    let at = huge_count.len() - 4;
    assert_eq!(&huge_count[at..], &0u32.to_le_bytes(), "the last word is the empty menu list's count");
    huge_count[at..].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(
        decode(&huge_count),
        Err(WireError::Truncated("vector element count"))
    );

    // An encoder never drops a frame silently: a payload over MAX_FRAME is a named refusal and appends nothing.
    let mut out = vec![];
    let big = UiCall::SetWindowTitle(object(1), "x".repeat(MAX_FRAME as usize + 1));
    assert!(matches!(encode_call(&big, &mut out), Err(WireError::TooLong { .. })));
    assert!(out.is_empty());
}
