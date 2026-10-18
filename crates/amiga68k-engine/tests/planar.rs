//! A BitMap InitBitMap'd over the program's own planes is read from those planes when used as a blit source (tests/guest/c/planar.c).
mod common;
use common::*;

const PROG: &[u8] = include_bytes!("guest/planar.bin");

#[test]
fn guest_planes_are_the_pixels_of_a_blit_source() {
    let mut a = machine();
    let r = run(&mut a, PROG);
    // plane0 = F0 (x0..3 set), plane1 = FF (all set): x0..3 = 3, x4..7 = 2.
    assert_eq!(&r[0..8], &[3, 3, 3, 3, 2, 2, 2, 2], "two planes combine into the pen");
    assert_eq!(r[8], 1, "row 1 (bytes AA,00 / 00,FF) x0: plane 0 only");
    assert_eq!(r[9], 0, "after the program zeroed the plane bytes, the next blit sees zeros (no stale host copy)");
    assert_eq!(r[10], 1, "a NULL plane reads as zeros: only plane 0 contributes");
}
