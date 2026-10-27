//! ARGUMENT CHECKS the engine makes before a call reaches the worker, for the argument domains GL 1.2 defines and gl4es does not
//! enforce (gl4es answers an out-of-range `glBegin` mode with no error at all). A failed check is the GL error the specification
//! names, raised exactly as GL does: the call has no other effect and is never sent (GL 1.2 §2.5).
use alloc::format;
use alloc::string::String;

const GL_INVALID_ENUM: u32 = 0x0500;
const GL_INVALID_VALUE: u32 = 0x0501;

const GL_INVALID_OPERATION: u32 = 0x0502;

/// One enumerated argument: the call, the argument index and the values GL 1.2 accepts. `absent` are accepted enums naming a
/// buffer THIS visual does not have (it is mono, with no auxiliary buffers): GL_INVALID_OPERATION, not GL_INVALID_ENUM (§4.2.1).
#[derive(Copy, Clone)]
pub struct EnumArg {
    pub call: &'static str,
    pub arg: usize,
    pub allowed: &'static [u32],
    pub absent: &'static [u32],
    pub spec: &'static str,
}

/// GL_FRONT_RIGHT, GL_BACK_RIGHT, GL_RIGHT, GL_AUX0..GL_AUX3: buffers of a stereo or auxiliary-buffer visual.
const NO_SUCH_BUFFER: &[u32] = &[0x0401, 0x0403, 0x0407, 0x0409, 0x040A, 0x040B, 0x040C];

/// The enumerated arguments checked here (§2.6.1 Begin: the ten primitive types).
pub static ENUM_ARGS: &[EnumArg] = &[
    EnumArg { call: "glBegin", arg: 0, allowed: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9], absent: &[], spec: "GL 1.2 §2.6.1: glBegin takes one of the ten primitive types" },
    EnumArg { call: "glNewList", arg: 1, allowed: &[0x1300, 0x1301], absent: &[], spec: "GL 1.2 §5.4: glNewList takes GL_COMPILE or GL_COMPILE_AND_EXECUTE" },
    EnumArg {
        call: "glDrawBuffer",
        arg: 0,
        allowed: &[0, 0x0400, 0x0401, 0x0402, 0x0403, 0x0404, 0x0405, 0x0406, 0x0407, 0x0408, 0x0409, 0x040A, 0x040B, 0x040C],
        absent: NO_SUCH_BUFFER,
        spec: "GL 1.2 §4.2.1: glDrawBuffer takes GL_NONE, a front/back/left/right buffer or GL_AUXi, and a buffer the visual lacks is GL_INVALID_OPERATION",
    },
    EnumArg {
        call: "glReadBuffer",
        arg: 0,
        allowed: &[0x0400, 0x0401, 0x0402, 0x0403, 0x0404, 0x0405, 0x0406, 0x0407, 0x0409, 0x040A, 0x040B, 0x040C],
        absent: NO_SUCH_BUFFER,
        spec: "GL 1.2 §4.3.2: glReadBuffer takes a front/back/left/right buffer or GL_AUXi (not GL_NONE or GL_FRONT_AND_BACK), and a buffer the visual lacks is GL_INVALID_OPERATION",
    },
    EnumArg { call: "glCopyPixels", arg: 4, allowed: &[0x1800, 0x1801, 0x1802], absent: &[0x1802], spec: "GL 1.2 §4.3.3: glCopyPixels copies GL_COLOR, GL_DEPTH or GL_STENCIL; a stencil copy needs a stencil buffer, which this visual lacks (GL_INVALID_OPERATION)" },
    EnumArg { call: "glAccum", arg: 0, allowed: &[0x0100, 0x0101, 0x0102, 0x0103, 0x0104], absent: &[], spec: "GL 1.2 §4.2.4: glAccum takes GL_ACCUM, GL_LOAD, GL_RETURN, GL_ADD or GL_MULT" },
];

/// Arguments (by index) that must be at least 1 (a count of divisions, GL 1.2 §5.1: "if n <= 0, INVALID_VALUE").
pub static POSITIVE_ARGS: &[(&str, &[usize])] = &[("glMapGrid1d", &[0]), ("glMapGrid1f", &[0]), ("glMapGrid2d", &[0, 3]), ("glMapGrid2f", &[0, 3])];

/// Calls that are always GL_INVALID_OPERATION on THIS visual, after their enum is checked: there is no accumulation buffer
/// (§4.2.4: "If there is no accumulation buffer ... Accum generates INVALID_OPERATION").
pub static NO_BUFFER_CALLS: &[(&str, &str)] = &[("glAccum", "this visual has no accumulation buffer (GL 1.2 §4.2.4)")];

