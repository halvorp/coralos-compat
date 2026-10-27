//! The argument-word layout of the state calls the worker executes, in ONE place for the encoder (engine) and the
//! decoder (worker): the engine builds `Command::Call { args }` with [`words_for`]'s arity and the worker refuses a frame
//! whose arity differs. A scalar is one word (an integer or enum zero-extended, a float as its `f32` bits, a double as
//! its `f64` bits); a vector argument (`glLightfv`'s `params`) is inlined as `f32` bits, its length decided by `pname`
//! ([`vec_len`]) so the engine and the worker cannot disagree about it. A pointer to client memory never crosses the ring.
//! Texture pixels and read-back pixels do not fit in words: they travel in `Command::CallData` / `Command::ReplyData`.
use alloc::string::String;

/// How a call's arguments are laid out in its words.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Shape {
    /// Exactly this many words.
    Words(usize),
    /// `lead` scalar words, then a vector whose length `pname` (the word at index `pname_at`) decides.
    Vector { lead: usize, pname_at: usize, kind: VecKind },
    /// `lead` scalar words naming a count at word 0, then that many `u32` names (`glDeleteTextures`).
    Names,
}

/// Which parameter table decides a vector's length.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum VecKind {
    Fog,
    LightModel,
    Light,
    Material,
}

/// How the call is answered.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    /// Fire and forget: the next batch end retires it.
    None,
    /// The engine waits for a `Reply` whose words are the result.
    Reply,
}

/// The layout of the state call `name`, or `None` when it is not a generic state call.
pub fn shape(name: &str) -> Option<(Shape, Answer)> {
    use Shape::*;
    Some(match name {
        "glAlphaFunc" | "glBindTexture" | "glBlendFunc" | "glColorMaterial" | "glDepthRange" | "glFogf" | "glFogi" | "glPixelStorei" | "glPolygonOffset" => (Words(2), Answer::None),
        "glClearColor" | "glScissor" => (Words(4), Answer::None),
        "glDepthFunc" | "glDepthMask" | "glDrawBuffer" | "glReadBuffer" => (Words(1), Answer::None),
        "glLightf" | "glTexEnvi" | "glTexGeni" | "glTexParameteri" => (Words(3), Answer::None),
        "glNormal3fv" => (Words(3), Answer::None),
        "glDeleteTextures" => (Names, Answer::None),
        "glFogfv" => (Vector { lead: 1, pname_at: 0, kind: VecKind::Fog }, Answer::None),
        "glLightModelfv" => (Vector { lead: 1, pname_at: 0, kind: VecKind::LightModel }, Answer::None),
        "glLightfv" => (Vector { lead: 2, pname_at: 1, kind: VecKind::Light }, Answer::None),
        "glMaterialfv" => (Vector { lead: 2, pname_at: 1, kind: VecKind::Material }, Answer::None),
        "glGenTextures" | "glGetIntegerv" | "glIsEnabled" => (Words(1), Answer::Reply),
        _ => return None,
    })
}

/// Floats a parameter vector carries for `pname`, or `None` when GL 1.2 defines no such parameter for that call.
pub fn vec_len(kind: VecKind, pname: u32) -> Option<usize> {
    Some(match (kind, pname) {
        // GL_FOG_INDEX 0x0B61, GL_FOG_DENSITY 0x0B62, GL_FOG_START 0x0B63, GL_FOG_END 0x0B64, GL_FOG_MODE 0x0B65, GL_FOG_COLOR 0x0B66
        (VecKind::Fog, 0x0B61..=0x0B65) => 1,
        (VecKind::Fog, 0x0B66) => 4,
        // GL_LIGHT_MODEL_LOCAL_VIEWER 0x0B51, _TWO_SIDE 0x0B52, _AMBIENT 0x0B53, GL_LIGHT_MODEL_COLOR_CONTROL 0x81F8 (1.2)
        (VecKind::LightModel, 0x0B51 | 0x0B52 | 0x81F8) => 1,
        (VecKind::LightModel, 0x0B53) => 4,
        // GL_AMBIENT 0x1200, _DIFFUSE 0x1201, _SPECULAR 0x1202, _POSITION 0x1203, _SPOT_DIRECTION 0x1204, _SPOT_EXPONENT 0x1205,
        // _SPOT_CUTOFF 0x1206, GL_CONSTANT_ATTENUATION 0x1207, _LINEAR 0x1208, _QUADRATIC 0x1209
        (VecKind::Light, 0x1200..=0x1203) => 4,
        (VecKind::Light, 0x1204) => 3,
        (VecKind::Light, 0x1205..=0x1209) => 1,
        // GL_EMISSION 0x1600, GL_SHININESS 0x1601, GL_AMBIENT_AND_DIFFUSE 0x1602, GL_COLOR_INDEXES 0x1603
        (VecKind::Material, 0x1200..=0x1202 | 0x1600 | 0x1602) => 4,
        (VecKind::Material, 0x1601) => 1,
        (VecKind::Material, 0x1603) => 3,
        _ => return None,
    })
}

