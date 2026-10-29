//! The machine-session frames (Control / Report) ride the ONE wire: every variant round-trips, every truncation and unknown tag is
//! refused by name, a chunk past its blob is refused, a UI host never takes a session frame as a UI call, and a host driver that
//! forwards its UI's events one by one sends engine ids.

use heirloom_ui_abi::wire::host::WireHost;
use heirloom_ui_abi::wire::{decode, encode_control, encode_report, Control, Frame, Report, WireError, WireStep};
use heirloom_ui_abi::*;

fn controls() -> Vec<Control> {
    vec![
        Control::Machine { profile: "mac-plus".into() },
        Control::Canvases { slot: 7, len: 8 << 20 },
        Control::Volume { offset: 0, total: 3, chunk: vec![1, 2, 3] },
        Control::Program { offset: 4, total: 6, chunk: vec![9, 9] },
        Control::StartFile { name: "Glider PRO".into() },
        Control::SetWallClock { local_unix_secs: -1 },
        Control::StopOnUnimplemented(true),
        Control::Advance { virtual_us: 1 << 40 },
        Control::Step { budget: 200_000 },
    ]
}

fn reports() -> Vec<Report> {
    vec![
        Report::Started(Ok(())),
        Report::Started(Err("not a Mac application".into())),
        Report::WallClock(Err("before 1904".into())),
        Report::Stepped { step: WireStep::Blocked, insns: 12, bus_faults: 0, deadline_us: Some(16_667), refused: vec![], undelivered: vec![] },
        Report::Stepped {
            step: WireStep::Fault("bus error at $0".into()),
            insns: 0,
            bus_faults: 3,
            deadline_us: None,
            refused: vec!["_Unimplemented".into()],
            undelivered: vec!["NotForGuestWindow".into()],
        },
        Report::Stepped { step: WireStep::Exited, insns: 1, bus_faults: 0, deadline_us: None, refused: vec![], undelivered: vec![] },
        Report::Stepped { step: WireStep::BudgetSpent, insns: 200_000, bus_faults: 0, deadline_us: None, refused: vec![], undelivered: vec![] },
    ]
}

fn frames() -> Vec<(Vec<u8>, Frame)> {
    let mut out = Vec::new();
    for c in controls() {
        let mut b = Vec::new();
        encode_control(&c, &mut b).unwrap();
        out.push((b, Frame::Control(c)));
    }
    for r in reports() {
        let mut b = Vec::new();
        encode_report(&r, &mut b).unwrap();
        out.push((b, Frame::Report(r)));
    }
    out
}

#[test]
fn every_session_frame_round_trips() {
    for (bytes, want) in frames() {
        assert_eq!(decode(&bytes), Ok((want, bytes.len())));
    }
}

#[test]
fn frames_concatenate_into_one_batch() {
    let all: Vec<(Vec<u8>, Frame)> = frames();
    let batch: Vec<u8> = all.iter().flat_map(|(b, _)| b.clone()).collect();
    let mut at = 0;
    for (_, want) in all {
        let (got, used) = decode(&batch[at..]).unwrap();
        assert_eq!(got, want);
        at += used;
    }
    assert_eq!(at, batch.len());
}

#[test]
fn every_truncation_and_unknown_tag_is_refused_by_name() {
    for (bytes, _) in frames() {
        for cut in 0..bytes.len() {
            assert!(decode(&bytes[..cut]).is_err(), "{cut} of {}", bytes.len());
        }
    }
    let mut b = Vec::new();
    encode_control(&Control::Step { budget: 1 }, &mut b).unwrap();
    b[8] = 0xEE; // the Control tag
    assert_eq!(decode(&b), Err(WireError::UnknownTag { what: "Control", tag: 0xEE }));
    let mut b = Vec::new();
    encode_report(&Report::Started(Ok(())), &mut b).unwrap();
    b[8] = 0x55;
    assert_eq!(decode(&b), Err(WireError::UnknownTag { what: "Report", tag: 0x55 }));
}

