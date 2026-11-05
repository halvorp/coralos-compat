//! RESIDENT ARRAYS (wire v4): a draw's array may travel as bytes the worker keeps under an id (`Keep::Store`) and later as the id
//! alone (`Keep::Ref`); the worker's `Resident` store puts the bytes back, so what it replays is byte for byte the plain draw.
use heirloom_gl_abi::command::{Command, DrawCommand, Frame};
use heirloom_gl_abi::snapshot::{Descriptor, ElemType, Keep, Resident, Role};
use heirloom_gl_abi::{Token, WireError};

const T: Token = Token { slot: 1, incarnation: 1 };

/// Guest RAM with big-endian u16 indices 0..4 at 0 and two arrays: 4 float xyz vertices at 0x100, 4 RGBA colours at 0x200.
fn guest() -> (Vec<u8>, Vec<Descriptor>) {
    let mut m = vec![0u8; 0x400];
    for i in 0..4u16 {
        m[2 * i as usize..2 * i as usize + 2].copy_from_slice(&i.to_be_bytes());
    }
    for k in 0..12 {
        m[0x100 + 4 * k..0x104 + 4 * k].copy_from_slice(&(k as f32).to_be_bytes());
    }
    for k in 0..16 {
        m[0x200 + k] = k as u8 * 7;
    }
    let d = vec![
        Descriptor { role: Role::Vertex, unit: 0, offset: 0x100, size: 3, ty: ElemType::F32, stride: 0 },
        Descriptor { role: Role::Color, unit: 0, offset: 0x200, size: 4, ty: ElemType::U8, stride: 0 },
    ];
    (m, d)
}

fn draw(bytes: &[u8]) -> DrawCommand {
    match Frame::decode(bytes).unwrap().0.command {
        Command::Draw(d) => d,
        other => panic!("{other:?}"),
    }
}

fn encode(m: &[u8], d: &[Descriptor], keep: [Keep; 2]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut spans = Vec::new();
    Frame::encode_draw_elements_kept_into(&mut out, T, m, 4, 0, 4, ElemType::U16, d, &mut |k, span| { spans.push(span); keep[k] }).unwrap();
    assert_eq!(spans, vec![0x100..0x130, 0x200..0x210], "each array is asked once, with the guest bytes the draw references");
    out
}

#[test]
fn a_kept_array_named_by_its_id_replays_the_bytes_it_was_stored_with() {
    let (m, d) = guest();
    let mut plain = Vec::new();
    Frame::encode_draw_elements_into(&mut plain, T, &m, 4, 0, 4, ElemType::U16, &d).unwrap();
    let want = draw(&plain);
    let mut w = Resident::default();
    // stored: the bytes travel, the worker keeps the vertices under id 7
    let stored = encode(&m, &d, [Keep::Store(7), Keep::Copy]);
    assert_eq!(stored.len(), plain.len() + 4, "a stored array costs its id");
    let mut got = draw(&stored);
    w.resolve(&mut got.arrays).unwrap();
    assert_eq!(got.arrays[0].data, want.arrays[0].data);
    // named: the 48 vertex bytes do not travel, and the worker puts them back
    let named = encode(&m, &d, [Keep::Ref(7), Keep::Copy]);
    assert_eq!(named.len(), plain.len() + 4 - 48, "a named array carries no bytes");
    let mut got = draw(&named);
    assert!(got.arrays[0].data.is_empty() && got.arrays[0].keep == Keep::Ref(7));
    w.resolve(&mut got.arrays).unwrap();
    assert_eq!(got.arrays.iter().map(|a| &a.data).collect::<Vec<_>>(), want.arrays.iter().map(|a| &a.data).collect::<Vec<_>>());
    assert_eq!((w.refs, w.ref_bytes, w.stores), (1, 48, 1));
    // the frame encoder agrees with the direct one for every Keep (one wire format, two writers)
    assert_eq!(Frame { token: T, command: Command::Draw(draw(&named)) }.encode(), named);
    assert_eq!(Frame { token: T, command: Command::Draw(draw(&stored)) }.encode(), stored);
}

#[test]
fn a_new_store_under_an_id_replaces_what_it_named() {
    let (mut m, d) = guest();
    let mut w = Resident::default();
    w.resolve(&mut draw(&encode(&m, &d, [Keep::Store(0), Keep::Copy])).arrays).unwrap();
    m[0x100..0x104].copy_from_slice(&99f32.to_be_bytes());
    w.resolve(&mut draw(&encode(&m, &d, [Keep::Store(0), Keep::Copy])).arrays).unwrap();
    let mut got = draw(&encode(&m, &d, [Keep::Ref(0), Keep::Copy]));
    w.resolve(&mut got.arrays).unwrap();
    assert_eq!(got.arrays[0].data[..4], 99f32.to_le_bytes(), "the id names the LAST bytes stored under it");
}

#[test]
fn a_reference_the_worker_cannot_back_is_refused() {
    let (m, d) = guest();
    let mut w = Resident::default();
    assert_eq!(w.resolve(&mut draw(&encode(&m, &d, [Keep::Ref(3), Keep::Copy])).arrays), Err(WireError::BadValue("resident array id never stored")));
    // kept as the 4 colours (16 bytes), named as the 4 vertices (48): the lengths disagree
    w.resolve(&mut draw(&encode(&m, &d, [Keep::Copy, Keep::Store(3)])).arrays).unwrap();
    assert_eq!(w.resolve(&mut draw(&encode(&m, &d, [Keep::Ref(3), Keep::Copy])).arrays), Err(WireError::LengthMismatch));
    // NEGATIVE CONTROL: an unknown keep tag is a decode error, never read as copied
    let mut bad = encode(&m, &d, [Keep::Copy, Keep::Copy]);
    let rec = 16 + 16 + 8; // the header, the draw's four fields, the 4 u16 indices: the first array's record
    assert_eq!(bad[rec + 3], 0);
    bad[rec + 3] = 3;
    assert_eq!(Frame::decode(&bad).unwrap_err(), WireError::BadValue("array keep"));
}
