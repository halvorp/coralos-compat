//! Import census of a Mach-O x86_64 image: what it loads, binds, defines and references, measured with this crate's own parser.
//!
//! `cargo run --example census -- <image> [--out <dir>]` prints a Markdown summary; with `--out` it also writes
//! `symbols.tsv` (dylib, symbol, kind, lazy), `selectors.txt`, `classes.tsv` (defined classes with superclass and counts) and
//! `categories.tsv`. Facts only: it never executes the image.
use heirloom_fmt_macho::header::{LC_LOAD_WEAK_DYLIB, LC_REEXPORT_DYLIB};
use heirloom_fmt_macho::MachO;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;

fn version(v: u32) -> String {
    format!("{}.{}.{}", v >> 16, (v >> 8) & 0xff, v & 0xff)
}

fn kind_of(sym: &str) -> &'static str {
    if sym.starts_with("_OBJC_CLASS_$_") {
        "objc-class"
    } else if sym.starts_with("_OBJC_METACLASS_$_") {
        "objc-metaclass"
    } else if sym.starts_with("_OBJC_IVAR_$_") {
        "objc-ivar-offset"
    } else if sym.starts_with("_objc_") || sym.starts_with("__objc_") {
        "objc-runtime"
    } else if sym.starts_with("_swift") || sym.starts_with("$s") || sym.starts_with("__swift") {
        "swift"
    } else if sym.starts_with("_CG") || sym.starts_with("_CT") {
        "coregraphics-text"
    } else if sym.starts_with("_CF") || sym.starts_with("___CF") {
        "corefoundation"
    } else if sym.starts_with("_NS") {
        "appkit-foundation-symbol"
    } else if sym.starts_with("_OBJC_EHTYPE") || sym.starts_with("__ZT") || sym.starts_with("__Z") || sym.starts_with("___cxa") {
        "c++-abi"
    } else {
        "c-or-other"
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).expect("usage: census <image> [--out <dir>]");
    let out_dir = args.iter().position(|a| a == "--out").and_then(|i| args.get(i + 1));
    let file = std::fs::read(path).expect("read image");
    let m = MachO::parse(&file).expect("parse image");
    let mut md = String::new();

    let _ = writeln!(md, "### Image `{}`", path);
    let _ = writeln!(
        md,
        "filetype {} · flags {:#x} · PIE {} · chained fixups {} · min OS {} · entry {:?}",
        m.filetype,
        m.flags,
        m.is_pie(),
        m.has_chained_fixups,
        m.min_os.map(version).unwrap_or_else(|| "?".into()),
        m.entry
    );
    if let Some(n) = &m.install_name {
        let _ = writeln!(md, "install name `{n}`");
    }
    let _ = writeln!(md, "dylinker {:?} · rpaths {:?}", m.dylinker, m.rpaths);
    let mut cmds: BTreeMap<u32, usize> = BTreeMap::new();
    for c in &m.commands {
        *cmds.entry(*c).or_default() += 1;
    }
    let _ = writeln!(md, "load commands ({}): {:?}", m.commands.len(), cmds.iter().map(|(c, n)| format!("{c:#x}x{n}")).collect::<Vec<_>>());
    let _ = writeln!(md, "\nsegments / sections:");
    for s in &m.segments {
        let _ = writeln!(md, "- `{}` vm {:#x}+{:#x} prot {}/{}", s.name, s.vmaddr, s.vmsize, s.initprot, s.maxprot);
        for sec in &s.sections {
            let _ = writeln!(md, "  - `{}` {} bytes", sec.sectname, sec.size);
        }
    }

    // binds
    let binds = m.binds().expect("binds");
    let rebases = m.rebases().expect("rebases");
    let mut slot_symbol: HashMap<u64, String> = HashMap::new();
    let mut by_dylib: BTreeMap<i32, Vec<&heirloom_fmt_macho::Bind>> = BTreeMap::new();
    for b in &binds {
        slot_symbol.insert(m.segments[b.segment].vmaddr + b.offset, b.symbol.clone());
        by_dylib.entry(b.dylib_ordinal).or_default().push(b);
    }
    let _ = writeln!(md, "\nrebases {} · binds {} ({} lazy)\n", rebases.len(), binds.len(), binds.iter().filter(|b| b.lazy).count());
    let _ = writeln!(md, "| # | dylib | kind | binds | lazy | objc classes | other objc | c++ |");
    let _ = writeln!(md, "|---|---|---|---:|---:|---:|---:|---:|");
    let mut tsv = String::new();
    for (i, d) in m.dylibs.iter().enumerate() {
        let ord = i as i32 + 1;
        let list = by_dylib.get(&ord).cloned().unwrap_or_default();
        let cls = list.iter().filter(|b| kind_of(&b.symbol) == "objc-class").count();
        let objc = list.iter().filter(|b| kind_of(&b.symbol).starts_with("objc-") && kind_of(&b.symbol) != "objc-class").count();
        let cpp = list.iter().filter(|b| kind_of(&b.symbol) == "c++-abi").count();
        let kind = if d.kind == LC_LOAD_WEAK_DYLIB {
            "weak"
        } else if d.kind == LC_REEXPORT_DYLIB {
            "reexport"
        } else {
            "load"
        };
        let _ = writeln!(md, "| {} | `{}` ({}) | {} | {} | {} | {} | {} | {} |", ord, d.path, version(d.current_version), kind, list.len(), list.iter().filter(|b| b.lazy).count(), cls, objc, cpp);
        for b in &list {
            let _ = writeln!(tsv, "{}\t{}\t{}\t{}", d.path, b.symbol, kind_of(&b.symbol), b.lazy);
        }
    }
    for (ord, list) in &by_dylib {
        if *ord < 1 || *ord as usize > m.dylibs.len() {
            let _ = writeln!(md, "| {} | (special ordinal) | | {} | | | | |", ord, list.len());
            for b in list {
                let _ = writeln!(tsv, "(ordinal {ord})\t{}\t{}\t{}", b.symbol, kind_of(&b.symbol), b.lazy);
            }
        }
    }
    let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
    for b in &binds {
        *kinds.entry(kind_of(&b.symbol)).or_default() += 1;
    }
    let _ = writeln!(md, "\nbinds by kind: {:?}", kinds);

    // ObjC: defined classes
    let mut classes = String::new();
    let mut n_classes = 0;
    let mut supers: BTreeMap<String, usize> = BTreeMap::new();
    let mut swift_classes = 0;
    if let Some(cl) = m.section("__DATA", "__objc_classlist").or_else(|| m.section("__DATA_CONST", "__objc_classlist")) {
        for i in 0..(cl.size / 8) {
            let Some(class) = m.u64_at(cl.addr + i * 8) else { continue };
            let sup_slot = class + 8;
            let sup = slot_symbol.get(&sup_slot).cloned().or_else(|| m.u64_at(sup_slot).filter(|p| *p != 0).and_then(|p| m.u64_at(p + 32)).and_then(|d| m.u64_at((d & !7) + 24)).and_then(|n| m.cstr_at(n)));
            let data = m.u64_at(class + 32).unwrap_or(0);
            let is_swift = data & 3 != 0;
            let ro = data & !7;
            let name = m.u64_at(ro + 24).and_then(|p| m.cstr_at(p)).unwrap_or_default();
            let isize = m.bytes_at(ro + 8, 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).unwrap_or(0);
            let methods = m.u64_at(ro + 32).filter(|p| *p != 0).and_then(|p| m.bytes_at(p + 4, 4)).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).unwrap_or(0);
            let ivars = m.u64_at(ro + 48).filter(|p| *p != 0).and_then(|p| m.bytes_at(p + 4, 4)).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).unwrap_or(0);
            let sup_name = sup.unwrap_or_else(|| "?".into());
            let _ = writeln!(classes, "{name}\t{sup_name}\t{isize}\t{methods}\t{ivars}\t{}", if is_swift { "swift" } else { "objc" });
            *supers.entry(sup_name).or_default() += 1;
            swift_classes += is_swift as usize;
            n_classes += 1;
        }
    }
    let _ = writeln!(md, "\nObjC classes defined: {n_classes} (swift-flagged {swift_classes})");
    let mut top: Vec<_> = supers.iter().collect();
    top.sort_by(|a, b| b.1.cmp(a.1));
    let _ = writeln!(md, "superclasses (system classes this image subclasses must be real, subclassable classes with a stable instance size):");
    for (s, n) in top.iter().take(40) {
        let _ = writeln!(md, "- {s} x{n}");
    }

    // categories
    let mut cats = String::new();
    let mut n_cats = 0;
    if let Some(cl) = m.section("__DATA", "__objc_catlist").or_else(|| m.section("__DATA_CONST", "__objc_catlist")) {
        for i in 0..(cl.size / 8) {
            let Some(cat) = m.u64_at(cl.addr + i * 8) else { continue };
            let name = m.u64_at(cat).and_then(|p| m.cstr_at(p)).unwrap_or_default();
            let cls = slot_symbol.get(&(cat + 8)).cloned().or_else(|| m.u64_at(cat + 8).and_then(|p| m.u64_at(p + 32)).and_then(|d| m.u64_at((d & !7) + 24)).and_then(|n| m.cstr_at(n))).unwrap_or_else(|| "?".into());
            let methods = m.u64_at(cat + 16).filter(|p| *p != 0).and_then(|p| m.bytes_at(p + 4, 4)).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).unwrap_or(0);
            let _ = writeln!(cats, "{name}\t{cls}\t{methods}");
            n_cats += 1;
        }
    }
    let _ = writeln!(md, "\nObjC categories: {n_cats}");

    // selectors referenced
    let mut sels: BTreeSet<String> = BTreeSet::new();
    let mut n_selrefs = 0;
    if let Some(sr) = m.section("__DATA", "__objc_selrefs").or_else(|| m.section("__DATA_CONST", "__objc_selrefs")) {
        for i in 0..(sr.size / 8) {
            n_selrefs += 1;
            if let Some(s) = m.u64_at(sr.addr + i * 8).and_then(|p| m.cstr_at(p)) {
                sels.insert(s);
            }
        }
    }
    let cfstrings = m.section("__DATA", "__cfstring").map(|s| s.size / 32).unwrap_or(0);
    let _ = writeln!(md, "selector refs {n_selrefs} ({} distinct) · constant CFStrings {cfstrings} · protocols {}", sels.len(), m.section("__DATA", "__objc_protolist").map(|s| s.size / 8).unwrap_or(0));
    let swift_secs: Vec<_> = m.segments.iter().flat_map(|s| s.sections.iter()).filter(|s| s.sectname.starts_with("__swift")).map(|s| s.sectname.clone()).collect();
    let _ = writeln!(md, "swift sections: {:?}", swift_secs);

    print!("{md}");
    if let Some(dir) = out_dir {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(format!("{dir}/symbols.tsv"), tsv).unwrap();
        std::fs::write(format!("{dir}/selectors.txt"), sels.iter().cloned().collect::<Vec<_>>().join("\n")).unwrap();
        std::fs::write(format!("{dir}/classes.tsv"), classes).unwrap();
        std::fs::write(format!("{dir}/categories.tsv"), cats).unwrap();
    }
}
