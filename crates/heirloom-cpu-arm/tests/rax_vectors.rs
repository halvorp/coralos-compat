//! The ported HexRaysSA/rax generated AArch32 tests (commit 526acfa), as Unicorn-validated vectors: see
//! tools/port_rax_tests.py and tests/data/rax_port_summary.txt for what was ported and what was dropped.

mod common;

fn run(what: &str, text: &str) {
    common::run_vectors(text, &common::reviewed_unpredictable()).assert_clean(what);
}

#[test]
fn rax_a32_data_processing() {
    run("rax data_processing", include_str!("data/rax_data_processing.vec"));
}

#[test]
fn rax_a32_multiply() {
    run("rax multiply", include_str!("data/rax_multiply.vec"));
}

#[test]
fn rax_a32_memory() {
    run("rax memory", include_str!("data/rax_memory.vec"));
}

#[test]
fn rax_a32_branch() {
    run("rax branch", include_str!("data/rax_branch.vec"));
}

#[test]
fn rax_a32_misc() {
    run("rax misc", include_str!("data/rax_misc.vec"));
}

#[test]
fn rax_a32_system() {
    run("rax system", include_str!("data/rax_system.vec"));
}

#[test]
fn rax_a32_coprocessor() {
    run("rax coprocessor", include_str!("data/rax_coprocessor.vec"));
}

#[test]
fn rax_a32_vfp() {
    run("rax simd/vfp", include_str!("data/rax_vfp.vec"));
}

#[test]
fn rax_t16() {
    run("rax t16", include_str!("data/rax_thumb.vec"));
}
