//! `GL_VERSION` and `GL_EXTENSIONS`, DERIVED from the contract table. No string here is a literal claim.
use crate::contract::Entry;
use crate::refusal::Refusal;
use crate::version::{Ext, Home, Version};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// The highest version whose Phase 3 qualification has been done: the table lists the COMPLETE core entry set (306 / 30 / 4
/// commands for 1.0 / 1.1 / 1.2, the Khronos registry's lists, 2026-10-27), every listed entry is [`Entry::complete`]
/// (worker-executed, no gap), AND each family has a semantic test that ran through the worker on a GPU
/// (`userland/coral-heirloom-gl/tests/gpu_state.rs`, tools/check-heirloom-gl-host.sh step 5).
///
/// `Some(V1_2)` since the worker runs on Mesa's own desktop OpenGL (2026-10-28): `contract::core_tally` says all 306 / 30 / 4 core
/// commands are COMPLETE (callable with nothing refused, `gaps` empty), and the worker's `gpu_families` scenarios grade every family
/// on a GPU, 3D textures, edge-flag arrays, the front buffer and the values gl4es used to refuse included. The test
/// `attested_through_never_exceeds_what_is_complete` fails the build if a row loses `complete` while this claims its version.
pub const ATTESTED_THROUGH: Option<Version> = Some(Version::V1_2);

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
        if listed == 0 || entries.iter().filter(at_or_below).any(|e| !e.complete()) {
            break;
        }
        version = Some(v);
    }
    let extensions = Ext::ALL
        .into_iter()
        .filter(|x| {
            let mut it = entries.iter().filter(|e| e.home == Home::Ext(*x)).peekable();
            it.peek().is_some() && it.all(|e| e.complete())
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
        ENTRIES.iter().map(|e| Entry { implemented: true, gap: None, ..*e }).collect()
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

    /// The table as it stands: every GL 1.0-1.2 core command and every extension row is complete, so the guest is told
    /// GL 1.2 and its four extensions; without the attestation it is told no version at all.
    #[test]
    fn the_table_claims_gl_1_2_and_four_extensions() {
        let c = derive(ENTRIES, ATTESTED_THROUGH);
        assert_eq!(c.version, Some(Version::V1_2));
        assert_eq!(version_string(&c).as_deref(), Ok("1.2 CoralOS Heirloom GL"));
        assert_eq!(extensions_string(&c), "GL_ARB_multitexture GL_EXT_compiled_vertex_array GL_APPLE_fence GL_APPLE_vertex_array_range");
        assert_eq!(derive(ENTRIES, None).version, None);
    }

    /// An entry that runs but refuses some argument values (`gap`) is callable and never claimable: one gap withdraws the
    /// version that lists it and the extension that lists it, while the call itself is still admitted.
    #[test]
    fn a_gap_withdraws_the_claim_but_not_the_call() {
        let mut e = all_on();
        let i = e.iter().position(|x| x.name == "glBindTexture").unwrap();
        e[i].gap = Some("a test gap");
        assert_eq!(derive(&e, Some(Version::V1_2)).version, Some(Version::V1_0), "glBindTexture is core 1.1: the ladder stops below it");
        assert!(crate::refusal::admit_call(&e, i as u16).is_ok(), "the call itself is admitted");
        let j = e.iter().position(|x| x.name == "glLockArraysEXT").unwrap();
        e[j].gap = Some("a test gap");
        assert!(!extensions_string(&derive(&e, Some(Version::V1_2))).contains("GL_EXT_compiled_vertex_array"));
    }

    #[test]
    fn attested_through_never_exceeds_what_is_complete() {
        assert!(ATTESTED_THROUGH <= derive(ENTRIES, Some(Version::V1_2)).version, "ATTESTED_THROUGH claims a version whose core entries are not all complete");
    }

    #[test]
    fn the_unattested_table_claims_no_version_even_when_all_on() {
        assert_eq!(derive(&all_on(), None).version, None);
    }
}
