//! The calls that carry DATA of a runtime-determined size, written by hand (the registry gives `COMPSIZE(...)` for them, which a
//! generator cannot size). The layout of each is the [`crate::family::P`] list: scalar words first (each one word, as in
//! [`crate::family`]), then the data elements as words of the stated type, tightly packed - the engine repacks the program's memory
//! (strides, byte order, element type) into exactly this, so the worker never sees the guest's layout.
//!
//! A reply to a call that has both a returned value and an OUT section carries the value FIRST, then the elements.
use crate::family::{Op::Arg, Sig, Ty::*, P, R};

pub static BULK: &[Sig] = &[
    // evaluators (GL 1.2 §5.1): glMap1{f,d}(target, u1, u2, order, k) with order * k control-point components, repacked tight
    // (stride k); glMap2{f,d}(target, u1, u2, uorder, v1, v2, vorder, k) with uorder * vorder * k components, the u index slowest
    Sig::new("glMap1f", R::Void, &[P::S(Enum), P::S(Float), P::S(Float), P::S(Int), P::S(Int), P::Data(Float, &[Arg(3), Arg(4)])]),
    Sig::new("glMap1d", R::Void, &[P::S(Enum), P::S(Double), P::S(Double), P::S(Int), P::S(Int), P::Data(Double, &[Arg(3), Arg(4)])]),
    Sig::new("glMap2f", R::Void, &[P::S(Enum), P::S(Float), P::S(Float), P::S(Int), P::S(Float), P::S(Float), P::S(Int), P::S(Int), P::Data(Float, &[Arg(3), Arg(6), Arg(7)])]),
    Sig::new("glMap2d", R::Void, &[P::S(Enum), P::S(Double), P::S(Double), P::S(Int), P::S(Double), P::S(Double), P::S(Int), P::S(Int), P::Data(Double, &[Arg(3), Arg(6), Arg(7)])]),
    Sig::new("glGetMapdv", R::Void, &[P::S(Enum), P::S(Enum), P::OutAny(Double)]),
    Sig::new("glGetMapfv", R::Void, &[P::S(Enum), P::S(Enum), P::OutAny(Float)]),
    Sig::new("glGetMapiv", R::Void, &[P::S(Enum), P::S(Enum), P::OutAny(Int)]),
    // pixel maps (§3.6.3): glPixelMap*(map, n) with n values
    Sig::new("glPixelMapfv", R::Void, &[P::S(Enum), P::S(Sizei), P::Data(Float, &[Arg(1)])]),
    Sig::new("glPixelMapuiv", R::Void, &[P::S(Enum), P::S(Sizei), P::Data(Uint, &[Arg(1)])]),
    Sig::new("glPixelMapusv", R::Void, &[P::S(Enum), P::S(Sizei), P::Data(Ushort, &[Arg(1)])]),
    Sig::new("glGetPixelMapfv", R::Void, &[P::S(Enum), P::OutAny(Float)]),
    Sig::new("glGetPixelMapuiv", R::Void, &[P::S(Enum), P::OutAny(Uint)]),
    Sig::new("glGetPixelMapusv", R::Void, &[P::S(Enum), P::OutAny(Ushort)]),
    // display lists (§5.4): glCallLists(n) with the n list names already converted from the program's type to unsigned integers
    Sig::new("glCallLists", R::Void, &[P::S(Sizei), P::Data(Uint, &[Arg(0)])]),
    // texture objects (§3.8.12): glPrioritizeTextures(n): n names then n priorities; glAreTexturesResident(n): n names, the reply is
    // the returned boolean then n residences
    Sig::new("glPrioritizeTextures", R::Void, &[P::S(Sizei), P::Data(Uint, &[Arg(0)]), P::Data(Float, &[Arg(0)])]),
    Sig::new("glAreTexturesResident", R::Bool, &[P::S(Sizei), P::Data(Uint, &[Arg(0)]), P::OutAny(Bool)]),
    // selection and feedback (§5.2, §5.3): the buffer's SIZE crosses the ring, never its pointer; glRenderMode returns the value
    // (hits or values written, or the negative overflow) and then the buffer contents (unsigned integers or floats)
    Sig::new("glSelectBuffer", R::Void, &[P::S(Sizei)]),
    Sig::new("glFeedbackBuffer", R::Void, &[P::S(Sizei), P::S(Enum)]),
    Sig::new("glRenderMode", R::Int, &[P::S(Enum), P::OutAny(Uint)]),
];

