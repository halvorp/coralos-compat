//! The core BUILD this client expects of cpu-ppc.library (lib.rs `EXPECTED_BUILD`): heirloom-cpu-ppc's sources at the same tree
//! (a cargo git dependency checks out the whole repository, so the sibling crate's src/ is there), hashed by the core's own
//! build_id.rs - a library that reports another number is refused at its first instance (REFUSED_BUILD).
use std::{env, fs, path::Path};
include!("../heirloom-cpu-ppc/build_id.rs");

fn main() {
    let core = Path::new(&env::var("CARGO_MANIFEST_DIR").expect("manifest dir")).join("../heirloom-cpu-ppc");
    let src = core.join("src");
    println!("cargo:rerun-if-changed={}", src.display());
    println!("cargo:rerun-if-changed={}", core.join("build_id.rs").display());
    fs::write(Path::new(&env::var("OUT_DIR").expect("out dir")).join("expected_build.rs"), format!("{:#018x}", build_id(&src))).expect("write");
}
