//! ARGUMENT VALUES a callable entry refuses by name, because the stack under the worker cannot do what GL 1.2 says the value
//! does. Each row is a [`crate::contract::Entry::gap`]: the entry runs for every other value, a version or extension that lists
//! it is NOT claimable, and the value is refused where it is first seen (the engine stops the program with the marker; the
//! worker refuses the frame) - never silently rendered as something else.
//!
//! EMPTY since the worker runs on Mesa's own desktop OpenGL (doctrine CORAL-GL-IS-ONE-MESA-WORKER): every value gl4es over
//! GLES 2 could not render (flat shading, spot cutoffs, GL_BLEND texturing, logic ops, smooth and stippled primitives,
//! polygon offset of points and lines, 3D textures, selection and feedback, depth and colour-index pixels, texture levels)
//! is Mesa's, and so is every call inside a display list. A value a future worker cannot serve is added here, with its test.
use alloc::format;
use alloc::string::String;

/// One refused argument value: `refuses` judges the call's words.
#[derive(Copy, Clone)]
pub struct ValueGap {
    pub call: &'static str,
    /// When `Some(v)`: the row refuses exactly the calls whose first word is `v` (a capability, a client-state name).
    pub eq0: Option<u64>,
    pub refuses: fn(&[u64]) -> bool,
    pub why: &'static str,
}

/// Every refused argument value. A row's call must be a `partial` entry in the contract (a test pins this).
pub static VALUE_GAPS: &[ValueGap] = &[];

/// Calls the worker cannot record into a display list: while a list is being compiled (between `glNewList` and
/// `glEndList`) they would be refused by name rather than applied at once.
pub static IN_LIST_GAPS: &[(&str, &str)] = &[];

/// The named refusal of `name` called while a display list is being compiled, or `None`.
pub fn refused_in_list(name: &str) -> Option<String> {
    IN_LIST_GAPS.iter().find(|(c, _)| *c == name).map(|(_, why)| format!("[HEIRLOOM-GL REFUSED call={name} reason=inside-a-display-list: {why}]"))
}

/// The named refusal for `args` of `name`, or `None` when the call is served.
pub fn refused_value(name: &str, args: &[u64]) -> Option<String> {
    VALUE_GAPS.iter().find(|g| g.call == name && g.eq0.map_or_else(|| (g.refuses)(args), |v| args.first() == Some(&v))).map(|g| format!("[HEIRLOOM-GL REFUSED call={name} reason={}]", g.why))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn every_refused_value_belongs_to_a_callable_entry_that_is_not_complete() {
        for g in VALUE_GAPS {
            let e = crate::contract::find(crate::contract::ENTRIES, g.call).map(|i| crate::contract::ENTRIES[i]).expect("a contract entry");
            assert!(e.implemented && !e.complete(), "{}: a value gap makes the entry callable but never complete", g.call);
        }
        for (c, _) in IN_LIST_GAPS {
            let e = crate::contract::find(crate::contract::ENTRIES, c).map(|i| crate::contract::ENTRIES[i]).expect("a contract entry");
            assert!(e.implemented && !e.complete(), "{c}");
        }
    }

    #[test]
    fn the_values_gl4es_refused_are_served() {
        let f = |v: f32| u64::from(v.to_bits());
        for (call, args) in [
            ("glShadeModel", vec![0x1D00]),
            ("glLightf", vec![0x4000, 0x1206, f(10.0)]),
            ("glEnable", vec![0x0BF2]),
            ("glEnable", vec![0x806F]),
            ("glEnableClientState", vec![0x8079]),
            ("glDrawBuffer", vec![0x0404]),
            ("glRenderMode", vec![0x1C02]),
            ("glDrawPixels", vec![0x1902]),
            ("glTexEnvi", vec![0x2300, 0x2200, 0x0BE2]),
        ] {
            assert_eq!(refused_value(call, &args), None, "{call} {args:?}");
        }
        assert_eq!(refused_in_list("glDrawPixels"), None);
    }
}
