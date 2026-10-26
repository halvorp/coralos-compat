//! Lists a SIS package's files: `cargo run -p heirloom-fmt-sis --example dump -- <package.sis>...`
fn main() {
    for p in std::env::args().skip(1) {
        let b = std::fs::read(&p).unwrap();
        match heirloom_fmt_sis::Package::parse(&b) {
            Ok(pk) => {
                println!("{p}: {:?} by {:?} {:?} uids {:x?}, {} files", pk.package_name, pk.vendor, pk.version, pk.uids, pk.files.len());
                for f in &pk.files {
                    println!("  {:>8} {}{}", f.bytes.len(), f.target, if f.is_conditional { " (conditional)" } else { "" });
                }
            }
            Err(e) => println!("{p}: ERR {e:?}"),
        }
    }
}
