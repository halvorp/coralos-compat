//! Client arrays from REAL PowerPC programs: glIndexPointer, glEdgeFlagPointer, glInterleavedArrays (all 14 formats), glDrawArrays,
//! glDrawRangeElements, glArrayElement, glPush/PopClientAttrib, glPixelStoref and glGetPointerv. The worker mock records the draws
//! and calls it receives.
mod ppc_gl;
use heirloom_gl_abi::snapshot::{ElemType, Role};
use ppc_gl::*;
use std::cell::Cell;

fn keep(name: &'static str, at: i32) -> Call { Call { name, ints: vec![], fl: vec![], keep: Some(at) } }
fn be(v: &[f32]) -> Vec<u8> { v.iter().flat_map(|x| x.to_be_bytes()).collect() }
fn rd(m: &M, base: u32, o: u32) -> u32 { m.mac.mem.read_u32(base + o).unwrap() }

#[test]
fn glinterleavedarrays_sets_every_array_of_every_format_as_the_specification_tabulates() {
    // (format, enabled normal, colour, texcoord; vertex size; stride in bytes when glInterleavedArrays is given 0): GL 1.2 Table 2.5
    let rows: [(u32, bool, bool, bool, u32, u32); 14] = [
        (0x2A20, false, false, false, 2, 8), (0x2A21, false, false, false, 3, 12), (0x2A22, false, true, false, 2, 12), (0x2A23, false, true, false, 3, 16),
        (0x2A24, false, true, false, 3, 24), (0x2A25, true, false, false, 3, 24), (0x2A26, true, true, false, 3, 40), (0x2A27, false, false, true, 3, 20),
        (0x2A28, false, false, true, 4, 32), (0x2A29, false, true, true, 3, 20), (0x2A2A, false, true, true, 3, 32), (0x2A2B, true, false, true, 3, 32),
        (0x2A2C, true, true, true, 3, 48), (0x2A2D, true, true, true, 4, 60),
    ];
    for (format, normal, colour, tex, vsize, stride) in rows {
        let at = Cell::new(0);
        let (m, _, d) = run(|data| {
            at.set(data.put(&[0xEE; 64]));
            let a = at.get();
            vec![
                call("glInterleavedArrays", vec![I::V(format), I::V(0), I::V(0x1000)], vec![]),
                Call { name: "glIsEnabled", ints: ints(&[0x8075]), fl: vec![], keep: Some(a) },
                Call { name: "glIsEnabled", ints: ints(&[0x8076]), fl: vec![], keep: Some(a + 4) },
                Call { name: "glIsEnabled", ints: ints(&[0x8078]), fl: vec![], keep: Some(a + 8) },
                Call { name: "glIsEnabled", ints: ints(&[0x8074]), fl: vec![], keep: Some(a + 12) },
                call("glGetIntegerv", vec![I::V(0x807A), I::P(a + 16)], vec![]),
                call("glGetIntegerv", vec![I::V(0x807C), I::P(a + 20)], vec![]),
            ]
        }, true);
        let base = d + at.get() as u32;
        assert_eq!([rd(&m, base, 0) == 1, rd(&m, base, 4) == 1, rd(&m, base, 8) == 1, rd(&m, base, 12) == 1], [normal, colour, tex, true], "format {format:#x}: enabled arrays");
        assert_eq!((rd(&m, base, 16), rd(&m, base, 20)), (vsize, stride), "format {format:#x}: vertex size and the default stride");
    }
}

#[test]
fn an_unknown_interleaved_format_and_a_negative_stride_are_errors_that_change_nothing() {
    let at = Cell::new(0);
    let (m, _, d) = run(|data| {
        at.set(data.put(&[0xEE; 16]));
        let a = at.get();
        vec![
            call("glInterleavedArrays", vec![I::V(0x2A1F), I::V(0), I::V(0x1000)], vec![]), keep("glGetError", a),
            call("glInterleavedArrays", vec![I::V(0x2A21), I::V(0xFFFF_FFFF), I::V(0x1000)], vec![]), keep("glGetError", a + 4),
            Call { name: "glIsEnabled", ints: ints(&[0x8074]), fl: vec![], keep: Some(a + 8) },
        ]
    }, true);
    let base = d + at.get() as u32;
    assert_eq!((rd(&m, base, 0), rd(&m, base, 4), rd(&m, base, 8)), (0x0500, 0x0501, 0));
}

#[test]
fn drawarrays_is_a_draw_of_the_indices_first_to_first_plus_count_with_the_enabled_arrays_snapshotted() {
    let (_, seen, _) = run(|d| {
        // vertices (x, y) as floats: 0:(1,2) 1:(3,4) 2:(5,6) 3:(7,8): draw 2..4 of a GL_TRIANGLE_STRIP-like list
        let v = d.put(&be(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]));
        vec![
            call("glEnableClientState", ints(&[0x8074]), vec![]),
            call("glVertexPointer", vec![I::V(2), I::V(0x1406), I::V(0), I::P(v)], vec![]),
            call("glDrawArrays", ints(&[5, 1, 3]), vec![]),
        ]
    }, true);
    let d = &seen.lock().unwrap().draws[0];
    assert_eq!(d.mode, 5);
    assert_eq!(d.indices.ty, ElemType::U32);
    assert_eq!(d.indices.data, [1u32, 2, 3].iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<_>>());
    assert_eq!(d.arrays.len(), 1);
    assert_eq!((d.arrays[0].role, d.arrays[0].first, d.arrays[0].count), (Role::Vertex, 1, 3));
    let floats: Vec<f32> = d.arrays[0].data.chunks(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    assert_eq!(floats, [3.0, 4.0, 5.0, 6.0, 7.0, 8.0], "vertices 1..=3, little-endian");
}

#[test]
fn double_precision_arrays_are_snapshotted_and_every_array_type_is_checked_by_role() {
    let at = Cell::new(0);
    let (m, seen, d) = run(|data| {
        at.set(data.put(&[0xEE; 16]));
        let a = at.get();
        let v = data.put(&[0x3F, 0xF0, 0, 0, 0, 0, 0, 0, 0x40, 0, 0, 0, 0, 0, 0, 0]); // 1.0, 2.0 as doubles
        vec![
            call("glEnableClientState", ints(&[0x8074]), vec![]),
            call("glVertexPointer", vec![I::V(2), I::V(0x140A), I::V(0), I::P(v)], vec![]),
            call("glDrawArrays", ints(&[0, 0, 1]), vec![]),
            call("glVertexPointer", vec![I::V(2), I::V(0x1401), I::V(0), I::P(v)], vec![]), keep("glGetError", a),       // unsigned bytes are not a vertex type
            call("glNormalPointer", vec![I::V(0x1401), I::V(0), I::P(v)], vec![]), keep("glGetError", a + 4),           // nor a normal type
            call("glColorPointer", vec![I::V(4), I::V(0x1401), I::V(0), I::P(v)], vec![]), keep("glGetError", a + 8),   // but a colour type
            call("glTexCoordPointer", vec![I::V(5), I::V(0x1406), I::V(0), I::P(v)], vec![]), keep("glGetError", a + 12), // size 5
        ]
    }, true);
    let dr = &seen.lock().unwrap().draws[0];
    assert_eq!(dr.arrays[0].ty, ElemType::F64);
    assert_eq!(dr.arrays[0].data, [1.0f64.to_le_bytes(), 2.0f64.to_le_bytes()].concat());
    let base = d + at.get() as u32;
    assert_eq!([rd(&m, base, 0), rd(&m, base, 4), rd(&m, base, 8), rd(&m, base, 12)], [0x0500, 0x0500, 0, 0x0501]);
}

#[test]
fn drawrangeelements_is_validated_and_is_drawelements() {
    let at = Cell::new(0);
    let (m, seen, d) = run(|data| {
        at.set(data.put(&[0xEE; 16]));
        let a = at.get();
        let v = data.put(&be(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]));
        let idx = data.put(&[0, 1, 2]);
        vec![
            call("glEnableClientState", ints(&[0x8074]), vec![]),
            call("glVertexPointer", vec![I::V(2), I::V(0x1406), I::V(0), I::P(v)], vec![]),
            call("glDrawRangeElements", vec![I::V(4), I::V(0), I::V(2), I::V(3), I::V(0x1401), I::P(idx)], vec![]),
            call("glDrawRangeElements", vec![I::V(4), I::V(5), I::V(2), I::V(3), I::V(0x1401), I::P(idx)], vec![]), keep("glGetError", a),  // end < start
            call("glDrawRangeElements", vec![I::V(4), I::V(0), I::V(2), I::V(0xFFFF_FFFF), I::V(0x1401), I::P(idx)], vec![]), keep("glGetError", a + 4), // negative count
            call("glDrawRangeElements", vec![I::V(4), I::V(0), I::V(2), I::V(3), I::V(0x1402), I::P(idx)], vec![]), keep("glGetError", a + 8), // signed type
            call("glDrawRangeElements", vec![I::V(10), I::V(0), I::V(2), I::V(3), I::V(0x1401), I::P(idx)], vec![]), keep("glGetError", a + 12), // mode 10
        ]
    }, true);
    assert_eq!(seen.lock().unwrap().draws.len(), 1, "only the valid draw reached the worker");
    let base = d + at.get() as u32;
    assert_eq!([rd(&m, base, 0), rd(&m, base, 4), rd(&m, base, 8), rd(&m, base, 12)], [0x0501, 0x0501, 0x0500, 0x0500]);
}

