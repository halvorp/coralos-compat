//! The bulk-data calls (`heirloom_gl_abi::bulk_table`) from REAL PowerPC programs: control points, pixel maps, list names and priorities
//! are repacked from the program's memory (strides, big-endian, element types) into the tight words the table lays out; answers land in
//! the program's memory; every invalid call is the GL error the specification names and is never sent.
mod ppc_gl;
use ppc_gl::*;
use std::cell::Cell;

fn keep(name: &'static str, at: i32) -> Call { Call { name, ints: vec![], fl: vec![], keep: Some(at) } }
fn be(v: &[f32]) -> Vec<u8> { v.iter().flat_map(|x| x.to_be_bytes()).collect() }
fn bed(v: &[f64]) -> Vec<u8> { v.iter().flat_map(|x| x.to_be_bytes()).collect() }
fn f(x: f32) -> u64 { u64::from(x.to_bits()) }

#[test]
fn map1f_control_points_are_repacked_tight_from_the_programs_stride() {
    let (_, seen, _) = run(|d| {
        // 3 control points of 3 components, a stride of 5 floats: the two padding floats per point are not sent
        let pts = d.put(&be(&[1.0, 2.0, 3.0, 9.0, 9.0, 4.0, 5.0, 6.0, 9.0, 9.0, 7.0, 8.0, 9.0]));
        let u = d.put(&be(&[0.0, 2.0]));
        vec![call("glMap1f", vec![I::V(0x0D97), I::V(0), I::V(0), I::V(5), I::V(3), I::P(pts)], vec![Fl::S(u), Fl::S(u + 4)])]
    }, true);
    assert_eq!(saw(&seen, "glMap1f")[0].0, [0x0D97, f(0.0), f(2.0), 3, 3, f(1.0), f(2.0), f(3.0), f(4.0), f(5.0), f(6.0), f(7.0), f(8.0), f(9.0)]);
}

#[test]
fn map1d_takes_its_domain_in_double_registers_and_the_stride_after_them() {
    let (_, seen, _) = run(|d| {
        let pts = d.put(&bed(&[1.5, 2.5, 3.5, 4.5]));
        let dom = d.put(&bed(&[-1.0, 3.0]));
        // glMap1d(target, u1 (2 words), u2 (2 words), stride, order, points): the integers are at parameter words 0, 5, 6, 7
        vec![call("glMap1d", vec![I::V(0x0D91), I::V(0), I::V(0), I::V(0), I::V(0), I::V(1), I::V(4), I::P(pts)], vec![Fl::D(dom), Fl::D(dom + 8)])]
    }, true);
    let w = &saw(&seen, "glMap1d")[0].0;
    assert_eq!(w[..5], [0x0D91, (-1.0f64).to_bits(), 3.0f64.to_bits(), 4, 1], "GL_MAP1_INDEX: one component per point, order 4");
    assert_eq!(w[5..], [1.5f64.to_bits(), 2.5f64.to_bits(), 3.5f64.to_bits(), 4.5f64.to_bits()]);
}

#[test]
fn map2f_uses_element_strides_in_both_directions_and_the_stack_for_the_late_arguments() {
    let (_, seen, _) = run(|d| {
        // a 2 x 2 net of 3-component points, ustride 8 (elements), vstride 4: point (i, j) at element i * 8 + j * 4
        let mut net = vec![0.0f32; 16];
        for (i, j, base) in [(0, 0, 1.0), (0, 1, 2.0), (1, 0, 3.0), (1, 1, 4.0)] { for c in 0..3 { net[i * 8 + j * 4 + c] = base * 10.0 + c as f32; } }
        let pts = d.put(&be(&net));
        let dom = d.put(&be(&[0.0, 1.0, -1.0, 1.0]));
        // glMap2f(target, u1, u2, ustride, uorder, v1, v2, vstride, vorder, points): parameter words 0, 3, 4, 7, 8, 9 (the last two on the stack)
        vec![call("glMap2f", vec![I::V(0x0DB7), I::V(0), I::V(0), I::V(8), I::V(2), I::V(0), I::V(0), I::V(4), I::V(2), I::P(pts)], vec![Fl::S(dom), Fl::S(dom + 4), Fl::S(dom + 8), Fl::S(dom + 12)])]
    }, true);
    let w = &saw(&seen, "glMap2f")[0].0;
    assert_eq!(w[..8], [0x0DB7, f(0.0), f(1.0), 2, f(-1.0), f(1.0), 2, 3]);
    let want: Vec<u64> = [(0, 0, 1.0), (0, 1, 2.0), (1, 0, 3.0), (1, 1, 4.0)].iter().flat_map(|(_, _, b)| (0..3).map(move |c| f(b * 10.0 + c as f32))).collect();
    assert_eq!(w[8..], want[..], "the u index is the slowest, each point is its three components");
}

#[test]
fn an_invalid_evaluator_map_is_the_errors_the_specification_names_and_is_never_sent() {
    let at = Cell::new(0);
    let (m, seen, d) = run(|data| {
        at.set(data.put(&[0xEE; 16]));
        let pts = data.put(&be(&[0.0; 16]));
        let dom = data.put(&be(&[0.0, 1.0, 0.0, 0.0]));
        let a = at.get();
        vec![
            call("glMap1f", vec![I::V(0x0D90 - 1), I::V(0), I::V(0), I::V(4), I::V(2), I::P(pts)], vec![Fl::S(dom), Fl::S(dom + 4)]), keep("glGetError", a),          // not a map target
            call("glMap1f", vec![I::V(0x0D90), I::V(0), I::V(0), I::V(3), I::V(2), I::P(pts)], vec![Fl::S(dom), Fl::S(dom + 4)]), keep("glGetError", a + 4),          // stride 3 < 4 components
            call("glMap1f", vec![I::V(0x0D97), I::V(0), I::V(0), I::V(3), I::V(0), I::P(pts)], vec![Fl::S(dom), Fl::S(dom + 4)]), keep("glGetError", a + 8),          // order 0
            call("glMap1f", vec![I::V(0x0D97), I::V(0), I::V(0), I::V(3), I::V(2), I::P(pts)], vec![Fl::S(dom + 8), Fl::S(dom + 8)]), keep("glGetError", a + 12),    // u1 == u2
        ]
    }, true);
    let errs: Vec<u32> = (0..4).map(|k| m.mac.mem.read_u32(d + at.get() as u32 + 4 * k).unwrap()).collect();
    assert_eq!(errs, [0x0500, 0x0501, 0x0501, 0x0501]);
    assert!(saw(&seen, "glMap1f").is_empty());
}

#[test]
fn get_map_stores_what_the_worker_answers_in_the_programs_type_and_order() {
    let at = Cell::new(0);
    let (m, _, d) = run(|data| {
        at.set(data.put(&[0xEE; 64]));
        let a = at.get();
        vec![
            call("glGetMapdv", vec![I::V(0x0D97), I::V(0x0A00), I::P(a)], vec![]),
            call("glGetMapfv", vec![I::V(0x0D97), I::V(0x0A00), I::P(a + 24)], vec![]),
            call("glGetMapiv", vec![I::V(0x0D97), I::V(0x0A01), I::P(a + 36)], vec![]),
        ]
    }, true);
    let base = d + at.get() as u32;
    let dbl = |o: u32| f64::from_bits(u64::from(m.mac.mem.read_u32(base + o).unwrap()) << 32 | u64::from(m.mac.mem.read_u32(base + o + 4).unwrap()));
    assert_eq!([dbl(0), dbl(8), dbl(16)], [0.5, 1.5, 2.5]);
    assert_eq!([f32::from_bits(m.mac.mem.read_u32(base + 24).unwrap()), f32::from_bits(m.mac.mem.read_u32(base + 28).unwrap())], [0.5, 1.5]);
    assert_eq!([m.mac.mem.read_u32(base + 36).unwrap(), m.mac.mem.read_u32(base + 40).unwrap(), m.mac.mem.read_u32(base + 44).unwrap()], [7, 0xFFFF_FFFF, 9]);
    assert_eq!(m.mac.mem.read_u8(base + 48).unwrap(), 0xEE, "not a byte past the answer");
}

#[test]
fn pixel_maps_are_read_in_the_programs_type_and_validated() {
    let at = Cell::new(0);
    let (m, seen, d) = run(|data| {
        at.set(data.put(&[0xEE; 64]));
        let fl = data.put(&be(&[0.0, 0.5, 1.0]));
        let ui = data.put(&[0, 0, 0, 1, 0, 0, 0, 2]);
        let us = data.put(&[0x80, 0x00, 0xFF, 0xFF]);
        let a = at.get();
        vec![
            call("glPixelMapfv", vec![I::V(0x0C76), I::V(3), I::P(fl)], vec![]),
            call("glPixelMapuiv", vec![I::V(0x0C70), I::V(2), I::P(ui)], vec![]),
            call("glPixelMapusv", vec![I::V(0x0C78), I::V(2), I::P(us)], vec![]),
            call("glPixelMapfv", vec![I::V(0x0BAD), I::V(3), I::P(fl)], vec![]), keep("glGetError", a),
            call("glPixelMapfv", vec![I::V(0x0C76), I::V(0xFFFF_FFFF), I::P(fl)], vec![]), keep("glGetError", a + 4),
            call("glGetPixelMapfv", vec![I::V(0x0C76), I::P(a + 8)], vec![]),
            call("glGetPixelMapusv", vec![I::V(0x0C78), I::P(a + 40)], vec![]),
        ]
    }, true);
    assert_eq!(saw(&seen, "glPixelMapfv").len(), 1, "the invalid ones are not sent");
    assert_eq!(saw(&seen, "glPixelMapfv")[0].0, [0x0C76, 3, f(0.0), f(0.5), f(1.0)]);
    assert_eq!(saw(&seen, "glPixelMapuiv")[0].0, [0x0C70, 2, 1, 2]);
    assert_eq!(saw(&seen, "glPixelMapusv")[0].0, [0x0C78, 2, 0x8000, 0xFFFF]);
    let base = d + at.get() as u32;
    assert_eq!((m.mac.mem.read_u32(base).unwrap(), m.mac.mem.read_u32(base + 4).unwrap()), (0x0500, 0x0501));
    assert_eq!(f32::from_bits(m.mac.mem.read_u32(base + 8 + 12).unwrap()), 0.75, "four floats came back, big-endian");
    assert_eq!(m.mac.mem.read_u16(base + 40 + 6).unwrap(), 0xFFFF, "unsigned shorts, big-endian");
}

#[test]
fn call_lists_converts_every_name_type_to_unsigned_integers() {
    let (_, seen, _) = run(|d| {
        let bytes = d.put(&[0xFF, 0x02]);
        let shorts = d.put(&[0xFF, 0xFE, 0x01, 0x02]);
        let ints = d.put(&[0xFF, 0xFF, 0xFF, 0xFD, 0, 0, 1, 0]);
        let floats = d.put(&be(&[2.9, 7.0]));
        let three = d.put(&[0x01, 0x02, 0x03, 0xAA, 0xBB, 0xCC]);
        let four = d.put(&[0x01, 0x02, 0x03, 0x04]);
        vec![
            call("glCallLists", vec![I::V(2), I::V(0x1400), I::P(bytes)], vec![]),    // GL_BYTE: signed
            call("glCallLists", vec![I::V(2), I::V(0x1401), I::P(bytes)], vec![]),    // GL_UNSIGNED_BYTE
            call("glCallLists", vec![I::V(2), I::V(0x1402), I::P(shorts)], vec![]),   // GL_SHORT
            call("glCallLists", vec![I::V(2), I::V(0x1403), I::P(shorts)], vec![]),   // GL_UNSIGNED_SHORT
            call("glCallLists", vec![I::V(2), I::V(0x1404), I::P(ints)], vec![]),     // GL_INT
            call("glCallLists", vec![I::V(2), I::V(0x1406), I::P(floats)], vec![]),   // GL_FLOAT
            call("glCallLists", vec![I::V(1), I::V(0x1407), I::P(shorts)], vec![]),   // GL_2_BYTES
            call("glCallLists", vec![I::V(2), I::V(0x1408), I::P(three)], vec![]),    // GL_3_BYTES
            call("glCallLists", vec![I::V(1), I::V(0x1409), I::P(four)], vec![]),     // GL_4_BYTES
            call("glCallLists", vec![I::V(1), I::V(0x1234), I::P(four)], vec![]),     // an invalid type: never sent
        ]
    }, true);
    let s = saw(&seen, "glCallLists");
    assert_eq!(s.len(), 9, "the invalid type is GL_INVALID_ENUM and not sent");
    let n = |k: usize| s[k].0.clone();
    assert_eq!(n(0), [2, 0xFFFF_FFFF, 2], "GL_BYTE sign-extends");
    assert_eq!(n(1), [2, 255, 2]);
    assert_eq!(n(2), [2, 0xFFFF_FFFE, 0x0102]);
    assert_eq!(n(3), [2, 0xFFFE, 0x0102]);
    assert_eq!(n(4), [2, 0xFFFF_FFFD, 256]);
    assert_eq!(n(5), [2, 2, 7], "GL_FLOAT truncates");
    assert_eq!(n(6), [1, 0xFFFE], "GL_2_BYTES: one big-endian group of two bytes");
    assert_eq!(n(7), [2, 0x010203, 0xAABBCC]);
    assert_eq!(n(8), [1, 0x0102_0304]);
}

#[test]
fn texture_priorities_and_residence_go_through_the_worker() {
    let at = Cell::new(0);
    let (m, seen, d) = run(|data| {
        at.set(data.put(&[0xEE; 16]));
        let names = data.put(&[0, 0, 0, 3, 0, 0, 0, 4, 0, 0, 0, 5]);
        let prios = data.put(&be(&[0.25, 0.5, 1.0]));
        let a = at.get();
        vec![
            call("glPrioritizeTextures", vec![I::V(3), I::P(names), I::P(prios)], vec![]),
            Call { name: "glAreTexturesResident", ints: vec![I::V(3), I::P(names), I::P(a + 4)], fl: vec![], keep: Some(a) },
        ]
    }, true);
    assert_eq!(saw(&seen, "glPrioritizeTextures")[0].0, [3, 3, 4, 5, f(0.25), f(0.5), f(1.0)]);
    assert_eq!(saw(&seen, "glAreTexturesResident")[0].0, [3, 3, 4, 5]);
    let base = d + at.get() as u32;
    assert_eq!(m.mac.mem.read_u32(base).unwrap(), 0, "the returned value is the worker's first word");
    assert_eq!([m.mac.mem.read_u8(base + 4).unwrap(), m.mac.mem.read_u8(base + 5).unwrap(), m.mac.mem.read_u8(base + 6).unwrap(), m.mac.mem.read_u8(base + 7).unwrap()], [1, 1, 0, 0xEE], "one byte per name");
}

#[test]
fn selection_and_feedback_buffers_are_validated_and_render_mode_serves_only_render() {
    let at = Cell::new(0);
    let (m, seen, d) = run(|data| {
        at.set(data.put(&[0xEE; 32]));
        let buf = data.put(&[0; 64]);
        let a = at.get();
        vec![
            call("glSelectBuffer", vec![I::V(16), I::P(buf)], vec![]),
            call("glFeedbackBuffer", vec![I::V(16), I::V(0x0602), I::P(buf)], vec![]),
            call("glFeedbackBuffer", vec![I::V(16), I::V(0x0700), I::P(buf)], vec![]), keep("glGetError", a),
            call("glSelectBuffer", vec![I::V(0xFFFF_FFFF), I::P(buf)], vec![]), keep("glGetError", a + 4),
            Call { name: "glRenderMode", ints: ints(&[0x1C00]), fl: vec![], keep: Some(a + 8) },
            call("glRenderMode", ints(&[0x1234]), vec![]), keep("glGetError", a + 12),
        ]
    }, true);
    assert_eq!(saw(&seen, "glSelectBuffer")[0].0, [16]);
    assert_eq!(saw(&seen, "glFeedbackBuffer")[0].0, [16, 0x0602]);
    assert_eq!(saw(&seen, "glFeedbackBuffer").len(), 1, "an invalid type is not sent");
    assert!(saw(&seen, "glRenderMode").is_empty(), "render mode is the engine's");
    let base = d + at.get() as u32;
    let rd = |o: u32| m.mac.mem.read_u32(base + o).unwrap();
    assert_eq!([rd(0), rd(4), rd(8), rd(12)], [0x0500, 0x0501, 0, 0x0500]);
    // GL_SELECT stops the program with a named refusal
    let (calls, dd) = layout(&|_: &mut Data| vec![call("glRenderMode", ints(&[0x1C02]), vec![])]);
    let (bytes, _) = assemble(&calls, &dd, true);
    let mut m = ppc_gl::M::new(heirloom_ui_abi::RecordingGuestUi::new()).unwrap();
    m.gl_attach(Box::new(Mock(heirloom_gl_abi::TokenTable::default(), Default::default())));
    let frag = m.instantiate("rm", &heirloom_fmt_pef::Pef::parse(&bytes).unwrap()).unwrap();
    m.start(&frag).unwrap();
    assert_ne!(m.step(5_000_000).0, heirloom_macppc_engine::Step::Exited);
    assert!(m.fault().expect("a named fault").to_string().contains("GL_SELECT"));
}
