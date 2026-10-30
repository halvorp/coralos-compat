use heirloom_bind::{
    parse, recognise, AnchorRule, BindingSpec, BoxField, CStrField, CountRule, FileForm, FlagTest,
    GroupLayout, IntWidth, ItemLayout, PlacementLayout, Recognised, Refusal, StrListPick, Bindings,
};
use heirloom_ui_abi::{DrawOp, FontRole, GuestPoint, GuestRect, PenRole};

fn sample_spec() -> BindingSpec {
    BindingSpec {
        id: "sample.v1".to_string(),
        file: FileForm {
            form_type: *b" TDX",
            legacy_form_types: vec![*b"TDX0", *b"TDX1"],
        },
        group: GroupLayout {
            chunk: *b"TGRP",
            size: 26,
            title: Some(CStrField { at: 0, max: 16 }),
            window_box: Some(BoxField { at: 18 }),
            inset: 0,
            no_inset_if: None,
        },
        item: ItemLayout {
            chunk: *b"TITM",
            min_size: 22,
            skip_if: Some(FlagTest {
                at: 2,
                width: IntWidth::U16,
                mask: 0x0080,
            }),
            label: StrListPick {
                strings_at: 22,
                count: CountRule::Field {
                    at: 20,
                    width: IntWidth::U8,
                    unless_flag: Some((
                        FlagTest {
                            at: 2,
                            width: IntWidth::U16,
                            mask: 0x0001,
                        },
                        1,
                    )),
                },
                per_record: 2,
                pick: 0,
            },
        },
        placement: Some(PlacementLayout { chunk: *b"TPLC", min_size: 10, window_box: BoxField { at: 0 }, path: CStrField { at: 8, max: 64 } }),
        anchor: AnchorRule::LabelInFill {
            prefix_cut: true,
            strip_lines_inside: true,
            accent_from_swatch: true,
        },
    }
}

fn make_chunk(id: [u8; 4], payload: &[u8], pad_odd: bool) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&id);
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(payload);
    if pad_odd && payload.len() % 2 == 1 {
        out.push(0);
    }
    out
}

fn make_form(form_type: [u8; 4], chunks: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"FORM");
    out.extend_from_slice(&((4 + chunks.len()) as u32).to_be_bytes());
    out.extend_from_slice(&form_type);
    out.extend_from_slice(chunks);
    out
}

fn make_tgrp(title: &str, box_: (i16, i16, i16, i16)) -> Vec<u8> {
    let mut payload = vec![0u8; 26];
    let bytes = title.as_bytes();
    let copy_len = bytes.len().min(15);
    payload[..copy_len].copy_from_slice(&bytes[..copy_len]);
    payload[18..20].copy_from_slice(&box_.0.to_be_bytes());
    payload[20..22].copy_from_slice(&box_.1.to_be_bytes());
    payload[22..24].copy_from_slice(&box_.2.to_be_bytes());
    payload[24..26].copy_from_slice(&box_.3.to_be_bytes());
    make_chunk(*b"TGRP", &payload, true)
}

fn make_titm(flags: u16, count_field: u8, records: &[(&str, &str)], pad_odd: bool) -> Vec<u8> {
    let mut payload = vec![0u8; 22];
    payload[2..4].copy_from_slice(&flags.to_be_bytes());
    payload[20] = count_field;
    for (s0, s1) in records {
        payload.extend_from_slice(s0.as_bytes());
        payload.push(0);
        payload.extend_from_slice(s1.as_bytes());
        payload.push(0);
    }
    make_chunk(*b"TITM", &payload, pad_odd)
}

