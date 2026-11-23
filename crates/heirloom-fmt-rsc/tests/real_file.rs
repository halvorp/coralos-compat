//! Real-file corpus test for `heirloom-fmt-rsc`: parses a real Symbian `.rsc` file named by `RSC_CORPUS`
//! (a single `.rsc` file or a directory searched recursively for `.rsc` files), skipping loudly when unset.
use heirloom_fmt_rsc::RscFile;
use std::path::{Path, PathBuf};

fn collect_rsc(p: &Path, out: &mut Vec<PathBuf>) {
    if p.is_file() {
        out.push(p.to_path_buf());
    } else if let Ok(rd) = std::fs::read_dir(p) {
        for e in rd.flatten() {
            let path = e.path();
            if path.is_dir() {
                collect_rsc(&path, out);
            } else if path.extension().and_then(|s| s.to_str()).is_some_and(|s| s.eq_ignore_ascii_case("rsc")) {
                out.push(path);
            }
        }
    }
}

#[test]
fn parses_real_symbian_rsc_corpus() {
    let Ok(val) = std::env::var("RSC_CORPUS") else {
        eprintln!("SKIPPED heirloom-fmt-rsc real_file test: RSC_CORPUS is unset");
        return;
    };
    let mut files = Vec::new();
    collect_rsc(Path::new(&val), &mut files);
    assert!(!files.is_empty(), "RSC_CORPUS={val:?} contained no .rsc files");
    let mut parsed_plain = 0usize;
    for path in &files {
        let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        match RscFile::parse(&bytes) {
            Ok(rsc) => {
                assert!(rsc.count() > 0, "{}: plain .rsc parsed with 0 resources", path.display());
                // Read resource 1 to verify run expansion on real bytes.
                let first = rsc.resource(1).unwrap_or_else(|e| panic!("{} resource 1: {e:?}", path.display()));
                assert!(!first.is_empty(), "{}: resource 1 is empty", path.display());
                parsed_plain += 1;
            }
            Err(heirloom_fmt_rsc::RscError::Unsupported(_)) => {
                // Dictionary-compressed or flag-0x20/0x40/0x80 RSC files are intentionally refused by name.
            }
            Err(e) => panic!("{}: unexpected parse error {e:?}", path.display()),
        }
    }
    assert!(parsed_plain > 0, "RSC_CORPUS={val:?} had {} files but none were plain UID1 0x101F4A6B resources", files.len());
}
