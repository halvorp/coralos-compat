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
/// `None` today, and it stays `None`: `contract::core_tally` says 306 / 30 / 1 of the 306 / 30 / 4 core commands are callable and 268 / 28 / 1
/// are COMPLETE (callable with nothing refused). What stands between each version and a claim is named on the entries
/// (`cargo run -q -p heirloom-gl-abi --example tally` prints it): for 1.0 and 1.1 the values gl4es over GLES 2 cannot render and the
/// worker refuses by name (`gaps::VALUE_GAPS`: flat shading, spot cutoffs, the GL_BLEND texture function, logic operations, smooth and
/// stippled primitives, polygon offset of points and lines, front buffers, selection and feedback, depth and colour-index pixels), the
/// calls the worker cannot record into a display list (`gaps::IN_LIST_GAPS`) and a few named deviations (nested list deletion, evaluators
/// inside a list, `glGetTexImage` of a level other than 0); for 1.2 the three 3D-texture calls (gl4es treats a 3D texture as a 2D one).
/// Set to `Some(Version::V1_1)` or `Some(Version::V1_2)` in the change that brings `core_tally` to `complete == listed` up to that version
/// and has a graded scenario for every entry (the worker's `gpu_families` coverage rule); the test
/// `attested_through_never_exceeds_what_is_complete` fails the build if it is set before that.
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
        assert_eq!(derive(&all_on(), ATTESTED_THROUGH).version, None);
    }
}