#[test]
fn arrayelement_sends_the_enabled_arrays_as_the_vertex_calls_it_is_equivalent_to_in_the_specified_order() {
    let (_, seen, _) = run(|d| {
        let v = d.put(&be(&[1.0, 2.0, 10.0, 20.0, 30.0, 40.0])); // vertex 0: (1,2) vertex 1: (10,20) ...
        let c = d.put(&[9, 8, 7, 200, 6, 5, 4, 100]);            // two RGBA ubyte colours
        let n = d.put(&[0x7F, 0x00, 0x81, 0x00, 0x01, 0x02]);    // two byte normals (stride 3)
        vec![
            call("glEnableClientState", ints(&[0x8074]), vec![]), call("glEnableClientState", ints(&[0x8076]), vec![]), call("glEnableClientState", ints(&[0x8075]), vec![]),
            call("glVertexPointer", vec![I::V(2), I::V(0x1406), I::V(0), I::P(v)], vec![]),
            call("glColorPointer", vec![I::V(4), I::V(0x1401), I::V(0), I::P(c)], vec![]),
            call("glNormalPointer", vec![I::V(0x1400), I::V(3), I::P(n)], vec![]),
            call("glBegin", ints(&[4]), vec![]),
            call("glArrayElement", ints(&[1]), vec![]),
            call("glEnd", vec![], vec![]),
        ]
    }, true);
    let names: Vec<String> = seen.lock().unwrap().calls.iter().map(|c| c.0.clone()).filter(|n| !n.starts_with("agl")).collect();
    assert_eq!(names, ["glBegin", "glColor4ubv", "glNormal3bv", "glVertex2fv", "glEnd"], "colour, normal, vertex: the order of the specification");
    let s = seen.lock().unwrap();
    let w = |n: &str| s.calls.iter().find(|c| c.0 == n).unwrap().1.clone();
    assert_eq!(w("glColor4ubv"), [6, 5, 4, 100], "element 1 of the colour array");
    assert_eq!(w("glNormal3bv"), [0x0, 0x1, 0x2], "element 1 of a stride-3 byte normal array");
    assert_eq!(w("glVertex2fv"), [u64::from(10.0f32.to_bits()), u64::from(20.0f32.to_bits())]);
}

