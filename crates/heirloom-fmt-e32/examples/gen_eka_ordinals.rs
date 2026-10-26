use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::process;

use heirloom_fmt_e32::{DsoExport, E32Image};

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: gen_eka_ordinals <sdk_lib_dir> <out.tsv> [--only <e32_image>...]");
        process::exit(2);
    }
    let sdk_dir = &args[1];
    let out_path = &args[2];

    // Parse --only
    let mut only_dlls: Option<HashSet<String>> = None;
    let mut i = 3;
    if i < args.len() && args[i] == "--only" {
        i += 1;
        let mut set = HashSet::new();
        while i < args.len() {
            let bytes = fs::read(&args[i]).expect("read e32 image");
            let img = E32Image::parse(&bytes).expect("parse e32");
            for d in &img.imports {
                set.insert(d.name.to_ascii_lowercase());
            }
            i += 1;
        }
        only_dlls = Some(set);
    }

    // Walk SDK dir for *.dso
    let mut all_exports: Vec<DsoExport> = Vec::new();
    let entries = fs::read_dir(sdk_dir).expect("read sdk dir");
    for entry in entries {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.eq_ignore_ascii_case("dso"))
            .unwrap_or(false)
        {
            let bytes = fs::read(&path).expect("read dso");
            match heirloom_fmt_e32::dso_exports(&bytes) {
                Ok(mut exps) => {
                    if let Some(ref filter) = only_dlls {
                        exps.retain(|e| filter.contains(&e.dll.to_ascii_lowercase()));
                    }
                    all_exports.extend(exps);
                }
                Err(e) => {
                    eprintln!("WARN: {}: {:?}", path.display(), e);
                }
            }
        }
    }

    // Dedupe + conflict detection
    let mut map: HashMap<(String, u16), String> = HashMap::new();
    for exp in &all_exports {
        let key = (exp.dll.to_ascii_lowercase(), exp.ordinal);
        match map.get(&key) {
            Some(existing) if existing != &exp.name => {
                eprintln!(
                    "CONFLICT: {} ordinal {}: '{}' vs '{}'",
                    exp.dll, exp.ordinal, existing, exp.name
                );
                process::exit(1);
            }
            Some(_) => {} // dedup
            None => {
                map.insert(key, exp.name.clone());
            }
        }
    }

    // Sort by (dll, ordinal)
    let mut rows: Vec<(String, u16, String)> =
        map.into_iter().map(|((dll, ord), name)| (dll, ord, name)).collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

    // Write TSV
    let mut out = String::new();
    out.push_str("# S60 5th Edition SDK v1.0.1 armv5 import stubs\n");
    out.push_str(&format!("# {} rows\n", rows.len()));
    for (dll, ord, name) in &rows {
        out.push_str(&format!("{}\t{}\t{}\n", dll, ord, name));
    }
    fs::write(out_path, out).expect("write tsv");
    eprintln!("Wrote {} rows to {}", rows.len(), out_path);
}