#[test]
fn a_chunk_reaching_past_its_blob_is_refused() {
    let mut b = Vec::new();
    encode_control(&Control::Program { offset: 5, total: 6, chunk: vec![0, 0] }, &mut b).unwrap();
    assert!(matches!(decode(&b), Err(WireError::TooLong { what: "blob chunk past its total", .. })));
}

#[test]
fn a_ui_host_refuses_session_frames_as_calls() {
    let mut host = WireHost::new(RecordingGuestUi::new());
    for (bytes, frame) in frames() {
        let kind = if matches!(frame, Frame::Control(_)) { 6 } else { 7 };
        assert_eq!(host.apply(&bytes), Err(WireError::UnknownKind(kind)));
    }
    assert!(host.ui().calls.is_empty());
}

#[test]
fn a_forwarded_host_event_carries_the_engine_window_id() {
    let mut host = WireHost::new(RecordingGuestUi::new());
    let model = WindowModel {
        title: "w".into(),
        rect: GuestRect { left: 0, top: 0, width: 10, height: 10 },
        role: WindowRole::Document,
        closable: true,
        resizable: false,
        token: CallbackToken(1),
    };
    // Two host windows exist before the engine's first: host ids and engine ids differ.
    host.ui_mut().open_window(model.clone()).unwrap();
    host.ui_mut().open_window(model.clone()).unwrap();
    let mut call = Vec::new();
    heirloom_ui_abi::wire::encode_call(&UiCall::OpenWindow(model), &mut call).unwrap();
    host.apply(&call).unwrap();
    let host_win = UiObjectRef { id: 3, generation: 1 };
    let frame = host.encode_host_event(GuestUiEvent::CloseRequested { win: host_win, token: CallbackToken(1) });
    let (got, _) = decode(&frame).unwrap();
    assert_eq!(got, Frame::Event(GuestUiEvent::CloseRequested { win: UiObjectRef { id: 1, generation: 1 }, token: CallbackToken(1) }));
}

mod canvas_arena {
    use heirloom_ui_abi::wire::canvas::*;

    #[test]
    fn a_canvas_written_at_an_offset_reads_back_through_its_region() {
        let mut arena = vec![0u8; 4096];
        let px: Vec<u32> = (0..12).map(|i| 0x00AB_CD00 | i).collect();
        let region = write_canvas(&mut arena, 32, 4, 3, &px).unwrap();
        assert_eq!(region, 3);
        assert_eq!(offset_of(region), Some(32));
        assert_eq!(read_canvas(&arena, region), Ok((4, 3, px)));
        assert_eq!(canvas_bytes(12), Some(64));
        // A present's damage is part of its canvas: fewer pixels than the whole is the normal case.
        let r = write_canvas(&mut arena, 128, 10, 10, &[1, 2]).unwrap();
        assert_eq!(read_canvas(&arena, r), Ok((10, 10, vec![1, 2])));
    }

    #[test]
    fn a_bad_region_shape_or_bound_is_refused_by_name() {
        let mut arena = vec![0u8; 256];
        assert_eq!(write_canvas(&mut arena, 8, 1, 1, &[0]), Err(CanvasError::BadRegion));
        assert_eq!(write_canvas(&mut arena, 0, 2, 2, &[0; 5]), Err(CanvasError::BadShape));
        assert_eq!(write_canvas(&mut arena, 0, -1, 2, &[]), Err(CanvasError::BadShape));
        assert_eq!(write_canvas(&mut arena, 240, 4, 4, &[0; 16]), Err(CanvasError::OutOfArena));
        assert_eq!(read_canvas(&arena, 0), Err(CanvasError::BadRegion));
        // A header whose count disagrees with its shape (an engine that lies) is refused, never trusted for the read length.
        write_canvas(&mut arena, 0, 2, 2, &[1, 2, 3, 4]).unwrap();
        arena[8] = 200;
        assert_eq!(read_canvas(&arena, 1), Err(CanvasError::BadShape));
        // A shape that runs past the arena's end.
        arena[0..4].copy_from_slice(&100u32.to_le_bytes());
        arena[8..12].copy_from_slice(&200u32.to_le_bytes());
        assert_eq!(read_canvas(&arena, 1), Err(CanvasError::OutOfArena));
    }
}
