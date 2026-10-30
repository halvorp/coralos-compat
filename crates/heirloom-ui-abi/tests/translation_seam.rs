//! Ruling 17's two mechanical checks, run from the workspace test: engines link only the seam and the pure crates, and no
//! crate names an application outside `src/bindings/`. The script's own `--selftest` plants a violation of each kind and
//! requires RED, so a check that stopped checking fails here too.
use std::path::PathBuf;
use std::process::Command;

fn script(args: &[&str]) -> Option<(bool, String)> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = match Command::new("bash").arg(root.join("tools/check-translation-seam.sh")).args(args).output() {
        Ok(o) => o,
        Err(e) => { eprintln!("SKIPPED translation-seam check: bash is not available ({e})"); return None; }
    };
    Some((out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))))
}

#[test]
fn the_tree_obeys_the_translation_seam() {
    if let Some((ok, text)) = script(&[]) { assert!(ok, "{text}"); }
}

#[test]
fn the_seam_check_catches_what_it_claims_to() {
    if let Some((ok, text)) = script(&["--selftest"]) {
        assert!(ok, "{text}");
        for must in ["planted gadget-library dependency is RED", "planted app name is RED", "bindings/ and in comments is allowed", "planted face in an engine is RED", "a face in an engine's bindings/ is RED (bindings are data)", "the face in the binding interpreter and in a comment is allowed"] {
            assert!(text.contains(must), "selftest did not exercise: {must}\n{text}");
        }
    }
}

/// Negative control for the harness itself: an unknown flag is a usage error, not a silent pass.
#[test]
fn an_unknown_flag_is_refused() {
    if let Some((ok, _)) = script(&["--nonsense"]) { assert!(!ok); }
}
