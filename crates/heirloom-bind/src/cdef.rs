//! Control-definition bindings (coralos doctrine HEIRLOOM-APP-CHROME-NEEDS-A-BINDING): DATA that names what ONE exact build of a
//! program's own Mac Control Manager definition (`CDEF` resource) MEANS, so its controls can be presented by the native kit part of
//! that meaning instead of falling back to the content plane.
//!
//! A binding is keyed strictly by what the `CDEF` resource IS ([`ControlDefIdentity`]), never by a program's or file's name:
//! resource ID, exact byte length, and FNV-1a 64 digest of the resource's bytes. Any other version or altered resource stays unbound.
//!
//! ```text
//! (control-definition (id "<program>-<version>-cdef<n>")
//!   (resource (id 2) (length 1824) (fnv1a64 "0x6cc07d31998bde0d"))
//!   (meaning scrollbar))
//! ```
use alloc::string::String;

/// What a `CDEF` resource IS.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlDefIdentity {
    /// A `CDEF` resource in the guest's resource chain: its resource ID, exact byte length, and FNV-1a 64 digest.
    Resource { res_id: i16, length: u32, fnv1a64: u64 },
}

/// What standard Control Manager role the custom `CDEF` implements.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlDefKind {
    PushButton,
    CheckBox,
    RadioButton,
    ScrollBar,
}

/// One control-definition binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlDefBinding {
    pub id: String,
    pub identity: ControlDefIdentity,
    pub kind: ControlDefKind,
}

impl core::fmt::Display for ControlDefIdentity {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Resource { res_id, length, fnv1a64 } => {
                write!(f, "(resource (id {res_id}) (length {length}) (fnv1a64 \"{fnv1a64:#018x}\"))")
            }
        }
    }
}

/// The binding that names `seen`, if any.
pub fn find<'a>(bindings: &'a [ControlDefBinding], seen: &ControlDefIdentity) -> Option<&'a ControlDefBinding> {
    bindings.iter().find(|b| b.identity == *seen)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wdef::fnv1a64;

    #[test]
    fn only_the_exact_cdef_identity_matches() {
        let bytes = [0x4Eu8, 0x56, 0x00, 0x00, 0x4E, 0x5E, 0x4E, 0x75];
        let sum = fnv1a64(&bytes);
        let b = ControlDefBinding {
            id: "test-cdef2".into(),
            identity: ControlDefIdentity::Resource { res_id: 2, length: 8, fnv1a64: sum },
            kind: ControlDefKind::ScrollBar,
        };
        let table = [b];
        let hit = find(&table, &ControlDefIdentity::Resource { res_id: 2, length: 8, fnv1a64: sum });
        assert_eq!(hit.map(|x| (x.id.as_str(), x.kind)), Some(("test-cdef2", ControlDefKind::ScrollBar)));
        // NEGATIVE CONTROLS: different id, length, or one flipped byte
        let mut altered = bytes;
        altered[7] ^= 1;
        assert!(find(&table, &ControlDefIdentity::Resource { res_id: 3, length: 8, fnv1a64: sum }).is_none());
        assert!(find(&table, &ControlDefIdentity::Resource { res_id: 2, length: 9, fnv1a64: sum }).is_none());
        assert!(find(&table, &ControlDefIdentity::Resource { res_id: 2, length: 8, fnv1a64: fnv1a64(&altered) }).is_none());
    }
}
