//! The regular call FAMILIES of GL 1.0-1.2, as one mechanism: the per-vertex calls (`glVertex{2,3,4}{s,i,f,d}[v]`,
//! `glColor{3,4}{b,s,i,f,d,ub,us,ui}[v]`, `glTexCoord*`, `glNormal3*`, `glRasterPos*`, `glRect*`, `glIndex*`,
//! `glEvalCoord*`, `glMultMatrix*`, ...), the plain state setters and the pname-sized parameter vectors and getters.
//! [`GENERATED`] (family_table.rs) is derived from the Khronos registry's `gl.xml` by `tools/gen-gl-sigs.py`: each
//! parameter's C type decides how a call's arguments travel, so there is no per-call hand-written layout to get wrong.
//!
//! Wire layout of one call (`Command::Call { args }`), the same for the encoder (guest engine) and the decoder (worker):
//! one word per scalar (an integer as the 32-bit value its GL type has, sign-extended for the signed types narrower than
//! 32 bits; a float as its `f32` bits; a double as its `f64` bits), then, for a vector parameter, one word per element
//! in the element's own type. A vector's length is a constant of the call (`glVertex3fv`: 3) or is decided by the
//! `pname` scalar before it ([`var_len`], the same table both sides use). An OUT vector (`glGetLightfv`) carries no
//! words in the call: the worker's `Reply` carries its elements, one word each in the element's type, and the engine
//! stores them into the program's memory in the program's byte order.
use crate::sig::VecKind;
use alloc::format;
use alloc::string::String;

/// A GL scalar type, by how it is passed and stored.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Ty {
    Enum,
    Int,
    Uint,
    Sizei,
    Short,
    Ushort,
    Byte,
    Ubyte,
    Bool,
    Bitfield,
    Float,
    Double,
}

impl Ty {
    /// Bytes of one element in guest memory.
    pub const fn guest_bytes(self) -> usize {
        match self {
            Ty::Bool | Ty::Byte | Ty::Ubyte => 1,
            Ty::Short | Ty::Ushort => 2,
            Ty::Double => 8,
            _ => 4,
        }
    }

    /// Words one argument of this type takes in the PowerPC parameter list (a double takes two).
    pub const fn slots(self) -> usize {
        if matches!(self, Ty::Double) { 2 } else { 1 }
    }

    /// Passed in a floating-point register (f1, f2, ...) rather than a general one.
    pub const fn is_float(self) -> bool {
        matches!(self, Ty::Float | Ty::Double)
    }

    /// The wire word of a raw integer (the register, or the memory element zero-extended to 32 bits).
    pub const fn int_word(self, raw: u32) -> u64 {
        match self {
            Ty::Short => (raw as u16 as i16 as i32 as u32) as u64,
            Ty::Ushort => (raw & 0xFFFF) as u64,
            Ty::Byte => (raw as u8 as i8 as i32 as u32) as u64,
            Ty::Ubyte => (raw & 0xFF) as u64,
            Ty::Bool => ((raw & 0xFF) != 0) as u64,
            _ => raw as u64,
        }
    }

    /// The wire word of a float or double value.
    pub fn float_word(self, v: f64) -> u64 {
        if self == Ty::Double { v.to_bits() } else { u64::from((v as f32).to_bits()) }
    }

    /// Whether a received word is a value of this type (the worker refuses a frame that is not).
    pub const fn word_fits(self, w: u64) -> bool {
        match self {
            Ty::Double => true,
            Ty::Short => w <= u32::MAX as u64 && (w as u32 as i32) >= i16::MIN as i32 && (w as u32 as i32) <= i16::MAX as i32,
            Ty::Ushort => w <= 0xFFFF,
            Ty::Byte => w <= u32::MAX as u64 && (w as u32 as i32) >= i8::MIN as i32 && (w as u32 as i32) <= i8::MAX as i32,
            Ty::Ubyte => w <= 0xFF,
            Ty::Bool => w <= 1,
            _ => w <= u32::MAX as u64,
        }
    }

