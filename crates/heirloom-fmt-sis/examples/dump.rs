//! Lists a SIS package's files: `cargo run -p heirloom-fmt-sis --example dump -- [--extract <dir>] <package.sis>...`.
//! `--extract <dir>` also writes every file under `<dir>` at its target path (`!:\resource\apps\x.rsc` -> `<dir>/resource/apps/x.rsc`;
//! the drive letter or `!` is dropped, a conditional file is written too and marked in the listing). Local test input only.
fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let out = match args.iter().position(|a| a == "--extract") {
        Some(i) => { args.remove(i); Some(std::path::PathBuf::from(args.remove(i))) }
        None => None,
    };
    for p in args {
        let b = std::fs::read(&p).unwrap();
        match heirloom_fmt_sis::Package::parse(&b) {
            Ok(pk) => {
                println!("{p}: {:?} by {:?} {:?} uids {:x?}, {} files", pk.package_name, pk.vendor, pk.version, pk.uids, pk.files.len());
                for f in &pk.files {
                    println!("  {:>8} {}{}", f.bytes.len(), f.target, if f.is_conditional { " (conditional)" } else { "" });
                    if let Some(dir) = &out {
                        let rel = f.target.split_once(":\\").map_or(f.target.as_str(), |(_, r)| r).replace('\\', "/");
                        if rel.split('/').any(|c| c == "..") { println!("    refused: path climbs out of {}", dir.display()); continue; }
                        let dst = dir.join(rel.to_lowercase());
                        std::fs::create_dir_all(dst.parent().unwrap()).unwrap();
                        std::fs::write(&dst, &f.bytes).unwrap();
                    }
                }
            }
            Err(e) => println!("{p}: ERR {e:?}"),
        }
    }
}
