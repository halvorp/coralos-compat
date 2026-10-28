//! GL 1.2's 3D textures from REAL PowerPC programs: glTexImage3D / glTexSubImage3D carry tight little-endian images converted
//! through the program's unpack state (including GL_UNPACK_SKIP_IMAGES / GL_UNPACK_IMAGE_HEIGHT), and arguments 9+ come from
//! the stack parameter area.
mod ppc_gl;
use ppc_gl::*;
use std::cell::Cell;

fn keep(name: &'static str, at: i32) -> Call { Call { name, ints: vec![], fl: vec![], keep: Some(at) } }

#[test]
fn teximage3d_sends_the_images_tight_through_the_skip_images_and_image_height_state() {
    let (_, seen, _) = run(|d| {
        // 1x1 RGBA unsigned-byte images, image height 2 rows (so an image is 8 bytes apart at alignment 1), one image skipped:
        // the two images the program means start at bytes 8 and 16
        let px = d.put(&[0, 0, 0, 0, 9, 9, 9, 9, 1, 2, 3, 4, 9, 9, 9, 9, 5, 6, 7, 8, 9, 9, 9, 9]);
        vec![
            call("glPixelStorei", ints(&[0x0CF5, 1]), vec![]),
            call("glPixelStorei", ints(&[0x806E, 2]), vec![]),
            call("glPixelStorei", ints(&[0x806D, 1]), vec![]),
            // target level internalformat w h d border format type data
            call("glTexImage3D", vec![I::V(0x806F), I::V(0), I::V(0x1908), I::V(1), I::V(1), I::V(2), I::V(0), I::V(0x1908), I::V(0x1401), I::P(px)], vec![]),
            // xoffset yoffset zoffset w h d format type data
            call("glTexSubImage3D", vec![I::V(0x806F), I::V(0), I::V(0), I::V(0), I::V(1), I::V(1), I::V(1), I::V(1), I::V(0x1908), I::V(0x1401), I::P(px)], vec![]),
        ]
    }, true);
    let full = saw(&seen, "glTexImage3D");
    assert_eq!(full[0].0, [0x806F, 0, 0x1908, 1, 1, 2, 0, 0x1908, 0x1401, 1], "nine scalars + has_data, the stack arguments read");
    assert_eq!(full[0].1, [1, 2, 3, 4, 5, 6, 7, 8], "the skipped image and the padding rows are not sent");
    let sub = saw(&seen, "glTexSubImage3D");
    assert_eq!(sub[0].0, [0x806F, 0, 0, 0, 1, 1, 1, 1, 0x1908, 0x1401]);
    assert_eq!(sub[0].1, [1, 2, 3, 4]);
}

#[test]
fn teximage3d_refuses_a_wrong_target_and_negative_sizes_with_gls_errors() {
    let at = Cell::new(0);
    let (m, seen, d) = run(|data| {
        at.set(data.put(&[0xEE; 16]));
        let a = at.get();
        let px = data.put(&[0; 16]);
        vec![
            call("glTexImage3D", vec![I::V(0x0DE1), I::V(0), I::V(0x1908), I::V(1), I::V(1), I::V(1), I::V(0), I::V(0x1908), I::V(0x1401), I::P(px)], vec![]), keep("glGetError", a),
            call("glTexImage3D", vec![I::V(0x806F), I::V(0), I::V(0x1908), I::V(1), I::V(1), I::V(-1i32 as u32), I::V(0), I::V(0x1908), I::V(0x1401), I::P(px)], vec![]), keep("glGetError", a + 4),
            call("glTexSubImage3D", vec![I::V(0x806F), I::V(0), I::V(0), I::V(0), I::V(0), I::V(1), I::V(1), I::V(1), I::V(0x1908), I::V(0x1401), I::V(0)], vec![]), keep("glGetError", a + 8),
        ]
    }, true);
    assert!(saw(&seen, "glTexImage3D").is_empty() && saw(&seen, "glTexSubImage3D").is_empty(), "NEGATIVE: no refused call reaches the worker");
    let base = d + at.get() as u32;
    let rd = |o: u32| m.mac.mem.read_u32(base + o).unwrap();
    assert_eq!([rd(0), rd(4), rd(8)], [0x0500, 0x0501, 0x0501]);
}
