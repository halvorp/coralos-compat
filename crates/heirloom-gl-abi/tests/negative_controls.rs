//! Phase 0 negative controls of docs/heirloom/22-GUEST-GL.md. Each test names the guarantee it pins; `mutants.sh`
//! breaks the implementation one way per guarantee and requires the named test to go RED.
use heirloom_gl_abi::command::{Command, DrawCommand, Frame};
use heirloom_gl_abi::contract::{Entry, ENTRIES};
use heirloom_gl_abi::snapshot::{snapshot_array, Descriptor, ElemType, Role, SnapshotError};
use heirloom_gl_abi::{admit_call, derive, extensions_string, find, refusal_marker, version_string, Refusal, Token, TokenError, TokenTable, Version, WireError};

/// Guest memory: 3 vertices of 3 big-endian f32, 3 RGBA bytes, 3 big-endian u16 indices.
fn guest() -> (Vec<u8>, Vec<Descriptor>, usize) {
    let mut m = Vec::new();
    for v in [[-0.5f32, -0.5, 0.0], [0.5, -0.5, 0.0], [0.0, 0.5, 0.0]] {
        for c in v {
            m.extend_from_slice(&c.to_be_bytes());
        }
    }
    let col_off = m.len();
    m.extend_from_slice(&[255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255]);
    let idx_off = m.len();
    for i in [0u16, 1, 2] {
        m.extend_from_slice(&i.to_be_bytes());
    }
    let arrays = vec![
        Descriptor { role: Role::Vertex, unit: 0, offset: 0, size: 3, ty: ElemType::F32, stride: 0 },
        Descriptor { role: Role::Color, unit: 0, offset: col_off, size: 4, ty: ElemType::U8, stride: 0 },
    ];
    (m, arrays, idx_off)
}

fn draw(m: &[u8], a: &[Descriptor], idx: usize) -> DrawCommand {
    DrawCommand::capture(m, 4, idx, 3, ElemType::U16, a).expect("capture")
}

// ---- control 1: array snapshot vs mutation after the draw ----------------------------------------------------------
#[test]
fn nc_snapshot_is_stable_against_mutation_after_draw() {
    let (mut m, a, idx) = guest();
    let d = draw(&m, &a, idx);
    let before = Frame { token: Token { slot: 1, incarnation: 1 }, command: Command::Draw(d.clone()) }.encode();
    for b in m.iter_mut() {
        *b = 0xEE; // the guest scribbles over every array after the draw
    }
    let after = Frame { token: Token { slot: 1, incarnation: 1 }, command: Command::Draw(d.clone()) }.encode();
    assert_eq!(before, after);
    assert_eq!(d.arrays[0].data[..4], (-0.5f32).to_le_bytes(), "the captured bytes are the pre-mutation values");
}

/// The checker itself must catch a lazy implementation: one that keeps descriptors and reads memory at encode time.
#[test]
fn nc_the_stability_check_catches_a_planted_lazy_encoder() {
    let (mut m, a, idx) = guest();
    let lazy = |mem: &[u8]| -> Vec<u8> {
        // PLANTED BUG: reads guest memory when encoding, not when drawing.
        snapshot_array(mem, &a[0], 0, 3).expect("lazy read").data
    };
    let _ = idx;
    let before = lazy(&m);
    m.iter_mut().for_each(|b| *b = 0xEE);
    assert_ne!(before, lazy(&m), "a lazy encoder changes after mutation, so nc_snapshot_is_stable... would be RED for it");
}

// ---- control 2: wrong-endian indices and components ------------------------------------------------------------------
#[test]
fn nc_indices_and_floats_are_converted_to_little_endian() {
    let (m, a, idx) = guest();
    let d = draw(&m, &a, idx);
    assert_eq!(d.indices.data, [0, 0, 1, 0, 2, 0], "u16 indices 0,1,2 as little-endian bytes");
    assert_eq!(d.indices.range, Some((0, 2)), "range from the CONVERTED values");
    assert_eq!(&d.arrays[0].data[12..16], &0.5f32.to_le_bytes());
    assert_eq!(d.arrays[1].data[..4], [255, 0, 0, 255], "bytes are never swapped");
}