/// Why a frame's words do not fit the layout (the worker's refusal text, the engine's assertion).
pub fn check_words(name: &str, args: &[u64]) -> Result<(), String> {
    let Some((shape, _)) = shape(name) else { return Err(String::from("not a generic state call")) };
    let need = match shape {
        Shape::Words(n) => n,
        Shape::Names => {
            let n = *args.first().ok_or_else(|| String::from("missing count"))?;
            1usize.checked_add(usize::try_from(n).map_err(|_| String::from("count overflow"))?).ok_or_else(|| String::from("count overflow"))?
        }
        Shape::Vector { lead, pname_at, kind } => {
            let p = *args.get(pname_at).ok_or_else(|| String::from("missing pname"))?;
            let n = u32::try_from(p).ok().and_then(|p| vec_len(kind, p)).ok_or_else(|| alloc::format!("pname {p:#x} is not a parameter of this call"))?;
            lead + n
        }
    };
    if args.len() == need { Ok(()) } else { Err(alloc::format!("{} words, layout needs {need}", args.len())) }
}

/// Integers `glGetIntegerv` returns for `pname`: every `GetPName` value GL 1.0-1.2 defines (the Khronos registry's
/// `GetPName` group restricted to the enums of those three features, 249 values), with its value count. `None`: not a
/// GL 1.2 state query. Values the worker returns are the current state, converted by GL's own rules (booleans 0/1,
/// floats rounded, colours and normals scaled to the full integer range).
pub fn integer_count(pname: u32) -> Option<usize> {
    Some(match pname {
        0x0BA6..=0x0BA8 => 16,
        0x0B00 | 0x0B03 | 0x0B04 | 0x0B06 | 0x0B07 | 0x0B53 | 0x0B66 | 0x0B80 | 0x0BA2 | 0x0C10 | 0x0C22 | 0x0C23 | 0x0DD2 => 4,
        0x0B02 => 3,
        0x0B12 | 0x0B22 | 0x0B40 | 0x0B70 | 0x0D3A | 0x0DD0 | 0x0DD3 | 0x846D | 0x846E => 2,
        0x0B01 | 0x0B05 | 0x0B08..=0x0B09 | 0x0B10..=0x0B11 | 0x0B13 | 0x0B20..=0x0B21 | 0x0B23..=0x0B26
        | 0x0B30..=0x0B33 | 0x0B41..=0x0B46 | 0x0B50..=0x0B52 | 0x0B54..=0x0B57 | 0x0B60..=0x0B65 | 0x0B71..=0x0B74
        | 0x0B90..=0x0B98 | 0x0BA0..=0x0BA1 | 0x0BA3..=0x0BA5 | 0x0BB0..=0x0BB1 | 0x0BC0..=0x0BC2 | 0x0BD0
        | 0x0BE0..=0x0BE2 | 0x0BF0..=0x0BF2 | 0x0C00..=0x0C02 | 0x0C11 | 0x0C20..=0x0C21 | 0x0C30..=0x0C33 | 0x0C40
        | 0x0C50..=0x0C54 | 0x0C60..=0x0C63 | 0x0CB0..=0x0CB9 | 0x0CF0..=0x0CF5 | 0x0D00..=0x0D05 | 0x0D10..=0x0D1F
        | 0x0D30..=0x0D39 | 0x0D3B | 0x0D50..=0x0D5B | 0x0D70 | 0x0D80 | 0x0D90..=0x0D98 | 0x0DB0..=0x0DB8 | 0x0DD1
        | 0x0DE0..=0x0DE1 | 0x0DF1..=0x0DF2 | 0x0DF4 | 0x2A00..=0x2A02 | 0x3000..=0x3005 | 0x4000..=0x4007
        | 0x8037..=0x8038 | 0x8068..=0x806E | 0x8073..=0x807C | 0x807E..=0x807F | 0x8081..=0x8083 | 0x8085..=0x8086
        | 0x8088..=0x808A | 0x808C | 0x80E8..=0x80E9 | 0x81F8 => 1,
        _ => return None,
    })
}

/// `GetPName` values (of the 249) that gl4es keeps no state for, so the worker REFUSES the query by name rather than answer
/// a GL error gl4es invents or a zero it does not know. Measured against the pinned gl4es (2037865590d4) by the worker's
/// host test `every GetPName ... answers`, which fails if this list and gl4es's behaviour ever disagree. Includes index
/// mode (`GL_CURRENT_INDEX`...), the accumulation buffer, evaluators' grids, selection/feedback, line/point/polygon
/// details, and client-array size/type/stride.
pub const INTEGER_PNAMES_UNANSWERED: &[u32] = &[
    0x0B01, 0x0B04, 0x0B05, 0x0B06, 0x0B07, 0x0B08, 0x0B09, 0x0B13, 0x0B22, 0x0B23, 0x0B25, 0x0B26, 0x0B30, 0x0B32, 0x0B33, 0x0B40, 0x0B41, 0x0B51,
    0x0B80, 0x0BF1, 0x0C20, 0x0C21, 0x0C30, 0x0C31, 0x0C33, 0x0C53, 0x0D11, 0x0D1E, 0x0D1F, 0x0D30, 0x0D35, 0x0D3B, 0x0D51, 0x0D58, 0x0D59, 0x0D5A,
    0x0D5B, 0x0DD0, 0x0DD1, 0x0DD2, 0x0DD3, 0x0DF1, 0x0DF2, 0x0DF4, 0x2A01, 0x2A02, 0x806B, 0x807A, 0x807B, 0x807C, 0x807E, 0x807F, 0x8081, 0x8082,
    0x8083, 0x8085, 0x8086, 0x8088, 0x8089, 0x808A, 0x808C, 0x81F8,
];

