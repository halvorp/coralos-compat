use std::{env, path::PathBuf, process::Command};
fn main() {
    let v = PathBuf::from("vendor");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    // Cross build for a bare-metal target (CoralOS user mode, x86_64-unknown-none): the host generator
    // still needs the HOST compiler, the Musashi objects need freestanding clang + the libc shim headers.
    let bare = env::var("CARGO_CFG_TARGET_OS").map(|o| o == "none").unwrap_or(false);
    let host = env::var("HOST").unwrap();
    let cc = cc::Build::new().target(&host).host(&host).opt_level(0).get_compiler();
    // Step 1: build and run Musashi's generator (m68kmake) to emit m68kops.{c,h}.
    let gen = out.join("m68kmake");
    let st = Command::new(cc.path()).arg(v.join("m68kmake.c")).arg("-o").arg(&gen).status().unwrap();
    assert!(st.success(), "compiling m68kmake failed");
    let st = Command::new(&gen).arg(&out).arg(v.join("m68k_in.c")).status().unwrap();
    assert!(st.success(), "running m68kmake failed");
    let mut b = cc::Build::new();
    // The `pic` feature builds the position-independent objects cpu-68k.library needs; the default stays -fno-pic -mcmodel=small.
    let pic = env::var_os("CARGO_FEATURE_PIC").is_some();
    if bare {
        if pic { b.pic(true).flag("-fPIC"); } else { b.pic(false).flag("-fno-pic").flag("-mcmodel=small"); }
        b.compiler("clang").flag("--target=x86_64-unknown-none-elf").flag("-ffreestanding").flag("-nostdinc")
            .flag("-mno-red-zone").flag("-fno-stack-protector")
            .include(v.join("shim")).file(v.join("shim/shim.c"));
        let ri = Command::new("clang").arg("-print-resource-dir").output().unwrap();
        b.flag("-isystem").flag(&format!("{}/include", String::from_utf8(ri.stdout).unwrap().trim()));
    }
    b.include(&v).include(&out).include(v.join("softfloat"))
        .file(v.join("m68kcpu.c")).file(out.join("m68kops.c")).file(v.join("softfloat/softfloat.c"))
        .warnings(false).opt_level(2)
        .compile("musashi");
    println!("cargo:rerun-if-changed=vendor");
}