#[test]
fn nc_index_range_comes_from_converted_values_not_raw_bytes() {
    // BE u16 0x0100 is 256; read raw-LE it would be 1. The array snapshot must cover 0..=256.
    let mut m = vec![0u8; 4096];
    m[3000..3002].copy_from_slice(&0x0100u16.to_be_bytes());
    let a = [Descriptor { role: Role::Vertex, unit: 0, offset: 0, size: 1, ty: ElemType::U8, stride: 0 }];
    let d = DrawCommand::capture(&m, 0, 3000, 1, ElemType::U16, &a).unwrap();
    assert_eq!(d.indices.range, Some((256, 256)));
    assert_eq!((d.arrays[0].first, d.arrays[0].count), (256, 1));
}

// ---- control 3: stride / count overflow ------------------------------------------------------------------------------
#[test]
fn nc_offset_and_stride_overflow_is_an_error_not_a_wrap() {
    let m = vec![0u8; 64];
    let d = Descriptor { role: Role::Vertex, unit: 0, offset: usize::MAX - 3, size: 3, ty: ElemType::F32, stride: 12 };
    assert_eq!(snapshot_array(&m, &d, 0, 2).unwrap_err(), SnapshotError::Overflow);
    let d2 = Descriptor { offset: 0, stride: u32::MAX, ..d };
    assert!(matches!(snapshot_array(&m, &d2, u32::MAX, 3), Err(SnapshotError::Overflow | SnapshotError::OutOfRange)));
}

#[test]
fn nc_a_huge_count_is_refused_before_anything_is_allocated() {
    let m = vec![0u8; 64];
    let d = Descriptor { role: Role::Vertex, unit: 0, offset: 0, size: 4, ty: ElemType::F32, stride: 0 };
    assert_eq!(snapshot_array(&m, &d, 0, u32::MAX).unwrap_err(), SnapshotError::OutOfRange);
    assert_eq!(DrawCommand::capture(&m, 4, 0, u32::MAX, ElemType::U32, &[d]).unwrap_err(), SnapshotError::OutOfRange);
}

// ---- control 4: stale context token ----------------------------------------------------------------------------------
#[test]
fn nc_a_token_from_an_earlier_incarnation_is_refused() {
    let mut t = TokenTable::default();
    let old = t.create().unwrap();
    assert_eq!(t.check(old), Ok(()));
    t.destroy(old).unwrap();
    assert_eq!(t.check(old), Err(TokenError::Stale), "destroyed");
    let new = t.create().unwrap();
    assert_eq!(new.slot, old.slot, "the slot is reused");
    assert_ne!(new.incarnation, old.incarnation);
    assert_eq!(t.check(old), Err(TokenError::Stale), "the old token must not reach the new context");
    assert_eq!(t.check(new), Ok(()));
    assert_eq!(t.check(Token { slot: 99, incarnation: 1 }), Err(TokenError::Unknown));
}

#[test]
fn tokens_grow_without_a_fixed_cap() {
    let mut t = TokenTable::default();
    let v: Vec<Token> = (0..100_000).map(|_| t.create().unwrap()).collect();
    assert!(v.iter().all(|k| t.check(*k).is_ok()));
}

