//! Window-definition bindings (coralos doctrine HEIRLOOM-CUSTOM-WINDOW-NEEDS-A-BINDING): DATA that names what ONE exact build of a
//! program's own window definition MEANS, so its window can be presented by the native frame of that meaning while the definition's
//! own code still runs for the window's regions and hit tests.
//!
//! A binding is keyed by what the definition IS ([`WindowDefIdentity`]), never by a program's or file's name: a definition passed as a
//! PROCEDURE (Carbon `CreateCustomWindow`, `kWindowDefProcPtr`) is the routine at `entry_offset` in a code section of exactly
//! `code_length` bytes whose FNV-1a 64 digest is `code_fnv1a64`, as the Code Fragment Manager placed it from its container. Another
//! build of the same library has another digest and stays refused. The binding file (the one s-expression grammar):
//!
//! ```text
//! (window-definition (id "<library>-<version>")
//!   (fragment-code (length 439296) (fnv1a64 "0x0123456789abcdef") (entry-offset 12345))
//!   (meaning (role dialog) (titled true) (movable true) (closable false) (resizable false)))
//! ```
//! This crate parses no text (the host reads the file with its s-expression reader): [`WindowDefBinding`] is the value both agree on.
use alloc::format;
use alloc::string::String;
use heirloom_ui_abi::WindowRole;

/// What a window definition IS.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowDefIdentity {
    /// A routine in an instantiated code fragment: its code section's length and digest, the routine's offset in that section.
    FragmentCode { code_length: u32, code_fnv1a64: u64, entry_offset: u32 },
}

/// What the binding says the definition's window means: the native frame it is presented with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowMeaning { pub role: WindowRole, pub titled: bool, pub movable: bool, pub closable: bool, pub resizable: bool }

/// One binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowDefBinding { pub id: String, pub identity: WindowDefIdentity, pub meaning: WindowMeaning }

/// FNV-1a, 64 bits (the digest every binding identity uses).
pub fn fnv1a64(b: &[u8]) -> u64 {
    b.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &x| (h ^ u64::from(x)).wrapping_mul(0x0000_0100_0000_01b3))
}

impl core::fmt::Display for WindowDefIdentity {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::FragmentCode { code_length, code_fnv1a64, entry_offset } =>
                write!(f, "(fragment-code (length {code_length}) (fnv1a64 \"{code_fnv1a64:#018x}\") (entry-offset {entry_offset}))"),
        }
    }
}

/// The binding that names `seen`, or the reason none does (said as the refusal; it carries the identity so the next binding can be
/// written from it).
pub fn find<'a>(bindings: &'a [WindowDefBinding], seen: &WindowDefIdentity) -> Result<&'a WindowDefBinding, String> {
    bindings.iter().find(|b| b.identity == *seen)
        .ok_or_else(|| format!("no window-definition binding names {seen} ({} binding(s) loaded)", bindings.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(len: u32, sum: u64, off: u32) -> WindowDefBinding {
        WindowDefBinding { id: "lib-1".into(), identity: WindowDefIdentity::FragmentCode { code_length: len, code_fnv1a64: sum, entry_offset: off },
            meaning: WindowMeaning { role: WindowRole::Dialog, titled: true, movable: true, closable: false, resizable: false } }
    }

    #[test]
    fn only_the_exact_identity_matches() {
        let code = [0x7Cu8, 0x08, 0x02, 0xA6, 0x4E, 0x80, 0x00, 0x20];
        let sum = fnv1a64(&code);
        let table = [b(8, sum, 4)];
        let seen = WindowDefIdentity::FragmentCode { code_length: 8, code_fnv1a64: sum, entry_offset: 4 };
        assert_eq!(find(&table, &seen).unwrap().id, "lib-1");
        // NEGATIVE CONTROLS: one byte changed, another entry, another length
        let mut other = code; other[7] ^= 1;
        let e = find(&table, &WindowDefIdentity::FragmentCode { code_length: 8, code_fnv1a64: fnv1a64(&other), entry_offset: 4 }).unwrap_err();
        assert!(e.contains("no window-definition binding names (fragment-code (length 8)"), "{e}");
        assert!(find(&table, &WindowDefIdentity::FragmentCode { code_length: 8, code_fnv1a64: sum, entry_offset: 0 }).is_err());
        assert!(find(&table, &WindowDefIdentity::FragmentCode { code_length: 12, code_fnv1a64: sum, entry_offset: 4 }).is_err());
    }
}
