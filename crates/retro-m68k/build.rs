use std::{env, path::PathBuf, process::Command};
fn main() {
    let v = PathBuf::from("vendor");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let cc = cc::Build::new().get_compiler();
    // Step 1: build and run Musashi's generator (m68kmake) to emit m68kops.{c,h}.
    let gen = out.join("m68kmake");
    let st = Command::new(cc.path()).arg(v.join("m68kmake.c")).arg("-o").arg(&gen).status().unwrap();
    assert!(st.success(), "compiling m68kmake failed");
    let st = Command::new(&gen).arg(&out).arg(v.join("m68k_in.c")).status().unwrap();
    assert!(st.success(), "running m68kmake failed");
    cc::Build::new()
        .include(&v).include(&out).include(v.join("softfloat"))
        .file(v.join("m68kcpu.c")).file(out.join("m68kops.c")).file(v.join("softfloat/softfloat.c"))
        .warnings(false).opt_level(2)
        .compile("musashi");
    println!("cargo:rerun-if-changed=vendor");
}