fn text(x: i32, y: i32, s: &str, width: i32) -> DrawOp {
    DrawOp::Text { at: GuestPoint { x, y }, string: s.to_string(), font_role: FontRole::Ui, px: 10, width, pen_role: PenRole::Text, bg_role: None }
}
fn r(left: i32, top: i32, width: i32, height: i32) -> GuestRect { GuestRect { left, top, width, height } }
fn one_item(name: &str) -> Vec<u8> { make_titm(0x0001, 1, &[(name, "x")], true) }
/// A bound window 0x10 (by box) over a group with the given single-record items.
fn bound(names: &[&str]) -> Bindings {
    let mut b = Bindings::default();
    b.add_spec(sample_spec());
    let mut chunks = make_tgrp("Bar", (0, 0, 400, 100));
    for n in names { chunks.extend_from_slice(&one_item(n)); }
    b.on_file("bank", &make_form(*b" TDX", &chunks));
    b.on_window(0x10, None, (0, 0, 400, 100));
    b
}

#[test]
fn a_file_is_recognised_by_its_form_and_parsed_to_its_groups_labels() {
    let spec = sample_spec();
    assert_eq!(recognise(&spec, b"short"), Recognised::NotThisForm);
    assert_eq!(recognise(&spec, b"NOPE0000 TDX"), Recognised::NotThisForm);
    assert_eq!(recognise(&spec, &make_form(*b"TDX0", &[])), Recognised::Legacy);
    let mut chunks = Vec::new();
    chunks.extend_from_slice(&make_chunk(*b"JUNK", &[1, 2, 3], true)); // an odd chunk with its pad byte
    chunks.extend_from_slice(&make_tgrp("Main Panel", (10, 20, 300, 200)));
    chunks.extend_from_slice(&make_titm(0x0080, 1, &[("Skipped", "cmd")], true)); // skip_if: an image item
    chunks.extend_from_slice(&make_titm(0x0000, 99, &[("Cut", "c"), ("IgnoredExtra", "x")], true)); // flag clear: count = 1, not 99
    chunks.extend_from_slice(&make_titm(0x0001, 2, &[("Copy", "cp"), ("Paste", "p")], false)); // 2 records; no final pad
    let file = make_form(*b" TDX", &chunks);
    assert_eq!(recognise(&spec, &file), Recognised::Ours);
    let groups = parse(&spec, &file).expect("valid parse").groups;
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].title.as_deref(), Some("Main Panel"));
    assert_eq!(groups[0].window_box, Some((10, 20, 300, 200)));
    let labels: Vec<&str> = groups[0].labels.iter().map(|l| l.text.as_str()).collect();
    // Only the FIRST record's pick is a label: "Paste" is Copy's second function, never a second button.
    assert_eq!(labels, vec!["Cut", "Copy"]);
}

#[test]
fn a_malformed_file_is_refused_by_name_never_a_panic() {
    let spec = sample_spec();
    let short_group = make_form(*b" TDX", &make_chunk(*b"TGRP", &[0; 25], true));
    let short_item = { let mut c = make_tgrp("A", (0, 0, 1, 1)); c.extend_from_slice(&make_chunk(*b"TITM", &[0; 21], true)); make_form(*b" TDX", &c) };
    let runaway = { let mut c = make_tgrp("A", (0, 0, 1, 1)); let mut p = vec![0u8; 22]; p[3] = 1; p[20] = 1; p.extend_from_slice(b"NoNul"); c.extend_from_slice(&make_chunk(*b"TITM", &p, true)); make_form(*b" TDX", &c) };
    let past_end = { let mut f = make_form(*b" TDX", &make_tgrp("A", (0, 0, 1, 1))); let n = f.len(); f.truncate(n - 3); f };
    for bad in [short_group, short_item, runaway, past_end] { assert_eq!(parse(&spec, &bad), Err(Refusal::Malformed)); }
    let mut b = Bindings::default();
    b.add_spec(sample_spec());
    b.on_file("x", &make_form(*b" TDX", &make_chunk(*b"TGRP", &[0; 25], true)));
    b.on_file("y", &make_form(*b"TDX1", &[]));
    assert_eq!((b.refusals().get("malformed"), b.refusals().get("legacy schema")), (Some(&1), Some(&1)));
}