/// The GL error `args` of `name` raise, and why, or `None` when every checked argument is in its domain: an enumerated argument
/// outside [`ENUM_ARGS`] is GL_INVALID_ENUM; a NEGATIVE `GLsizei` argument of a family call is GL_INVALID_VALUE (every command of
/// GL 1.2 that takes a size, count or range says so).
pub fn violation(name: &str, args: &[u64]) -> Option<(u32, String)> {
    let enumerated = ENUM_ARGS.iter().filter(|c| c.call == name).find_map(|c| {
        let v = *args.get(c.arg)?;
        if c.absent.iter().any(|a| u64::from(*a) == v) {
            return Some((GL_INVALID_OPERATION, format!("{name} argument {}: {v:#x} names a buffer this visual does not have ({})", c.arg, c.spec)));
        }
        (!c.allowed.iter().any(|a| u64::from(*a) == v)).then(|| (GL_INVALID_ENUM, format!("{name} argument {}: {v:#x} is not accepted ({})", c.arg, c.spec)))
    });
    let enumerated = enumerated.or_else(|| NO_BUFFER_CALLS.iter().find(|(c, _)| *c == name).map(|(_, why)| (GL_INVALID_OPERATION, format!("{name}: {why}"))));
    let enumerated = enumerated.or_else(|| {
        POSITIVE_ARGS.iter().find(|(c, _)| *c == name).and_then(|(_, idx)| idx.iter().find(|i| args.get(**i).is_some_and(|w| (*w as u32 as i32) < 1)).map(|i| (GL_INVALID_VALUE, format!("{name} argument {i}: a division count must be at least 1 (GL 1.2 §5.1)"))))
    });
    enumerated.or_else(|| negative_size(name, args).map(|(k, v)| (GL_INVALID_VALUE, format!("{name} argument {k}: the size {v} is negative (GL 1.2: GL_INVALID_VALUE)"))))
}

fn negative_size(name: &str, args: &[u64]) -> Option<(usize, i32)> {
    use crate::family::{Ty, P};
    let sig = crate::family::lookup(name)?;
    let mut at = 0;
    for p in sig.params {
        match *p {
            P::S(t) => {
                let v = *args.get(at)? as u32 as i32;
                if t == Ty::Sizei && v < 0 {
                    return Some((at, v));
                }
                at += 1;
            }
            // vectors and data come last in every generated call, so no later scalar can follow them
            _ => return None,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_argument_outside_its_domain_is_invalid_enum() {
        assert_eq!(violation("glBegin", &[9]), None);
        assert_eq!(violation("glBegin", &[10]).map(|v| v.0), Some(0x0500));
        assert_eq!(violation("glBegin", &[0xFFFF]).map(|v| v.0), Some(0x0500));
        assert_eq!(violation("glEnd", &[]), None);
        assert_eq!(violation("glBegin", &[]), None);
        assert_eq!(violation("glDeleteLists", &[1, 3]), None);
        assert_eq!(violation("glDeleteLists", &[1, u64::from(-1i32 as u32)]).map(|v| v.0), Some(0x0501));
        assert_eq!(violation("glGenLists", &[u64::from(-5i32 as u32)]).map(|v| v.0), Some(0x0501));
        assert_eq!(violation("glColor3b", &[0, 0, 0]), None);
        assert_eq!(violation("glMapGrid1f", &[1, 0, 0]), None);
        assert_eq!(violation("glMapGrid1f", &[0, 0, 0]).map(|v| v.0), Some(0x0501));
        assert_eq!(violation("glMapGrid2d", &[2, 0, 0, 0, 0, 0]).map(|v| v.0), Some(0x0501), "the v count is checked too");
        assert_eq!(violation("glDrawBuffer", &[0x0405]), None);
        assert_eq!(violation("glDrawBuffer", &[0x0407]).map(|v| v.0), Some(0x0502), "a right buffer is absent, not invalid");
        assert_eq!(violation("glDrawBuffer", &[0x1234]).map(|v| v.0), Some(0x0500));
        assert_eq!(violation("glReadBuffer", &[0]).map(|v| v.0), Some(0x0500));
        assert_eq!(violation("glReadBuffer", &[0x0408]).map(|v| v.0), Some(0x0500));
        assert_eq!(violation("glAccum", &[0x0100, 0]).map(|v| v.0), Some(0x0502), "no accumulation buffer");
        assert_eq!(violation("glAccum", &[0x1234, 0]).map(|v| v.0), Some(0x0500), "an invalid operation enum is reported first");
        assert_eq!(violation("glNewList", &[1, 0x1300]), None);
        assert_eq!(violation("glNewList", &[1, 0x1234]).map(|v| v.0), Some(0x0500));
    }
}
