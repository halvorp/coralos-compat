//! The crate's BUILD identity (lib.rs `BUILD`, build_id.rs): cpu-ppc.library reports it (PpcProf::build) and the client refuses a
//! library whose number is not the one it was built against.
use std::{env, fs, path::Path};
include!("build_id.rs");

fn main() {
    let src = Path::new(&env::var("CARGO_MANIFEST_DIR").expect("manifest dir")).join("src");
    println!("cargo:rerun-if-changed={}", src.display());
    println!("cargo:rerun-if-changed=build_id.rs");
    fs::write(Path::new(&env::var("OUT_DIR").expect("out dir")).join("build_id.rs"), format!("{:#018x}", build_id(&src))).expect("write");
}
