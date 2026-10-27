//! Host-side extractor: writes the CDL layout data file `heirloom_ekaarm_engine::cdlextract` builds (see there for what is read).
//!
//! usage: gen_cdl_layout <aknlayout2-dir> <generated-subdir e.g. Qhd_touch_akn_app> <out.cdldata> [<avkon.rsg> [<layout.dll>]]
fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 4 || a.len() > 6 {
        eprintln!("usage: gen_cdl_layout <aknlayout2-dir> <generated-subdir> <out.cdldata> [<avkon.rsg> [<layout.dll>]]");
        std::process::exit(2);
    }
    match heirloom_ekaarm_engine::cdlextract::extract(std::path::Path::new(&a[1]), &a[2], a.get(4).map(std::path::Path::new), a.get(5).map(std::path::Path::new)) {
        Ok((text, summary)) => {
            std::fs::write(&a[3], text).expect("write");
            eprintln!("{summary} -> {}", a[3]);
        }
        Err(e) => {
            eprintln!("gen_cdl_layout: {e}");
            std::process::exit(1);
        }
    }
}