#[test]
fn a_label_drawn_in_its_own_fill_becomes_a_face_with_that_fills_colour_and_the_rest_is_untouched() {
    let mut b = bound(&["Copy", "Stop Playback", "Move"]);
    assert!(b.take_log().iter().any(|l| l == "[HEIRLOOM-BINDING sample.v1] bound window 0x10 (3 labels)"));
    let ground = DrawOp::Fill { rect: r(0, 0, 400, 100), pen_role: PenRole::Background };
    let keep_line = DrawOp::Line { a: GuestPoint { x: 10, y: 10 }, b: GuestPoint { x: 200, y: 10 }, pen_role: PenRole::Shadow };
    let mut ops = vec![
        ground.clone(),
        DrawOp::Swatch { rect: r(10, 10, 60, 14), rgb: 0x00C0_3020 },
        DrawOp::Line { a: GuestPoint { x: 10, y: 23 }, b: GuestPoint { x: 69, y: 23 }, pen_role: PenRole::Shadow }, // its hairline
        keep_line.clone(),
        text(14, 12, "Copy", 32),
        DrawOp::Swatch { rect: r(80, 10, 60, 14), rgb: 0x0020_40C0 },
        text(84, 12, "Stop P", 48), // cut to its cell: a unique prefix of "Stop Playback"
        DrawOp::Fill { rect: r(150, 10, 60, 14), pen_role: PenRole::Fill },
        text(154, 12, "Move", 32),
    ];
    b.bind_window(0x10, &mut ops);
    assert!(b.refusals().is_empty(), "{:?}", b.refusals());
    assert_eq!(ops, vec![
        ground,
        DrawOp::Face { rect: r(10, 10, 60, 14), label: "Copy".into(), pressed: false, accent: Some(0x00C0_3020) },
        keep_line,
        DrawOp::Face { rect: r(80, 10, 60, 14), label: "Stop Playback".into(), pressed: false, accent: Some(0x0020_40C0) },
        DrawOp::Face { rect: r(150, 10, 60, 14), label: "Move".into(), pressed: false, accent: None },
    ]);
}

#[test]
fn ambiguity_is_refused_by_name_and_leaves_every_op_as_drawn() {
    let mut b = bound(&["Dup", "Dup", "Twice", "Nowhere", "Make", "Makedir"]);
    let mut ops = vec![
        DrawOp::Swatch { rect: r(0, 0, 100, 40), rgb: 0x00AA_0000 },
        text(5, 5, "Twice", 30), text(50, 5, "Twice", 30), // drawn twice
        text(5, 20, "Dup", 20),                              // twice in the group
        text(150, 60, "Nowhere", 40),                        // no fill under it
        text(60, 20, "Ma", 12),                              // a prefix of two labels
        text(5, 30, "Mystery", 30),                          // names no label: not ours, not counted
    ];
    let before = ops.clone();
    b.bind_window(0x10, &mut ops);
    assert_eq!(ops, before, "no refused run changes anything");
    let want = [("label twice in the group", 1), ("label drawn twice", 1), ("no fill under the label", 1), ("ambiguous prefix", 1)];
    for (why, n) in want { assert_eq!(b.refusals().get(why), Some(&n), "{why}: {:?}", b.refusals()); }
    assert_eq!(b.refusals().len(), 4, "nothing else is counted: {:?}", b.refusals());
    let said = b.take_log();
    b.bind_window(0x10, &mut ops);
    assert!(b.take_log().is_empty(), "each reason is said once: first time {said:?}");
    assert_eq!(b.refusals().get("label drawn twice"), Some(&2), "but counted every time");
}