#[test]
fn the_client_attribute_stack_saves_and_restores_arrays_and_pixel_storage_and_underflows_by_name() {
    let at = Cell::new(0);
    let (m, seen, d) = run(|data| {
        at.set(data.put(&[0xEE; 32]));
        let a = at.get();
        vec![
            call("glPushClientAttrib", ints(&[0x3]), vec![]),
            call("glEnableClientState", ints(&[0x8074]), vec![]),
            call("glPixelStorei", ints(&[0x0CF5, 1]), vec![]),
            Call { name: "glIsEnabled", ints: ints(&[0x8074]), fl: vec![], keep: Some(a) },
            call("glPopClientAttrib", vec![], vec![]),
            Call { name: "glIsEnabled", ints: ints(&[0x8074]), fl: vec![], keep: Some(a + 4) },
            call("glPopClientAttrib", vec![], vec![]), keep("glGetError", a + 8),
        ]
    }, true);
    let base = d + at.get() as u32;
    assert_eq!((rd(&m, base, 0), rd(&m, base, 4), rd(&m, base, 8)), (1, 0, 0x0504), "the array is enabled in between, disabled after the pop; a second pop is GL_STACK_UNDERFLOW");
    let alignment_calls: Vec<Vec<u64>> = saw(&seen, "glPixelStorei").into_iter().map(|c| c.0).filter(|w| w[0] == 0x0CF5).collect();
    assert_eq!(alignment_calls, [vec![0x0CF5, 1], vec![0x0CF5, 4]], "the worker is told the restored unpack alignment too");
}

#[test]
fn pixelstoref_rounds_and_getpointerv_returns_what_the_program_set() {
    let at = Cell::new(0);
    let (m, seen, d) = run(|data| {
        at.set(data.put(&[0xEE; 32]));
        let a = at.get();
        let half = data.put(&be(&[2.5, -0.4]));
        vec![
            call("glPixelStoref", ints(&[0x0CF2]), vec![Fl::S(half)]),
            call("glPixelStoref", ints(&[0x0CF5]), vec![Fl::S(half)]), keep("glGetError", a), // alignment 3: invalid
            call("glVertexPointer", vec![I::V(3), I::V(0x1406), I::V(0), I::V(0x4000)], vec![]),
            call("glIndexPointer", vec![I::V(0x1402), I::V(0), I::V(0x5000)], vec![]),
            call("glEdgeFlagPointer", vec![I::V(0), I::V(0x6000)], vec![]),
            call("glGetPointerv", vec![I::V(0x808E), I::P(a + 4)], vec![]),
            call("glGetPointerv", vec![I::V(0x8091), I::P(a + 8)], vec![]),
            call("glGetPointerv", vec![I::V(0x8093), I::P(a + 12)], vec![]),
            call("glGetPointerv", vec![I::V(0x1234), I::P(a + 16)], vec![]), keep("glGetError", a + 20),
        ]
    }, true);
    let base = d + at.get() as u32;
    assert_eq!(saw(&seen, "glPixelStorei")[0].0, [0x0CF2, 3], "2.5 rounds to 3 and is forwarded as an integer");
    assert_eq!((rd(&m, base, 0), rd(&m, base, 4), rd(&m, base, 8), rd(&m, base, 12), rd(&m, base, 16), rd(&m, base, 20)), (0x0501, 0x4000, 0x5000, 0x6000, 0xEEEE_EEEE, 0x0500));
}
