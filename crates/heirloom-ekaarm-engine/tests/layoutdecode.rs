//! The byte-code decoder and the layout arithmetic on vectors made here (no platform data): the encodings the decoder reads are
//! 0xff word, 0xfe empty, 0xfd parent-relative (signed byte), 0xfc parent-relative (two bytes), 0xfb long, below that a literal.
use heirloom_ekaarm_engine::layoutdecode::{Instance, LAYOUT_EMPTY};
use heirloom_ekaarm_engine::layoutmath::{compose, layout_line, layout_rect, Rect};

const EMPTY: u8 = 0xfe;
const P0: [u8; 2] = [0xfd, 0x00];

fn inst<'a>(blob: &'a [u8], lookup: &'a [u16]) -> Instance<'a> {
    Instance { blob, base: 0, lookup, mirrored: false }
}

fn word(v: u16) -> [u8; 3] {
    [0xff, (v >> 8) as u8, v as u8]
}

#[test]
fn a_simple_window_line_decodes_to_its_cells() {
    // type 3 (pane), no header block: C=empty l=10 t=20 r=empty b=empty W=parent H=300
    let mut blob = vec![3 << 1, EMPTY, 10, 20, EMPTY, EMPTY];
    blob.extend(P0);
    blob.extend(word(300));
    let w = inst(&blob, &[0]).line(0, None, false, 0, 0, 0).unwrap();
    assert_eq!(w[5], 3, "the component type");
    assert_eq!(w[6], LAYOUT_EMPTY);
    assert_eq!((w[7], w[8]), (10, 20));
    assert_eq!((w[9], w[10]), (LAYOUT_EMPTY, LAYOUT_EMPTY));
    assert_eq!(w[11], 0x4000, "parent-relative zero is the parent's size");
    assert_eq!(w[12], 300);
}

#[test]
fn a_parameterised_line_checks_its_variety_and_a_mirrored_instance_swaps_l_and_r() {
    // type 3 with a header block: 2 varieties, 0 multi-values, one column, one row, then C l t r b W H (one value each)
    let mut blob = vec![(3 << 1) | 1, 2, 0, 1, 1, EMPTY, 7, 9, 11, EMPTY, 40, 50];
    let w = inst(&blob, &[0]).line(0, None, false, 1, 0, 0).unwrap();
    assert_eq!((w[7], w[8], w[9], w[11], w[12]), (7, 9, 11, 40, 50));
    let err = inst(&blob, &[0]).line(0, None, false, 2, 0, 0).unwrap_err();
    assert!(err.contains("variety"), "{err}");
    let m = Instance { mirrored: true, ..inst(&blob, &[0]) }.line(0, None, false, 0, 0, 0).unwrap();
    assert_eq!((m[7], m[9]), (11, 7), "l and r unswapped");
    // limits: 2 varieties, one column, one row
    assert_eq!(inst(&blob, &[0]).limits(0, None, 0).unwrap(), [0, 1, 0, 0, 0, 0]);
    blob.truncate(4);
    assert!(inst(&blob, &[0]).line(0, None, false, 0, 0, 0).is_err(), "a truncated blob is refused, not read past");
}

#[test]
fn a_table_line_is_found_through_its_offsets() {
    // at offset 0: count 2, then two big-endian offsets; the lines follow
    let mut blob = vec![2, 0, 6, 0, 16];
    blob.resize(6, 0);
    let line_a = [3u8 << 1, EMPTY, 1, 2, EMPTY, EMPTY, 5, 6];
    let line_b = [3u8 << 1, EMPTY, 3, 4, EMPTY, EMPTY, 7, 8];
    blob.extend(line_a);
    blob.resize(16, 0);
    blob.extend(line_b);
    let i = inst(&blob, &[0]);
    assert_eq!((i.line(0, Some(0), false, 0, 0, 0).unwrap()[7], i.line(0, Some(1), false, 0, 0, 0).unwrap()[7]), (1, 3));
    assert!(i.line(0, Some(2), false, 0, 0, 0).is_err());
    assert_eq!(i.table_limits(0).unwrap(), [0, 0]);
}

fn line(l: i16, t: i16, r: i16, b: i16, w: i16, h: i16) -> [u16; 16] {
    let mut x = [0u16; 16];
    for (i, v) in [LAYOUT_EMPTY as i16, l, t, r, b, w, h].iter().enumerate() {
        x[6 + i] = *v as u16;
    }
    x
}

#[test]
fn rectangles_follow_the_layout_line_equations() {
    let screen = Rect::new(0, 0, 360, 640);
    let empty = LAYOUT_EMPTY as i16;
    let p = 0x4000i16;
    // the whole parent
    assert_eq!(layout_rect(screen, &line(0, 0, empty, empty, p, p)), screen);
    // 60 high at the bottom: top empty, bottom 0, height 60
    assert_eq!(layout_rect(screen, &line(0, empty, 0, 0, p, 60)), Rect::new(0, 580, 360, 60));
    // 20 in from the left and right, 30 from the top
    assert_eq!(layout_rect(screen, &line(20, 30, 20, empty, empty, 100)), Rect::new(20, 30, 320, 100));
    // nothing to trade when none of l, W, r is empty
    let l = layout_line(&line(5, 0, 7, 0, p, 10));
    assert_eq!((l.l, l.w, l.r), (5, p as i32, 7));
    // l + W + r = parent: an empty W and a parent-relative r trade places (W = p - r - l, r = empty)
    let l = layout_line(&line(5, 0, p - 100, 0, empty, 10));
    assert_eq!((l.l, l.w, l.r), (5, 95, empty as i32));
}

#[test]
fn compose_puts_the_child_in_the_parents_parent() {
    let screen = Rect::new(0, 0, 360, 640);
    let empty = LAYOUT_EMPTY as i16;
    let p = 0x4000i16;
    // a pane 100 from the top, 60 from the bottom ...
    let pane = line(0, 100, 0, 60, p, empty);
    // ... and a child 10 inside it on every side
    let child = line(10, 10, 10, 10, empty, empty);
    let r = layout_rect(screen, &compose(pane, child));
    assert_eq!(r, Rect::new(10, 110, 340, 460));
}