#[test]
fn a_window_binds_only_by_data_and_ambiguous_windows_bind_neither() {
    let mut b = Bindings::default();
    b.add_spec(sample_spec());
    let mut chunks = make_tgrp("Same", (0, 0, 100, 100));
    chunks.extend_from_slice(&one_item("Copy"));
    chunks.extend_from_slice(&make_tgrp("Same", (10, 10, 100, 100)));
    chunks.extend_from_slice(&one_item("Copy"));
    chunks.extend_from_slice(&make_tgrp("Solo", (50, 50, -1, -1))); // no positive size: never matched by box
    chunks.extend_from_slice(&one_item("Copy"));
    b.on_file("bank", &make_form(*b" TDX", &chunks));
    let face_ops = || vec![DrawOp::Swatch { rect: r(0, 0, 60, 14), rgb: 0x0011_2233 }, text(4, 2, "Copy", 30)];
    b.on_window(1, Some("Same"), (999, 999, 9, 9)); // two groups by title
    b.on_window(2, None, (50, 50, -1, -1));          // a -1 box never anchors
    b.on_window(3, Some("Other"), (1, 1, 1, 1));     // NEGATIVE CONTROL: no data names it
    for w in [1, 2, 3] { let mut ops = face_ops(); b.bind_window(w, &mut ops); assert_eq!(ops, face_ops(), "window {w} stays unbound"); }
    assert_eq!(b.refusals().get("ambiguous window"), Some(&1));
    b.on_window(4, None, (0, 0, 100, 100));          // the first "Same" by its exact box
    b.on_window(5, Some("Solo"), (7, 7, 7, 7));      // "Solo" by its title
    for w in [4, 5] { let mut ops = face_ops(); b.bind_window(w, &mut ops); assert!(matches!(ops[..], [DrawOp::Face { .. }]), "window {w}: {ops:?}"); }
    b.on_window(6, Some("Solo"), (8, 8, 8, 8));      // a second window for a group already anchored: neither binds
    for w in [5, 6] { let mut ops = face_ops(); b.bind_window(w, &mut ops); assert_eq!(ops, face_ops(), "window {w}"); }
    assert_eq!(b.refusals().get("ambiguous window"), Some(&2));
    b.on_close(4);
    let mut ops = face_ops();
    b.bind_window(4, &mut ops);
    assert_eq!(ops, face_ops(), "a closed window's anchor is gone");
}

#[test]
fn a_reread_file_replaces_its_groups_instead_of_making_its_own_window_ambiguous() {
    let mut b = Bindings::default();
    b.add_spec(sample_spec());
    let mut chunks = make_tgrp("Bar", (0, 0, 400, 100));
    chunks.extend_from_slice(&one_item("Copy"));
    let file = make_form(*b" TDX", &chunks);
    b.on_file("bank", &file);
    b.on_file("bank", &file); // the program reads its bank again before opening the window
    b.on_window(0x10, None, (0, 0, 400, 100));
    let mut ops = vec![DrawOp::Swatch { rect: r(0, 0, 60, 14), rgb: 1 }, text(4, 2, "Copy", 30)];
    b.bind_window(0x10, &mut ops);
    assert!(matches!(ops[..], [DrawOp::Face { .. }]), "bound after a re-read: {ops:?}");
    assert!(b.refusals().is_empty(), "{:?}", b.refusals());
}

#[test]
fn an_image_items_name_drawn_as_text_is_not_a_button() {
    let mut b = Bindings::default();
    b.add_spec(sample_spec());
    let mut chunks = make_tgrp("Bar", (0, 0, 400, 100));
    chunks.extend_from_slice(&make_titm(0x0081, 1, &[("Picture", "p")], true)); // skip_if set
    b.on_file("bank", &make_form(*b" TDX", &chunks));
    b.on_window(1, None, (0, 0, 400, 100));
    let want = vec![DrawOp::Swatch { rect: r(0, 0, 60, 14), rgb: 1 }, text(4, 2, "Picture", 40)];
    let mut ops = want.clone();
    b.bind_window(1, &mut ops);
    assert_eq!(ops, want);
}

fn placement(path: &str, box_: (i16, i16, i16, i16)) -> Vec<u8> {
    let mut p = Vec::new();
    for v in [box_.0, box_.1, box_.2, box_.3] { p.extend_from_slice(&v.to_be_bytes()); }
    p.extend_from_slice(path.as_bytes());
    p.push(0);
    make_chunk(*b"TPLC", &p, true)
}