    /// The raw big-endian guest element of `bytes` (exactly [`Ty::guest_bytes`] long) as a wire word.
    pub fn from_guest(self, bytes: &[u8]) -> u64 {
        match self.guest_bytes() {
            1 => self.int_word(u32::from(bytes[0])),
            2 => self.int_word(u32::from(u16::from_be_bytes([bytes[0], bytes[1]]))),
            4 if self == Ty::Float => u64::from(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
            4 => self.int_word(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
            _ => u64::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7]]),
        }
    }

    /// A wire word as the bytes the program's memory holds (big-endian): the engine's store of an OUT vector element.
    pub fn to_guest(self, w: u64, out: &mut alloc::vec::Vec<u8>) {
        match self.guest_bytes() {
            1 => out.push(w as u8),
            2 => out.extend_from_slice(&(w as u16).to_be_bytes()),
            4 => out.extend_from_slice(&(w as u32).to_be_bytes()),
            _ => out.extend_from_slice(&w.to_be_bytes()),
        }
    }
}

/// One parameter.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum P {
    /// A scalar: one word.
    S(Ty),
    /// A pointer to exactly `n` elements the call reads (`glVertex3fv`: 3): `n` words.
    V(Ty, u8),
    /// A pointer to the elements the call reads, as many as the `pname` word before it asks for ([`var_len`]).
    In(Ty, VecKind),
    /// A pointer the call writes; as many elements as the `pname` word before it asks for. No words in the call.
    Out(Ty, VecKind),
    /// Input DATA whose element count is the product of the listed operands (earlier scalar words and constants): the control
    /// points of an evaluator, a list of display-list names. The engine repacks the program's memory (strides, byte order)
    /// into exactly that many words.
    Data(Ty, &'static [Op]),
    /// An output whose length only the worker knows (the coefficients of a stored evaluator map, a pixel map): the reply carries
    /// the elements and the engine stores as many as the reply holds.
    OutAny(Ty),
}

/// An operand of a [`P::Data`] element count.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Op {
    /// The value of the scalar word at this index.
    Arg(u8),
    Const(u32),
}

/// What the call returns (the value travels as one reply word).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum R {
    Void,
    Bool,
    Uint,
    Int,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Sig {
    pub name: &'static str,
    pub ret: R,
    pub params: &'static [P],
}

impl Sig {
    pub const fn new(name: &'static str, ret: R, params: &'static [P]) -> Sig {
        Sig { name, ret, params }
    }

    /// Whether the engine waits for a `Reply`: a returned value or an OUT vector.
    pub fn answers(&self) -> bool {
        self.ret != R::Void || self.params.iter().any(|p| matches!(p, P::Out(..) | P::OutAny(_)))
    }

    /// The OUT vector, if any: its element type and the kind that sizes it.
    pub fn out(&self) -> Option<(Ty, VecKind)> {
        self.params.iter().find_map(|p| if let P::Out(t, k) = p { Some((*t, *k)) } else { None })
    }
}

/// The GENERATED family call for `name`, or `None`.
pub fn lookup(name: &str) -> Option<&'static Sig> {
    crate::family_table::GENERATED.iter().find(|s| s.name == name)
}

/// The hand-written bulk-data call for `name` (`bulk_table`), or `None`.
pub fn lookup_bulk(name: &str) -> Option<&'static Sig> {
    crate::bulk_table::BULK.iter().find(|s| s.name == name)
}

/// Either table.
pub fn lookup_any(name: &str) -> Option<&'static Sig> {
    lookup(name).or_else(|| lookup_bulk(name))
}

/// The element count of a [`P::Data`] for the call's scalar words so far (`None`: an operand is not a scalar word, or the product
/// overflows `usize`).
pub fn data_len(ops: &[Op], args: &[u64]) -> Option<usize> {
    let mut n: usize = 1;
    for op in ops {
        let v = match *op {
            Op::Const(c) => usize::try_from(c).ok()?,
            Op::Arg(i) => usize::try_from(*args.get(usize::from(i))? as u32).ok()?,
        };
        n = n.checked_mul(v)?;
    }
    Some(n)
}

