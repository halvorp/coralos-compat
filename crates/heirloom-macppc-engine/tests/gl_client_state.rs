//! Client-array state is the ENGINE's (`sig::engine_answers`): the program's enables, sizes, types and strides are answered
//! without asking the worker, in the type the query asks for; every other state is the worker's to answer.
mod ppc_gl;
use ppc_gl::*;
use std::cell::Cell;

#[test]
fn client_array_queries_are_answered_by_the_engine_in_every_type() {
    let at = Cell::new(0);
    let (m, seen, d) = run(|data| {
        at.set(data.put(&[0xEE; 64]));
        let a = at.get();
        vec![
            call("glEnableClientState", ints(&[0x8074]), vec![]),
            call("glVertexPointer", vec![I::V(3), I::V(0x1402), I::V(8), I::P(a)], vec![]),
            Call { name: "glIsEnabled", ints: ints(&[0x8074]), fl: vec![], keep: Some(a) },
            Call { name: "glIsEnabled", ints: ints(&[0x8075]), fl: vec![], keep: Some(a + 4) },
            call("glGetIntegerv", vec![I::V(0x807A), I::P(a + 8)], vec![]),
            call("glGetIntegerv", vec![I::V(0x807B), I::P(a + 12)], vec![]),
            call("glGetFloatv", vec![I::V(0x807C), I::P(a + 16)], vec![]),
            call("glGetBooleanv", vec![I::V(0x8074), I::P(a + 20)], vec![]),
            call("glGetDoublev", vec![I::V(0x808C), I::P(a + 24)], vec![]),
            call("glGetIntegerv", vec![I::V(0x8082), I::P(a + 32)], vec![]),
        ]
    }, true);
    let rd = |k: u32| m.mac.mem.read_u32(d + at.get() as u32 + k).unwrap();
    assert_eq!((rd(0), rd(4)), (1, 0), "GL_VERTEX_ARRAY is enabled, GL_NORMAL_ARRAY is not");
    assert_eq!((rd(8), rd(12)), (3, 0x1402), "size and type as glVertexPointer set them");
    assert_eq!(f32::from_bits(rd(16)), 8.0, "the stride asked as a float");
    assert_eq!(m.mac.mem.read_u8(d + at.get() as u32 + 20).unwrap(), 1, "asked as a boolean: one byte");
    assert_eq!(m.mac.mem.read_u8(d + at.get() as u32 + 21).unwrap(), 0xEE, "and not a byte more");
    assert_eq!(f64::from_bits(u64::from(rd(24)) << 32 | u64::from(rd(28))), 0.0, "GL_EDGE_FLAG_ARRAY_STRIDE initial value, asked as a double (big-endian)");
    assert_eq!(rd(32), 0x1406, "the initial colour array type is GL_FLOAT");
    for n in ["glIsEnabled", "glGetIntegerv", "glGetFloatv", "glGetBooleanv", "glGetDoublev"] {
        assert!(saw(&seen, n).is_empty(), "{n} on client-array state never reached the worker");
    }
}

#[test]
fn the_index_and_edge_flag_arrays_are_client_state_the_engine_keeps() {
    let at = Cell::new(0);
    let (m, _, d) = run(|data| {
        at.set(data.put(&[0xEE; 16]));
        vec![
            call("glEnableClientState", ints(&[0x8077]), vec![]),
            Call { name: "glIsEnabled", ints: ints(&[0x8077]), fl: vec![], keep: Some(at.get()) },
            call("glEnableClientState", ints(&[0x8079]), vec![]),
            Call { name: "glIsEnabled", ints: ints(&[0x8079]), fl: vec![], keep: Some(at.get() + 4) },
        ]
    }, true);
    assert_eq!(m.mac.mem.read_u32(d + at.get() as u32).unwrap(), 1, "GL_INDEX_ARRAY has no effect in RGBA mode but it is real state");
    assert_eq!(m.mac.mem.read_u32(d + at.get() as u32 + 4).unwrap(), 1, "GL_EDGE_FLAG_ARRAY is enabled (a draw carries it)");
    assert!(m.fault().is_none());
}
