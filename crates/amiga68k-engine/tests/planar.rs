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

#[test]
fn what_the_engine_draws_into_a_planar_bitmap_reaches_the_programs_planes_and_back() {
    let mut a = machine();
    let r = run(&mut a, PROG);
    assert_eq!((r[11], r[12]), (0xCD, 0xAC), "WritePixelLine8 of pens 3,1,2,0,3,3,0,1 wrote plane 0 = 11001101 and plane 1 = 10101100 into the program's planes");
    assert_eq!(r[13], 0x55, "control: the neighbouring plane byte the line did not cover is untouched");
    assert_eq!((r[14], r[15]), (0, 3), "ReadPixelLine8 reads pens from the plane bytes the program wrote itself (x8 = 00, x9 = 11 in the two planes)");
}
