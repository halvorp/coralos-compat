//! Import census of E32 images - which DLLs and ordinals an app needs:
//! `cargo run -p heirloom-fmt-e32 --example census -- <image>...`
fn main() {
    for p in std::env::args().skip(1) {
        let f = std::fs::read(&p).unwrap();
        match heirloom_fmt_e32::E32Image::parse(&f) {
            Ok(img) => {
                let total: usize = img.imports.iter().map(|d| d.imports.len()).sum();
                println!("{p}: {} DLLs, {total} imports, {} exports", img.imports.len(), img.exports.len());
                for d in &img.imports {
                    let mut ords: Vec<u16> = d.imports.iter().map(|i| i.ordinal).collect();
                    ords.sort_unstable();
                    ords.dedup();
                    println!("  {:>4} {} {:?}", d.imports.len(), d.name, ords);
                }
            }
            Err(e) => println!("{p}: ERR {e:?}"),
        }
    }
}