/// Elements a pname-sized vector holds for `pname`, or `None` when GL 1.2 defines no such parameter for the call.
/// Extends [`crate::sig::vec_len`] with the kinds only the generated families use.
pub fn var_len(kind: VecKind, pname: u32) -> Option<usize> {
    crate::sig::vec_len(kind, pname)
}

/// Why `args` does not fit the layout of `sig` (the worker's refusal text, the engine's assertion).
pub fn check_words(sig: &Sig, args: &[u64]) -> Result<(), String> {
    let mut at = 0usize;
    for p in sig.params {
        match *p {
            P::S(t) => {
                let w = *args.get(at).ok_or_else(|| String::from("too few words"))?;
                if !t.word_fits(w) {
                    return Err(format!("word {at} ({w:#x}) is not a {t:?}"));
                }
                at += 1;
            }
            P::V(t, n) => {
                for k in 0..usize::from(n) {
                    let w = *args.get(at + k).ok_or_else(|| String::from("too few words"))?;
                    if !t.word_fits(w) {
                        return Err(format!("word {} ({w:#x}) is not a {t:?}", at + k));
                    }
                }
                at += usize::from(n);
            }
            P::In(t, kind) => {
                let pname = at.checked_sub(1).and_then(|i| args.get(i)).copied().and_then(|w| u32::try_from(w).ok()).ok_or_else(|| String::from("no pname before the vector"))?;
                let n = var_len(kind, pname).ok_or_else(|| format!("pname {pname:#x} is not a parameter of this call"))?;
                for k in 0..n {
                    let w = *args.get(at + k).ok_or_else(|| String::from("too few words"))?;
                    if !t.word_fits(w) {
                        return Err(format!("word {} ({w:#x}) is not a {t:?}", at + k));
                    }
                }
                at += n;
            }
            P::Out(_, kind) => {
                let pname = at.checked_sub(1).and_then(|i| args.get(i)).copied().and_then(|w| u32::try_from(w).ok()).ok_or_else(|| String::from("no pname before the vector"))?;
                var_len(kind, pname).ok_or_else(|| format!("pname {pname:#x} is not a parameter of this call"))?;
            }
            P::OutAny(_) => {}
            P::Data(t, ops) => {
                // the scalars are the first words (a data section may follow another one: its operands still name scalar words)
                let n = data_len(ops, args).ok_or_else(|| String::from("a data count is not a scalar word or overflows"))?;
                for k in 0..n {
                    let w = *args.get(at + k).ok_or_else(|| String::from("too few words"))?;
                    if !t.word_fits(w) {
                        return Err(format!("word {} ({w:#x}) is not a {t:?}", at + k));
                    }
                }
                at += n;
            }
        }
    }
    if at == args.len() { Ok(()) } else { Err(format!("{} words, layout needs {at}", args.len())) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_generated_and_bulk_call_is_a_core_contract_entry_named_once_with_no_hand_layout_beside_it() {
        use crate::contract::{find, ENTRIES};
        use crate::version::Home;
        let mut seen = alloc::collections::BTreeSet::new();
        for s in crate::family_table::GENERATED.iter().chain(crate::bulk_table::BULK) {
            assert!(seen.insert(s.name), "{} is in the tables twice", s.name);
            let i = find(ENTRIES, s.name).unwrap_or_else(|| panic!("{} is not a contract entry", s.name));
            assert!(matches!(ENTRIES[i].home, Home::Core(_)), "{} is not core", s.name);
            assert!(crate::sig::shape(s.name).is_none(), "{} has a hand-written layout (sig::shape) beside its table row", s.name);
            // a pname-sized vector is always preceded by the scalar that sizes it
            for (k, p) in s.params.iter().enumerate() {
                if matches!(p, P::In(..) | P::Out(..)) {
                    assert!(k > 0 && matches!(s.params[k - 1], P::S(_)), "{}: a sized vector needs its pname before it", s.name);
                }
            }
        }
        assert_eq!(crate::family_table::GENERATED.len(), 259);
    }

    #[test]
    fn check_words_follows_the_layout_of_scalars_vectors_and_data() {
        let l = lookup("glLightiv").unwrap();
        assert!(check_words(l, &[0x4000, 0x1203, 1, 2, 3, 4]).is_ok());
        assert!(check_words(l, &[0x4000, 0x1203, 1, 2, 3]).is_err(), "GL_POSITION is four values");
        assert!(check_words(l, &[0x4000, 0xBEEF, 1]).is_err(), "not a light parameter");
        let c = lookup("glColor3b").unwrap();
        assert!(check_words(c, &[0xFFFF_FF80, 0, 0x7F]).is_ok());
        assert!(check_words(c, &[0x80, 0, 0]).is_err(), "0x80 is not a signed byte: the engine sign-extends");
        let m = lookup_bulk("glMap1f").unwrap();
        assert!(check_words(m, &[0x0D97, 0, 0, 2, 3, 1, 2, 3, 4, 5, 6]).is_ok(), "order 2 x 3 components");
        assert!(check_words(m, &[0x0D97, 0, 0, 2, 3, 1, 2, 3, 4, 5]).is_err(), "a point short");
        let g = lookup("glGetLightfv").unwrap();
        assert!(check_words(g, &[0x4000, 0x1203]).is_ok() && g.answers() && g.out().is_some());
        assert_eq!(data_len(&[Op::Arg(0), Op::Const(3)], &[5]), Some(15));
        assert_eq!(data_len(&[Op::Arg(0), Op::Arg(1)], &[u64::from(u32::MAX), u64::from(u32::MAX), 0, 0]), data_len(&[Op::Arg(0), Op::Arg(1)], &[u64::from(u32::MAX), u64::from(u32::MAX)]));
    }

    #[test]
    fn narrow_integers_are_sign_or_zero_extended_by_their_type() {
        assert_eq!(Ty::Byte.int_word(0xFF), 0xFFFF_FFFF);
        assert_eq!(Ty::Ubyte.int_word(0x1FF), 0xFF);
        assert_eq!(Ty::Short.int_word(0x8000), 0xFFFF_8000);
        assert_eq!(Ty::Ushort.int_word(0x1_8000), 0x8000);
        assert_eq!(Ty::Bool.int_word(0x100), 0);
        assert_eq!(Ty::Bool.int_word(2), 1);
        assert_eq!(Ty::Int.int_word(0x8000_0000), 0x8000_0000);
    }

    #[test]
    fn a_word_that_is_not_a_value_of_its_type_does_not_fit() {
        assert!(Ty::Byte.word_fits(0xFFFF_FF80) && !Ty::Byte.word_fits(0x80) && !Ty::Byte.word_fits(0xFF));
        assert!(Ty::Short.word_fits(0xFFFF_8000) && !Ty::Short.word_fits(0x8000));
        assert!(Ty::Ubyte.word_fits(255) && !Ty::Ubyte.word_fits(256));
        assert!(Ty::Bool.word_fits(1) && !Ty::Bool.word_fits(2));
        assert!(Ty::Int.word_fits(u64::from(u32::MAX)) && !Ty::Int.word_fits(1 << 32));
        assert!(Ty::Double.word_fits(u64::MAX));
    }

    #[test]
    fn guest_elements_round_trip_through_the_wire_word() {
        for (t, bytes) in [(Ty::Short, vec![0xFF, 0x38]), (Ty::Byte, vec![0x9C]), (Ty::Float, vec![0x3F, 0x80, 0, 0]), (Ty::Double, 1.5f64.to_be_bytes().to_vec()), (Ty::Uint, vec![0xDE, 0xAD, 0xBE, 0xEF])] {
            let w = t.from_guest(&bytes);
            assert!(t.word_fits(w), "{t:?}");
            let mut out = alloc::vec::Vec::new();
            t.to_guest(w, &mut out);
            assert_eq!(out, bytes, "{t:?}");
        }
    }
}