// ---- control 5: an unsupported call is refused by NAME, never skipped ------------------------------------------------
#[test]
fn nc_unsupported_call_is_refused_by_name() {
    // glTexImage3D is core GL 1.2, listed in the contract, and refused by name (gl4es treats a 3D texture as a 2D one).
    let i = find(ENTRIES, "glTexImage3D").unwrap() as u16;
    let r = admit_call(ENTRIES, i).unwrap_err();
    assert_eq!(r, Refusal::Unsupported { name: "glTexImage3D".into() });
    assert_eq!(refusal_marker(&r), "[HEIRLOOM-GL REFUSED call=glTexImage3D]");
    let mut on: Vec<Entry> = ENTRIES.to_vec();
    on[i as usize].implemented = true;
    assert_eq!(admit_call(&on, i).unwrap().name, "glTexImage3D");
    assert!(admit_call(ENTRIES, find(ENTRIES, "glDrawElements").unwrap() as u16).is_ok(), "an implemented row passes");
    assert_eq!(admit_call(&on, u16::MAX).unwrap_err(), Refusal::UnknownEntry { index: u16::MAX });
}

#[test]
fn no_gpu_accel_has_its_own_marker() {
    assert_eq!(refusal_marker(&Refusal::NoGpuAccel), "[HEIRLOOM-GL REFUSED reason=no-gpu-accel]");
}

// ---- control 6: the version string never exceeds the implemented set -------------------------------------------------
#[test]
fn nc_with_nothing_implemented_no_version_and_no_extension_is_claimed() {
    let none: Vec<Entry> = ENTRIES.iter().map(|x| Entry { implemented: false, ..*x }).collect();
    let c = derive(&none, Some(Version::V1_2));
    assert_eq!(version_string(&c), Err(Refusal::NoClaimableVersion));
    assert_eq!(extensions_string(&c), "");
}

#[test]
fn nc_claims_never_exceed_the_implemented_set_for_any_flag_pattern() {
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for round in 0..4000 {
        let density = 1 + (round % 7) as u64; // sparse to dense
        let mut e: Vec<Entry> = ENTRIES.to_vec();
        for x in e.iter_mut() {
            x.implemented = next() % 8 < density + 1;
            x.gap = if next() % 16 == 0 { Some("a random gap") } else { None };
        }
        let c = derive(&e, Some(Version::V1_2));
        if let Some(v) = c.version {
            for x in &e {
                if let heirloom_gl_abi::Home::Core(w) = x.home {
                    assert!(w > v || x.complete(), "claims {v:?} but {} (core {w:?}) is not implemented", x.name);
                }
            }
        }
        for ext in &c.extensions {
            assert!(e.iter().filter(|x| x.home == heirloom_gl_abi::Home::Ext(*ext)).all(|x| x.complete()), "claims {ext:?} with a missing entry");
        }
        assert_eq!(extensions_string(&c).split_whitespace().count(), c.extensions.len());
    }
}

#[test]
fn nc_one_missing_entry_withdraws_its_extension_and_the_version() {
    let mut e: Vec<Entry> = ENTRIES.iter().map(|x| Entry { implemented: true, gap: None, ..*x }).collect();
    assert_eq!(derive(&e, Some(Version::V1_2)).version, Some(Version::V1_2));
    let i = find(&e, "glTexImage3D").unwrap();
    e[i].implemented = false; // a GL 1.2 entry
    assert_eq!(derive(&e, Some(Version::V1_2)).version, Some(Version::V1_1));
    let j = find(&e, "glTestFenceAPPLE").unwrap();
    e[j].implemented = false;
    assert!(!extensions_string(&derive(&e, Some(Version::V1_2))).contains("GL_APPLE_fence"));
}

// ---- the ring: version-first, little-endian, robust ------------------------------------------------------------------
#[test]
fn nc_frames_are_version_first_and_little_endian() {
    let b = Frame { token: Token { slot: 0x0102_0304, incarnation: 5 }, command: Command::FenceSet { id: 9 } }.encode();
    assert_eq!(b[..2], [2, 0], "ABI version 2, little-endian, FIRST");
    assert_eq!(b[8..12], [4, 3, 2, 1]);
    let mut bad = b.clone();
    bad[0] = 3;
    assert_eq!(Frame::decode(&bad).unwrap_err(), WireError::Version { got: 3 });
    bad[0] = 1;
    assert_eq!(Frame::decode(&bad).unwrap_err(), WireError::Version { got: 1 }, "a version-1 peer is refused, not misread");
    assert_eq!(Frame::decode(&b[..1]).unwrap_err(), WireError::Truncated);
}

