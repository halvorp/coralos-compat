//! Differential vectors: random valid ARMv6 encodings with random operands, flags and FPSCR, checked
//! against Unicorn's ARM1176 model (tools/fuzz_vectors.py). One test per instruction-group template.

mod common;

fn run(what: &str, text: &str) {
    common::run_vectors(text, &common::reviewed_unpredictable()).assert_clean(what);
}

#[test]
fn fuzz_branch() {
    run("fuzz branch", include_str!("data/fuzz_branch.vec"));
}

#[test]
fn fuzz_cp15() {
    run("fuzz cp15", include_str!("data/fuzz_cp15.vec"));
}

#[test]
fn fuzz_dp_imm() {
    run("fuzz dp_imm", include_str!("data/fuzz_dp_imm.vec"));
}

#[test]
fn fuzz_dp_shift_imm() {
    run("fuzz dp_shift_imm", include_str!("data/fuzz_dp_shift_imm.vec"));
}

#[test]
fn fuzz_dp_shift_reg() {
    run("fuzz dp_shift_reg", include_str!("data/fuzz_dp_shift_reg.vec"));
}

#[test]
fn fuzz_halfword_multiply() {
    run("fuzz halfword_multiply", include_str!("data/fuzz_halfword_multiply.vec"));
}

#[test]
fn fuzz_load_store() {
    run("fuzz load_store", include_str!("data/fuzz_load_store.vec"));
}

#[test]
fn fuzz_load_store_multiple() {
    run("fuzz load_store_multiple", include_str!("data/fuzz_load_store_multiple.vec"));
}

#[test]
fn fuzz_media_misc() {
    run("fuzz media_misc", include_str!("data/fuzz_media_misc.vec"));
}

#[test]
fn fuzz_misc_arith() {
    run("fuzz misc_arith", include_str!("data/fuzz_misc_arith.vec"));
}

#[test]
fn fuzz_multiply() {
    run("fuzz multiply", include_str!("data/fuzz_multiply.vec"));
}

#[test]
fn fuzz_parallel() {
    run("fuzz parallel", include_str!("data/fuzz_parallel.vec"));
}

#[test]
fn fuzz_setend() {
    run("fuzz setend", include_str!("data/fuzz_setend.vec"));
}

#[test]
fn fuzz_status() {
    run("fuzz status", include_str!("data/fuzz_status.vec"));
}

#[test]
fn fuzz_swp_exclusive() {
    run("fuzz swp_exclusive", include_str!("data/fuzz_swp_exclusive.vec"));
}

#[test]
fn fuzz_thumb_alu() {
    run("fuzz thumb_alu", include_str!("data/fuzz_thumb_alu.vec"));
}

#[test]
fn fuzz_thumb_branch() {
    run("fuzz thumb_branch", include_str!("data/fuzz_thumb_branch.vec"));
}

#[test]
fn fuzz_thumb_memory() {
    run("fuzz thumb_memory", include_str!("data/fuzz_thumb_memory.vec"));
}

#[test]
fn fuzz_thumb_misc() {
    run("fuzz thumb_misc", include_str!("data/fuzz_thumb_misc.vec"));
}

#[test]
fn fuzz_vfp_arith() {
    run("fuzz vfp_arith", include_str!("data/fuzz_vfp_arith.vec"));
}

#[test]
fn fuzz_vfp_compare_convert() {
    run("fuzz vfp_compare_convert", include_str!("data/fuzz_vfp_compare_convert.vec"));
}

#[test]
fn fuzz_vfp_transfer() {
    run("fuzz vfp_transfer", include_str!("data/fuzz_vfp_transfer.vec"));
}

#[test]
fn fuzz_vfp_vector() {
    run("fuzz vfp_vector", include_str!("data/fuzz_vfp_vector.vec"));
}

/// A larger run over vectors generated outside the tree: `tools/fuzz_vectors.py <dir> 5000`, then
/// `HEIRLOOM_ARM_SOAK=<dir> cargo test -p heirloom-cpu-arm --test fuzz_vectors -- --ignored`.
#[test]
#[ignore = "soak run: needs HEIRLOOM_ARM_SOAK=<directory of .vec files>"]
fn soak_directory() {
    let dir = std::env::var("HEIRLOOM_ARM_SOAK").expect("HEIRLOOM_ARM_SOAK");
    let mut names: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
    names.sort();
    let reviewed = common::reviewed_unpredictable();
    let mut failed = 0;
    for path in names.iter().filter(|p| p.extension().is_some_and(|e| e == "vec")) {
        let report = common::run_vectors(&std::fs::read_to_string(path).unwrap(), &reviewed);
        failed += report.failures.len();
        let what = path.file_name().unwrap().to_string_lossy().into_owned();
        if report.failures.is_empty() {
            report.assert_clean(&what);
        } else {
            eprintln!("{what}: {} of {} failed", report.failures.len(), report.total());
            let shown = if std::env::var_os("HEIRLOOM_ARM_SOAK_VERBOSE").is_some() { usize::MAX } else { 5 };
            for f in report.failures.iter().take(shown) {
                eprintln!("  FAIL {f}");
            }
        }
    }
    assert_eq!(failed, 0);
}
