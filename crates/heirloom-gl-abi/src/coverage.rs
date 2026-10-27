//! Which contract entries are graded where. The worker's GPU tests (`userland/coral-heirloom-gl/tests/gpu_families`, `gpu_state`) grade
//! every entry whose behaviour is a picture or a GL state; an entry whose behaviour lives ENTIRELY in the engine (it validates, keeps the
//! program's own client state, or answers a query from it) is graded by the engine's tests, and is listed here with the test file that
//! calls it. The engine's `tests/gl_coverage.rs` fails if a listed file does not exercise the entry, and the worker's coverage test counts
//! these entries as graded - so no switched-on entry is graded nowhere.

/// (entry, the engine test file in `crates/heirloom-macppc-engine/tests` that calls it).
pub const ENGINE_GRADED: &[(&str, &str)] = &[
    ("glArrayElement", "gl_arrays.rs"),
    ("glDrawRangeElements", "gl_arrays.rs"),
    ("glEdgeFlagPointer", "gl_arrays.rs"),
    ("glFeedbackBuffer", "gl_bulk.rs"),
    ("glGetPointerv", "gl_arrays.rs"),
    ("glIndexPointer", "gl_arrays.rs"),
    ("glInterleavedArrays", "gl_arrays.rs"),
    ("glPixelStoref", "gl_arrays.rs"),
    ("glPopClientAttrib", "gl_arrays.rs"),
    ("glPushClientAttrib", "gl_arrays.rs"),
    ("glRenderMode", "gl_bulk.rs"),
    ("glSelectBuffer", "gl_bulk.rs"),
];
