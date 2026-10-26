use std::env;
use std::fs;
use std::process;

use heirloom_fmt_e32::{E32Image, OrdinalTable};

fn main() {
    let mut args = env::args().skip(1);
    let mut table: Option<OrdinalTable> = None;
    let mut paths = Vec::new();

    while let Some(arg) = args.next() {
        if arg == "--ordinals" && paths.is_empty() {
            let path = match args.next() {
                Some(path) => path,
                None => {
                    eprintln!("--ordinals requires a TSV path");
                    process::exit(2);
                }
            };
            let tsv = fs::read_to_string(&path).expect("read ordinals TSV");
            table = Some(match OrdinalTable::parse(&tsv) {
                Ok(table) => table,
                Err((line, error)) => {
                    eprintln!("{}:{}: {}", path, line, error);
                    process::exit(1);
                }
            });
        } else {
            paths.push(arg);
        }
    }

    if paths.is_empty() {
        eprintln!("Usage: census [--ordinals <tsv>] <e32_image>...");
        process::exit(2);
    }

    for path in paths {
        let bytes = fs::read(&path).expect("read e32 image");
        let img = E32Image::parse(&bytes).expect("parse e32");
        println!("{}:", path);

        let mut named = 0usize;
        let mut total = 0usize;
        for dll in &img.imports {
            println!("  {}:", dll.name);
            for imp in &dll.imports {
                let name = table.as_ref().and_then(|t| t.name_of(&dll.name, imp.ordinal));
                if name.is_some() {
                    named += 1;
                }
                total += 1;
                println!("    {} {}", imp.ordinal, name.unwrap_or("?"));
            }
        }
        println!("named {} / {} imports, unnamed {}", named, total, total - named);
    }
}