/// A group whose own box is not where its window opens anchors through a placement record (another file of the same form): its
/// path names the group's file and its left/top are the window's. The size is the program's to fit, so it is not compared.
#[test]
fn a_placement_record_anchors_a_group_by_its_file_path_and_the_windows_position() {
    let mut b = Bindings::default();
    b.add_spec(sample_spec());
    let mut chunks = make_tgrp("", (582, 15, 84, 332)); // no title; its own box is not where the window opens
    chunks.extend_from_slice(&one_item("Copy"));
    b.on_file("PROGDIR:Buttons/Bank", &make_form(*b" TDX", &chunks));
    let mut env = placement("progdir:buttons/bank", (712, 55, 84, 332)); // names the file without its case
    env.extend_from_slice(&placement("PROGDIR:Buttons/Other", (100, 100, 50, 50)));
    b.on_file("PROGDIR:Environment/default", &make_form(*b" TDX", &env));
    let face_ops = || vec![DrawOp::Swatch { rect: r(1, 1, 82, 10), rgb: 0x00EF_AE4F }, text(20, 1, "Copy", 30)];
    b.on_window(1, None, (712, 56, 92, 336));  // one pixel lower: not the record's position
    b.on_window(2, None, (100, 100, 50, 50));  // NEGATIVE CONTROL: a record for another file
    for w in [1, 2] { let mut ops = face_ops(); b.bind_window(w, &mut ops); assert_eq!(ops, face_ops(), "window {w} stays unbound"); }
    b.on_window(3, None, (712, 55, 92, 336));  // the record's position, the program's own size
    let mut ops = face_ops();
    b.bind_window(3, &mut ops);
    assert_eq!(ops, vec![DrawOp::Face { rect: r(1, 1, 82, 10), label: "Copy".into(), pressed: false, accent: Some(0x00EF_AE4F) }]);
    assert!(b.refusals().is_empty(), "{:?}", b.refusals());
}

/// The format's border ring: with an inset the face covers the whole cell (fill grown by the inset) and the program's own edge
/// lines on that ring go; a group whose flag says borderless keeps the fill as the face.
#[test]
fn the_declared_inset_makes_the_face_the_whole_cell_unless_the_group_is_borderless() {
    let mut spec = sample_spec();
    spec.group.inset = 1;
    spec.group.no_inset_if = Some(FlagTest { at: 16, width: IntWidth::U8, mask: 1 }); // title byte 16 is past the 15-byte title
    let ring = |y| DrawOp::Line { a: GuestPoint { x: 0, y }, b: GuestPoint { x: 83, y }, pen_role: PenRole::Shadow };
    let ops = || vec![ring(88), DrawOp::Swatch { rect: r(1, 89, 82, 10), rgb: 0x00EF_AE4F }, ring(99), text(20, 89, "Run", 20)];
    for (borderless, want) in [(0u8, r(0, 88, 84, 12)), (1u8, r(1, 89, 82, 10))] {
        let mut b = Bindings::default();
        b.add_spec(spec.clone());
        let mut group = make_tgrp("Bank", (0, 0, 84, 332));
        group[8 + 16] = borderless;
        group.extend_from_slice(&one_item("Run"));
        b.on_file("bank", &make_form(*b" TDX", &group));
        b.on_window(1, Some("Bank"), (5, 5, 5, 5));
        let mut got = ops();
        b.bind_window(1, &mut got);
        let face = DrawOp::Face { rect: want, label: "Run".into(), pressed: false, accent: Some(0x00EF_AE4F) };
        if borderless == 0 { assert_eq!(got, vec![face], "the ring lines are the face's own edges"); }
        else { assert_eq!(got, vec![ring(88), face, ring(99)], "borderless: the lines outside the fill are not the face's"); }
    }
}
