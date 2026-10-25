//! `GL_VERSION` and `GL_EXTENSIONS`, DERIVED from the contract table. No string here is a literal claim.
use crate::contract::Entry;
use crate::refusal::Refusal;
use crate::version::{Ext, Home, Version};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// The highest version for which the table lists the COMPLETE core entry set. `None` until the per-version lists are
/// complete (Phase 3, semantic qualification): the Phase 0 table is the import census plus the GL 1.2 additions, which
/// is priority, not a capability claim (Astra 2026-10-24), so no core version may be advertised on it.
pub const ATTESTED_THROUGH: Option<Version> = None;

/// What the worker may truthfully tell the guest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capabilities {
    /// Highest version whose every listed core entry is implemented (and which is attested); `None`: claim nothing.
    pub version: Option<Version>,
    /// Extensions whose every listed entry is implemented, in [`Ext`] order.
    pub extensions: Vec<Ext>,
}

/// Derive the claim from `entries`. `attested_through` bounds the version (see [`ATTESTED_THROUGH`]).
pub fn derive(entries: &[Entry], attested_through: Option<Version>) -> Capabilities {
    let mut version = None;
    for v in Version::ALL {
        if attested_through.map_or(true, |a| v > a) {
            break;
        }
        let at_or_below = |e: &&Entry| matches!(e.home, Home::Core(w) if w <= v);
        let listed = entries.iter().filter(at_or_below).count();
        if listed == 0 || entries.iter().filter(at_or_below).any(|e| !e.implemented) {
            break;
        }
        version = Some(v);
    }
    let extensions = Ext::ALL
        .into_iter()
        .filter(|x| {
            let mut it = entries.iter().filter(|e| e.home == Home::Ext(*x)).peekable();
            it.peek().is_some() && it.all(|e| e.implemented)
        })
        .collect();
    Capabilities { version, extensions }
}

/// The `GL_VERSION` string, or a named refusal when nothing can be claimed.
pub fn version_string(c: &Capabilities) -> Result<String, Refusal> {
    let v = c.version.ok_or(Refusal::NoClaimableVersion)?;
    let (maj, min) = v.pair();
    Ok(format!("{maj}.{min} CoralOS Heirloom GL"))
}

/// The `GL_EXTENSIONS` string: space-separated, nothing else.
pub fn extensions_string(c: &Capabilities) -> String {
    let mut s = String::new();
    for (i, x) in c.extensions.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        s.push_str(x.name());
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::ENTRIES;

    fn all_on() -> Vec<Entry> {
        ENTRIES.iter().map(|e| Entry { implemented: true, ..*e }).collect()
    }

    #[test]
    fn nothing_implemented_claims_nothing() {
        let none: Vec<Entry> = ENTRIES.iter().map(|x| Entry { implemented: false, ..*x }).collect();
        let c = derive(&none, Some(Version::V1_2));
        assert_eq!(c.version, None);
        assert!(c.extensions.is_empty());
        assert_eq!(version_string(&c), Err(Refusal::NoClaimableVersion));
        assert_eq!(extensions_string(&c), "");
    }

    #[test]
    fn everything_implemented_claims_the_ladder_and_all_extensions() {
        let c = derive(&all_on(), Some(Version::V1_2));
        assert_eq!(c.version, Some(Version::V1_2));
        assert_eq!(version_string(&c).as_deref(), Ok("1.2 CoralOS Heirloom GL"));
        assert_eq!(extensions_string(&c), "GL_ARB_multitexture GL_EXT_compiled_vertex_array GL_APPLE_fence GL_APPLE_vertex_array_range");
    }

    /// The Phase 1 table: every extension row is switched on, no core version is (the core lists are the census, not the
    /// full per-version sets), so the guest is told its four extensions and NO version.
    #[test]
    fn the_phase_1_table_claims_four_extensions_and_no_version() {
        let c = derive(ENTRIES, ATTESTED_THROUGH);
        assert_eq!(c.version, None);
        assert_eq!(version_string(&c), Err(Refusal::NoClaimableVersion));
        assert_eq!(extensions_string(&c), "GL_ARB_multitexture GL_EXT_compiled_vertex_array GL_APPLE_fence GL_APPLE_vertex_array_range");
        // and the same table attested through 1.2 still claims no version: glTexSubImage2D etc. are not implemented
        assert_eq!(derive(ENTRIES, Some(Version::V1_2)).version, None);
    }

    #[test]
    fn the_unattested_table_claims_no_version_even_when_all_on() {
        assert_eq!(derive(&all_on(), ATTESTED_THROUGH).version, None);
    }
}
