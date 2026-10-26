//! tools/check-fmt-real-corpus.sh, run from the workspace test: every heirloom-fmt-* reader has a test that reads a real
//! file, or is named in the shrink-only baseline. The script's `--selftest` must see a synthetic-only reader go RED.
use std::path::PathBuf;
use std::process::Command;

fn script(args: &[&str]) -> Option<(bool, String)> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = match Command::new("bash").arg(root.join("tools/check-fmt-real-corpus.sh")).args(args).current_dir(&root).output() {
        Ok(o) => o,
        Err(e) => { eprintln!("SKIPPED fmt-real-corpus check: bash is not available ({e})"); return None; }
    };
    Some((out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))))
}

#[test]
fn every_format_reader_reads_a_real_file() {
    if let Some((ok, text)) = script(&[]) { assert!(ok, "{text}"); }
}

#[test]
fn the_corpus_check_catches_a_synthetic_only_reader() {
    if let Some((ok, text)) = script(&["--selftest"]) {
        assert!(ok && text.contains("synthetic-only reader is RED"), "{text}");
    }
}