#[test]
fn every_command_round_trips() {
    let (m, a, idx) = guest();
    let t = Token { slot: 3, incarnation: 7 };
    for c in [
        Command::ContextCreate,
        Command::ContextDestroy,
        Command::MakeCurrent,
        Command::Call { entry: 12, args: vec![1, u64::MAX, 0] },
        Command::CallData { entry: 13, args: vec![3, 4], data: vec![9, 8, 7] },
        Command::CallData { entry: 13, args: vec![], data: vec![] },
        Command::ReplyData { status: 0, words: vec![1], data: vec![0, 255] },
        Command::Draw(draw(&m, &a, idx)),
        Command::FenceSet { id: 4 },
        Command::Swap { drawable: Token { slot: 1, incarnation: 2 } },
        Command::Refused { text: "[HEIRLOOM-GL REFUSED call=glFoo]".into() },
        Command::Reply { status: 0, words: vec![3, 1], text: "Mock GPU".into() },
        Command::Reply { status: 1, words: vec![], text: String::new() },
        Command::BatchEnd { seq: u64::MAX },
        Command::Retired { seq: 7 },
    ] {
        let f = Frame { token: t, command: c };
        let b = f.encode();
        assert_eq!(Frame::decode(&b).unwrap(), (f, b.len()));
    }
}

#[test]
fn a_truncated_or_lying_frame_is_an_error_never_a_panic() {
    let (m, a, idx) = guest();
    let b = Frame { token: Token { slot: 1, incarnation: 1 }, command: Command::Draw(draw(&m, &a, idx)) }.encode();
    for n in 0..b.len() {
        assert!(Frame::decode(&b[..n]).is_err(), "prefix {n} decoded");
    }
    // A Call frame claiming u32::MAX args with no bytes behind it must not allocate for them.
    let mut call = Frame { token: Token { slot: 1, incarnation: 1 }, command: Command::Call { entry: 1, args: vec![] } }.encode();
    call[20..24].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(Frame::decode(&call).unwrap_err(), WireError::LengthMismatch);
}

#[test]
fn a_streaming_reader_learns_each_frames_length_from_its_header_alone() {
    use heirloom_gl_abi::command::peek_len;
    let b = Frame { token: Token { slot: 1, incarnation: 1 }, command: Command::Reply { status: 0, words: vec![1, 2, 3], text: "x".into() } }.encode();
    for n in 0..16 {
        assert_eq!(peek_len(&b[..n]), if n >= 2 { Ok(None) } else { Ok(None) }, "header incomplete at {n}");
    }
    assert_eq!(peek_len(&b[..16]), Ok(Some(b.len())));
    assert_eq!(peek_len(&[3, 0]), Err(WireError::Version { got: 3 }), "an unknown ABI is refused from its first two bytes");
}

#[test]
fn a_reply_that_claims_more_words_than_it_carries_is_refused_without_allocating_them() {
    let mut b = Frame { token: Token { slot: 0, incarnation: 0 }, command: Command::Reply { status: 0, words: vec![], text: String::new() } }.encode();
    b[20..24].copy_from_slice(&u32::MAX.to_le_bytes()); // nwords
    assert_eq!(Frame::decode(&b).unwrap_err(), WireError::LengthMismatch);
}

#[test]
fn a_calldata_or_replydata_that_claims_more_words_than_it_carries_is_refused_without_allocating_them() {
    for c in [Command::CallData { entry: 1, args: vec![], data: vec![1, 2, 3] }, Command::ReplyData { status: 0, words: vec![], data: vec![1, 2, 3] }] {
        let mut b = Frame { token: Token { slot: 0, incarnation: 0 }, command: c }.encode();
        b[20..24].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(Frame::decode(&b).unwrap_err(), WireError::LengthMismatch);
    }
}