/// Capabilities `glIsEnabled` cannot answer on gl4es (it raises a GL error): refused by name.
pub const CAPABILITIES_UNANSWERED: &[u32] = &[0x0BF1, 0x2A01, 0x2A02, 0x0B41, 0x8077, 0x8079];

/// The named refusal for a query the worker cannot answer, or `None` when it can.
pub fn refused_query(name: &str, pname: u32) -> Option<alloc::string::String> {
    let no = match name {
        "glGetIntegerv" => INTEGER_PNAMES_UNANSWERED.contains(&pname),
        "glIsEnabled" => CAPABILITIES_UNANSWERED.contains(&pname),
        _ => false,
    };
    no.then(|| alloc::format!("[HEIRLOOM-GL REFUSED call={name} pname={pname:#06x} reason=gl4es-keeps-no-such-state]"))
}

/// The `GetPName` values that are also `glEnable` capabilities: their value is `glIsEnabled`'s (0 or 1).
pub fn is_capability_pname(pname: u32) -> bool {
    matches!(pname, 0x0B10 | 0x0B20 | 0x0B24 | 0x0B41 | 0x0B42 | 0x0B44 | 0x0B50 | 0x0B57 | 0x0B60 | 0x0B71 | 0x0B90 | 0x0BA1 | 0x0BC0 | 0x0BD0 | 0x0BE2 | 0x0BF1 | 0x0BF2
        | 0x0C11 | 0x0C60..=0x0C63 | 0x0D80 | 0x0D90..=0x0D98 | 0x0DB0..=0x0DB8 | 0x0DE0 | 0x0DE1 | 0x2A01 | 0x2A02 | 0x3000..=0x3005 | 0x4000..=0x4007 | 0x8037)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vector_lengths_follow_the_pname() {
        assert_eq!(vec_len(VecKind::Light, 0x1203), Some(4)); // GL_POSITION
        assert_eq!(vec_len(VecKind::Light, 0x1204), Some(3)); // GL_SPOT_DIRECTION
        assert_eq!(vec_len(VecKind::Light, 0x1205), Some(1)); // GL_SPOT_EXPONENT
        assert_eq!(vec_len(VecKind::Material, 0x1601), Some(1));
        assert_eq!(vec_len(VecKind::Fog, 0x0B66), Some(4));
        assert_eq!(vec_len(VecKind::LightModel, 0x0B52), Some(1));
        assert_eq!(vec_len(VecKind::Material, 0x0B66), None);
    }

    #[test]
    fn the_unanswered_lists_are_inside_the_spec_tables_and_sorted_and_the_refusal_names_the_pname() {
        assert!(INTEGER_PNAMES_UNANSWERED.iter().all(|p| integer_count(*p).is_some()), "every refused pname is a real GetPName");
        assert!(INTEGER_PNAMES_UNANSWERED.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(INTEGER_PNAMES_UNANSWERED.len(), 62);
        assert_eq!(refused_query("glGetIntegerv", 0x0B01).as_deref(), Some("[HEIRLOOM-GL REFUSED call=glGetIntegerv pname=0x0b01 reason=gl4es-keeps-no-such-state]"));
        assert_eq!(refused_query("glGetIntegerv", 0x0BA2), None);
        assert!(refused_query("glIsEnabled", 0x0B41).is_some());
        assert_eq!(refused_query("glIsEnabled", 0x0DE1), None);
    }

    #[test]
    fn a_frame_with_the_wrong_number_of_words_is_refused() {
        assert!(check_words("glLightfv", &[0x4000, 0x1203, 1, 2, 3, 4]).is_ok());
        assert!(check_words("glLightfv", &[0x4000, 0x1203, 1, 2, 3]).is_err());
        assert!(check_words("glLightfv", &[0x4000, 0xBEEF, 1]).is_err());
        assert!(check_words("glDeleteTextures", &[2, 7, 8]).is_ok());
        assert!(check_words("glDeleteTextures", &[2, 7]).is_err());
        assert!(check_words("glDeleteTextures", &[u64::MAX]).is_err());
        assert!(check_words("glClearColor", &[0; 4]).is_ok() && check_words("glClearColor", &[0; 3]).is_err());
        assert!(check_words("glBegin", &[]).is_err());
    }
}
