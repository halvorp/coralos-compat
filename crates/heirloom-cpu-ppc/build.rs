//! The crate's BUILD identity (lib.rs `BUILD`): FNV-1a 64 over every source file under src/, in path order - the same sources
//! give the same number on any host, so a device's cpu-ppc.library can be matched against the tree it was built from.
use std::{env, fs, path::Path};

fn files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let mut v: Vec<_> = fs::read_dir(dir).expect("src").map(|e| e.expect("entry").path()).collect();
    v.sort();
    for p in v { if p.is_dir() { files(&p, out) } else { out.push(p) } }
}

fn main() {
    let src = Path::new(&env::var("CARGO_MANIFEST_DIR").expect("manifest dir")).join("src");
    println!("cargo:rerun-if-changed={}", src.display());
    let mut all = Vec::new();
    files(&src, &mut all);
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for p in &all {
        for b in p.strip_prefix(&src).expect("under src").to_string_lossy().bytes().chain(fs::read(p).expect("read")) {
            h ^= b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    fs::write(Path::new(&env::var("OUT_DIR").expect("out dir")).join("build_id.rs"), format!("{h:#018x}")).expect("write");
}
