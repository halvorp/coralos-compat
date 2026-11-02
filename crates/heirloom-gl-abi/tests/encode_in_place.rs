//! `Frame::encode_into` appends a frame to a queue in place (the engine's per-call path): the bytes are exactly the
//! wire layout (`u16 version | u16 kind | u32 payload_len | u32 slot | u32 incarnation | payload`, little-endian), what was
//! already queued is untouched, and every variant decodes back to itself.
use heirloom_gl_abi::command::{Command, DrawCommand, Frame};
use heirloom_gl_abi::snapshot::{Descriptor, ElemType, Role};
use heirloom_gl_abi::{Token, ABI_VERSION};

fn tok() -> Token {
    Token { slot: 7, incarnation: 0x0102_0304 }
}

fn every_variant() -> Vec<Command> {
    let mut m = Vec::new();
    for v in [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0] {
        m.extend_from_slice(&v.to_be_bytes());
    }
    let idx = m.len();
    for i in [0u16, 1, 0] {
        m.extend_from_slice(&i.to_be_bytes());
    }
    let a = [Descriptor { role: Role::Vertex, unit: 0, offset: 0, size: 3, ty: ElemType::F32, stride: 0 }];
    let d = DrawCommand::capture(&m, 4, idx, 3, ElemType::U16, &a).unwrap();
    vec![
        Command::ContextCreate,
        Command::ContextDestroy,
        Command::MakeCurrent,
        Command::Call { entry: 0x1234, args: vec![1, u64::MAX, 0x8077] },
        Command::Call { entry: 3, args: vec![] },
        Command::CallData { entry: 9, args: vec![5, 6], data: vec![1, 2, 3, 4, 5] },
        Command::Draw(d),
        Command::FenceSet { id: 42 },
        Command::Swap { drawable: tok() },
        Command::Refused { text: "[HEIRLOOM-GL REFUSED call=glFoo]".into() },
        Command::Reply { status: 1, words: vec![9, 8], text: "x".into() },
        Command::ReplyData { status: 0, words: vec![2], data: vec![7; 9] },
        Command::BatchEnd { seq: 77 },
        Command::Retired { seq: 78 },
    ]
}

#[test]
fn a_call_frame_is_the_documented_wire_layout() {
    let mut out = Vec::new();
    Frame { token: tok(), command: Command::Call { entry: 0x1234, args: vec![0x1122_3344_5566_7788] } }.encode_into(&mut out);
    let mut want = Vec::new();
    want.extend_from_slice(&ABI_VERSION.to_le_bytes());
    want.extend_from_slice(&4u16.to_le_bytes()); // kind: Call
    want.extend_from_slice(&16u32.to_le_bytes()); // payload: entry, pad, count, one word
    want.extend_from_slice(&7u32.to_le_bytes());
    want.extend_from_slice(&0x0102_0304u32.to_le_bytes());
    want.extend_from_slice(&0x1234u16.to_le_bytes());
    want.extend_from_slice(&0u16.to_le_bytes());
    want.extend_from_slice(&1u32.to_le_bytes());
    want.extend_from_slice(&0x1122_3344_5566_7788u64.to_le_bytes());
    assert_eq!(out, want);
}

#[test]
fn encode_into_appends_after_queued_bytes_and_each_frame_decodes_back() {
    let mut queue = vec![0xAAu8; 5];
    let mut ends = Vec::new();
    for c in every_variant() {
        let f = Frame { token: tok(), command: c };
        let before = queue.clone();
        f.encode_into(&mut queue);
        assert_eq!(&queue[..before.len()], &before[..], "queued bytes changed");
        assert_eq!(&queue[before.len()..], &f.encode()[..], "encode_into and encode differ");
        let (back, used) = Frame::decode(&queue[before.len()..]).unwrap();
        assert_eq!(used, queue.len() - before.len());
        assert_eq!(back, f);
        ends.push(queue.len());
    }
    assert_eq!(&queue[..5], &[0xAA; 5]);
}
