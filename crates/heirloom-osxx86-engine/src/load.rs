//! The program loader, a dyld subset: maps the main executable and every Mach-O library it links (found through `@rpath`,
//! `@loader_path` and `@executable_path`), applies each slide, binds every import to the native tables or to the export trie of
//! the library that provides it, registers the Objective-C metadata of each image dependencies first, refuses images that carry a
//! raw trap, and publishes the pages with their final protections.
//!
//! Order is fixed: map every image (copy, rebase), bind every image (pages are read-write and never executable), register the
//! Objective-C metadata, scan the final code bytes, then publish. Every error releases every reservation made so far.
use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;

use heirloom_fmt_macho::header::{LC_LOAD_WEAK_DYLIB, MH_DYLIB, MH_EXECUTE, VM_PROT_EXECUTE, VM_PROT_READ, VM_PROT_WRITE};
use heirloom_fmt_macho::{Entry, MachO, MachoError, Segment};

use crate::{libsystem, trap, CodeMem, ImageSource, Prot, Refusal};

/// `S_ATTR_PURE_INSTRUCTIONS | S_ATTR_SOME_INSTRUCTIONS`: a section with either bit holds machine code.
const SECTION_HOLDS_CODE: u32 = 0x8000_0400;
const PAGE_MASK: u64 = 0xfff;
/// `S_MOD_INIT_FUNC_POINTERS`: a section of initializer function pointers.
const S_MOD_INIT_FUNC_POINTERS: u32 = 0x9;
const SECTION_TYPE_MASK: u32 = 0xff;
/// How many libraries a program may map (a cycle or a runaway closure ends here).
const MAX_IMAGES: usize = 256;

/// `v` rounded up to a page, or None on overflow.
fn page_up(v: u64) -> Option<u64> {
    Some(v.checked_add(PAGE_MASK)? & !PAGE_MASK)
}

fn bad_image(why: &'static str) -> Refusal {
    Refusal::Image(MachoError::BadLoadCommand { index: 0, why })
}

/// The address of `len` bytes at `seg_vmaddr + offset` inside the reservation, or None when they are not wholly inside it.
fn slot(base: usize, reserved: usize, lo: u64, seg_vmaddr: u64, offset: u64, len: usize) -> Option<usize> {
    let at = seg_vmaddr.checked_sub(lo)?.checked_add(offset)?;
    let end = at.checked_add(len as u64)?;
    if end > reserved as u64 {
        return None;
    }
    base.checked_add(usize::try_from(at).ok()?)
}

/// The byte ranges (segment-relative, within the file-backed part) of an executable segment that the trap sweep decodes: everything
/// except the Mach-O header with its load commands (only in the segment that starts at file offset 0) and sections declared as data
/// (`__const`, `__unwind_info`, `__eh_frame`, strings). Bytes that no section declares are swept, so a segment without sections
/// is swept whole.
pub(crate) fn code_ranges(seg: &Segment, header_len: usize) -> Vec<(usize, usize)> {
    let end = usize::try_from(seg.filesize).unwrap_or(usize::MAX);
    let mut skip: Vec<(usize, usize)> = Vec::new();
    if seg.fileoff == 0 {
        skip.push((0, header_len.min(end)));
    }
    for s in &seg.sections {
        if s.flags & SECTION_HOLDS_CODE != 0 {
            continue;
        }
        let Some(rel) = s.addr.checked_sub(seg.vmaddr) else { continue };
        let (Ok(start), Ok(len)) = (usize::try_from(rel), usize::try_from(s.size)) else { continue };
        skip.push((start.min(end), start.saturating_add(len).min(end)));
    }
    skip.sort_unstable();
    let mut ranges = Vec::new();
    let mut cursor = 0usize;
    for (s, e) in skip {
        if s > cursor {
            ranges.push((cursor, s));
        }
        cursor = cursor.max(e);
    }
    if cursor < end {
        ranges.push((cursor, end));
    }
    ranges
}

/// The resolver for a library the engine serves natively, by install name; None for every other library.
fn served_library(path: &str) -> Option<fn(&str) -> Option<usize>> {
    match path {
        "/usr/lib/libSystem.B.dylib" | "/usr/lib/libSystem.dylib" => Some(libsystem::resolve),
        "/usr/lib/libobjc.A.dylib" => Some(heirloom_objc::libobjc::symbol),
        _ if is_framework(path, &["AppKit", "Foundation", "Cocoa", "CoreFoundation"]) => Some(heirloom_osx_appkit::symbol),
        _ => None,
    }
}

/// Whether `path` is `/System/Library/Frameworks/<name>.framework/...` for one of `names`.
fn is_framework(path: &str, names: &[&str]) -> bool {
    path.strip_prefix("/System/Library/Frameworks/")
        .and_then(|rest| rest.split_once(".framework/"))
        .is_some_and(|(name, _)| names.contains(&name))
}

/// Where the image's Objective-C metadata sections are in memory (slid addresses) and how many entries each holds.
fn objc_meta(image: &MachO<'_>, base: usize, lo: u64) -> heirloom_objc::ImageMeta {
    let find = |name: &str| {
        image
            .segments
            .iter()
            .flat_map(|s| s.sections.iter())
            .find(|s| s.sectname == name)
            .map(|s| (base + (s.addr - lo) as usize, (s.size / 8) as usize))
            .unwrap_or((0, 0))
    };
    heirloom_objc::ImageMeta { selrefs: find("__objc_selrefs"), classlist: find("__objc_classlist"), catlist: find("__objc_catlist") }
}


/// Where a library import binds: a native table, another mapped image, or nowhere (an absent weak library).
#[derive(Clone, Copy)]
enum Dep {
    Served(fn(&str) -> Option<usize>),
    Image(usize),
    Absent,
}

/// One mapped Mach-O image.
struct Img {
    path: String,
    file: Vec<u8>,
    base: *mut u8,
    reserved: usize,
    lo: u64,
    slide: i64,
    deps: Vec<Dep>,
    imports: usize,
}

/// What runs on the guest's stack before `main`: per image, dependencies first, its `+load` methods and then its initializers.
#[derive(Default)]
pub struct Startup {
    pub steps: Vec<(Option<heirloom_objc::ImageMeta>, Vec<usize>)>,
}

/// A mapped program: the main executable, its libraries and what the guest runs first.
pub struct Program {
    images: Vec<Img>,
    /// The absolute address of the main entry point.
    pub entry: usize,
    /// Segments mapped over all images (PAGEZERO excluded).
    pub segments: usize,
    /// Bind records written over all images.
    pub imports: usize,
    /// Libraries mapped besides the main executable.
    pub libraries: usize,
    /// `+load` methods and initializers to run before `main`.
    pub startup: Startup,
}

impl Program {
    /// The address ranges `[start, end)` of every mapped image.
    pub fn spans(&self) -> Vec<(usize, usize)> {
        self.images.iter().map(|i| (i.base as usize, i.base as usize + i.reserved)).collect()
    }
    /// Returns every reservation to `mem`.
    pub fn release(&mut self, mem: &mut dyn CodeMem) {
        for i in self.images.drain(..) {
            mem.release(i.base, i.reserved);
        }
    }
}

/// Resolves `@executable_path`, `@loader_path` and `@rpath` in `template` to candidate absolute paths, normalized.
fn expand(template: &str, exe_dir: &str, loader_dir: &str, rpaths: &[String]) -> Vec<String> {
    if let Some(rest) = template.strip_prefix("@executable_path/") {
        return alloc::vec![normalize(&format!("{exe_dir}/{rest}"))];
    }
    if let Some(rest) = template.strip_prefix("@loader_path/") {
        return alloc::vec![normalize(&format!("{loader_dir}/{rest}"))];
    }
    if let Some(rest) = template.strip_prefix("@rpath/") {
        let mut out = Vec::new();
        for r in rpaths {
            for base in expand(&format!("{r}/"), exe_dir, loader_dir, &[]) {
                out.push(normalize(&format!("{base}/{rest}")));
            }
        }
        return out;
    }
    alloc::vec![normalize(template)]
}

/// Collapses `.`, `..` and repeated slashes.
fn normalize(path: &str) -> String {
    let absolute = path.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for p in path.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    let joined = parts.join("/");
    if absolute {
        format!("/{joined}")
    } else {
        joined
    }
}

fn dir_of(path: &str) -> String {
    match path.rfind('/') {
        Some(0) => String::from("/"),
        Some(i) => String::from(&path[..i]),
        None => String::new(),
    }
}

fn is_main_dylib_check(image: &MachO<'_>, main: bool) -> Result<(), Refusal> {
    if image.has_chained_fixups {
        return Err(Refusal::UnsupportedFixups);
    }
    if main {
        if image.filetype != MH_EXECUTE {
            return Err(bad_image("not an executable"));
        }
        if !matches!(image.entry, Some(Entry::Main { .. })) {
            return Err(Refusal::NoEntry);
        }
    } else if image.filetype != MH_DYLIB {
        return Err(bad_image("a linked library is not a dylib"));
    }
    Ok(())
}

/// The address of `name` exported by image `j` (following re-exports through that image's own library list).
fn lookup_export(images: &[Img], j: usize, name: &str, depth: u8) -> Option<usize> {
    if depth > 8 {
        return None;
    }
    let img = &images[j];
    let m = MachO::parse(&img.file).ok()?;
    let e = m.export(name).ok()??;
    if let Some((ordinal, original)) = e.reexport {
        let target = img.deps.get((ordinal as usize).checked_sub(1)?)?;
        let n = if original.is_empty() { name } else { original.as_str() };
        return match *target {
            Dep::Served(r) => r(n),
            Dep::Image(k) => lookup_export(images, k, n, depth + 1),
            Dep::Absent => None,
        };
    }
    let text = m.segments.iter().find(|s| s.name == "__TEXT")?;
    Some(img.base as usize + (text.vmaddr - img.lo) as usize + e.address as usize)
}

/// Maps `file` (the main executable at `exe_path`) and every library it links, binds, registers and publishes them.
pub fn load_program(file: &[u8], exe_path: &str, source: &mut dyn ImageSource, mem: &mut dyn CodeMem) -> Result<Program, Refusal> {
    let mut program = Program { images: Vec::new(), entry: 0, segments: 0, imports: 0, libraries: 0, startup: Startup::default() };
    match load_into(&mut program, file, exe_path, source, mem) {
        Ok(()) => Ok(program),
        Err(e) => {
            program.release(mem);
            Err(e)
        }
    }
}

fn load_into(p: &mut Program, file: &[u8], exe_path: &str, source: &mut dyn ImageSource, mem: &mut dyn CodeMem) -> Result<(), Refusal> {
    let exe_dir = dir_of(exe_path);
    // 1. discover: the main image first, then every library breadth first
    p.images.push(Img { path: String::from(exe_path), file: file.to_vec(), base: core::ptr::null_mut(), reserved: 0, lo: 0, slide: 0, deps: Vec::new(), imports: 0 });
    let main_rpaths: Vec<String> = MachO::parse(file).map_err(Refusal::Image)?.rpaths;
    let mut next = 0;
    while next < p.images.len() {
        let (path, bytes) = (p.images[next].path.clone(), p.images[next].file.clone());
        let m = MachO::parse(&bytes).map_err(Refusal::Image)?;
        is_main_dylib_check(&m, next == 0)?;
        let mut rpaths = m.rpaths.clone();
        rpaths.extend(main_rpaths.iter().cloned());
        let loader_dir = dir_of(&path);
        let mut deps = Vec::new();
        for d in &m.dylibs {
            if let Some(r) = served_library(&d.path) {
                deps.push(Dep::Served(r));
                continue;
            }
            let found = expand(&d.path, &exe_dir, &loader_dir, &rpaths).into_iter().find_map(|cand| {
                if let Some(i) = p.images.iter().position(|x| x.path == cand) {
                    return Some((cand, None, Some(i)));
                }
                source.read(&cand).map(|b| (cand, Some(b), None))
            });
            match found {
                Some((_, _, Some(i))) => deps.push(Dep::Image(i)),
                Some((cand, Some(b), None)) => {
                    if p.images.len() >= MAX_IMAGES {
                        return Err(Refusal::BadFixup("too many libraries"));
                    }
                    p.images.push(Img { path: cand, file: b, base: core::ptr::null_mut(), reserved: 0, lo: 0, slide: 0, deps: Vec::new(), imports: 0 });
                    deps.push(Dep::Image(p.images.len() - 1));
                }
                _ if d.kind == LC_LOAD_WEAK_DYLIB => deps.push(Dep::Absent),
                _ => return Err(Refusal::ForeignDylib { path: d.path.clone() }),
            }
        }
        p.images[next].deps = deps;
        next += 1;
    }
    p.libraries = p.images.len() - 1;

    // 2. map: reserve, copy, rebase
    for idx in 0..p.images.len() {
        map_image(p, idx, mem)?;
    }
    // 3. bind
    let mut unbound: Vec<String> = Vec::new();
    for idx in 0..p.images.len() {
        bind_image(p, idx, &mut unbound)?;
    }
    if !unbound.is_empty() {
        return Err(Refusal::Unbound { symbols: unbound });
    }
    p.imports = p.images.iter().map(|i| i.imports).sum();

    // 4. Objective-C metadata and initializers, dependencies first
    let order = post_order(&p.images);
    for &idx in &order {
        let img = &p.images[idx];
        let m = MachO::parse(&img.file).map_err(Refusal::Image)?;
        let meta = objc_meta(&m, img.base as usize, img.lo);
        let has_meta = meta != heirloom_objc::ImageMeta::default();
        if has_meta {
            // SAFETY: the image was copied, rebased and bound above; its pages are read-write until published.
            unsafe { heirloom_objc::register_image(&meta) };
        }
        let mut inits = Vec::new();
        for seg in &m.segments {
            for s in &seg.sections {
                if s.flags & SECTION_TYPE_MASK == S_MOD_INIT_FUNC_POINTERS {
                    for k in 0..(s.size / 8) {
                        let at = img.base as usize + (s.addr - img.lo) as usize + (k * 8) as usize;
                        // SAFETY: the section lies inside the reservation and was rebased above.
                        inits.push(unsafe { core::ptr::read_unaligned(at as *const usize) });
                    }
                }
            }
        }
        if has_meta || !inits.is_empty() {
            p.startup.steps.push((has_meta.then_some(meta), inits));
        }
    }

    // 5. admission scan and publish
    for idx in 0..p.images.len() {
        scan_and_publish(p, idx, mem)?;
    }

    // 6. the entry point
    let img = &p.images[0];
    let m = MachO::parse(&img.file).map_err(Refusal::Image)?;
    let Some(Entry::Main { entryoff, .. }) = m.entry else { return Err(Refusal::NoEntry) };
    let text = &m.segments[m.segment_index("__TEXT").ok_or(Refusal::NoEntry)?];
    if entryoff >= text.filesize {
        return Err(Refusal::NoEntry);
    }
    p.entry = img.base as usize + (text.vmaddr - img.lo) as usize + entryoff as usize;
    Ok(())
}

/// Image indices with every image after the libraries it links (the main executable last).
fn post_order(images: &[Img]) -> Vec<usize> {
    fn visit(images: &[Img], i: usize, seen: &mut Vec<usize>, out: &mut Vec<usize>) {
        if seen.contains(&i) {
            return;
        }
        seen.push(i);
        for d in &images[i].deps {
            if let Dep::Image(j) = d {
                visit(images, *j, seen, out);
            }
        }
        out.push(i);
    }
    let mut out = Vec::new();
    let mut seen = Vec::new();
    visit(images, 0, &mut seen, &mut out);
    out
}

fn map_image(p: &mut Program, idx: usize, mem: &mut dyn CodeMem) -> Result<(), Refusal> {
    let file = p.images[idx].file.clone();
    let image = MachO::parse(&file).map_err(Refusal::Image)?;
    let (lo, hi) = image.vm_span().ok_or(Refusal::NoEntry)?;
    // a segment starts on a page; its size may stop short of one (the linker leaves __LINKEDIT unrounded), and the pages are whole
    let hi = page_up(hi).ok_or_else(|| bad_image("segment range overflows"))?;
    for seg in image.segments.iter().filter(|s| !s.is_pagezero()) {
        if seg.vmaddr & PAGE_MASK != 0 {
            return Err(bad_image("segment not page aligned"));
        }
    }
    if idx == 0 && !image.is_pie() {
        return Err(Refusal::FixedBase { vmaddr: lo });
    }
    let reserved = usize::try_from(hi - lo).map_err(|_| Refusal::OutOfMemory)?;
    let base = mem.reserve(reserved).ok_or(Refusal::OutOfMemory)?;
    {
        let img = &mut p.images[idx];
        img.base = base;
        img.reserved = reserved;
        img.lo = lo;
        img.slide = (base as u64).wrapping_sub(lo) as i64;
    }
    let (slide, base_u) = (p.images[idx].slide, base as usize);
    let at = |seg: &Segment, offset: u64, len: usize| slot(base_u, reserved, lo, seg.vmaddr, offset, len);
    for (index, seg) in image.segments.iter().enumerate() {
        if seg.is_pagezero() {
            continue;
        }
        let bytes = image.segment_bytes(index);
        let dest = at(seg, 0, bytes.len()).ok_or(Refusal::BadFixup("segment outside the reservation"))?;
        // SAFETY: `slot` proved [dest, dest + len) lies inside the reservation, which is writable; the source is a file slice.
        unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), dest as *mut u8, bytes.len()) };
        p.segments += 1;
    }
    for r in image.rebases().map_err(Refusal::Image)? {
        let seg = &image.segments[r.segment];
        let word = at(seg, r.offset, 8).ok_or(Refusal::BadFixup("rebase outside the image"))?;
        // SAFETY: `slot` proved the eight bytes lie inside the writable reservation; the access is unaligned-safe.
        let value = unsafe { core::ptr::read_unaligned(word as *const u64) };
        if !(lo..hi).contains(&value) {
            return Err(Refusal::BadFixup("rebase target outside the image"));
        }
        // SAFETY: the same proven slot.
        unsafe { core::ptr::write_unaligned(word as *mut u64, value.wrapping_add(slide as u64)) };
    }
    Ok(())
}

fn bind_image(p: &mut Program, idx: usize, unbound: &mut Vec<String>) -> Result<(), Refusal> {
    let file = p.images[idx].file.clone();
    let image = MachO::parse(&file).map_err(Refusal::Image)?;
    let (base_u, reserved, lo) = (p.images[idx].base as usize, p.images[idx].reserved, p.images[idx].lo);
    let mut imports = 0;
    for b in image.binds().map_err(Refusal::Image)? {
        let seg = &image.segments[b.segment];
        let word = slot(base_u, reserved, lo, seg.vmaddr, b.offset, 8).ok_or(Refusal::BadFixup("bind outside the reservation"))?;
        let found: Option<usize> = match b.dylib_ordinal {
            0 => lookup_export(&p.images, idx, &b.symbol, 0),
            -1 => lookup_export(&p.images, 0, &b.symbol, 0),
            -2 => (0..p.images.len()).find_map(|j| lookup_export(&p.images, j, &b.symbol, 0)),
            n if n >= 1 && (n as usize) <= p.images[idx].deps.len() => match p.images[idx].deps[n as usize - 1] {
                Dep::Served(r) => r(&b.symbol),
                Dep::Image(j) => lookup_export(&p.images, j, &b.symbol, 0),
                Dep::Absent => None,
            },
            _ => None,
        };
        let value = match found {
            Some(addr) => (addr as u64).wrapping_add(b.addend as u64),
            None if b.weak_import => 0,
            None => {
                if !unbound.contains(&b.symbol) {
                    unbound.push(b.symbol.clone());
                }
                continue;
            }
        };
        // SAFETY: `slot` proved the eight bytes lie inside the writable reservation.
        unsafe { core::ptr::write_unaligned(word as *mut u64, value) };
        imports += 1;
    }
    p.images[idx].imports = imports;
    Ok(())
}

fn scan_and_publish(p: &mut Program, idx: usize, mem: &mut dyn CodeMem) -> Result<(), Refusal> {
    let file = p.images[idx].file.clone();
    let image = MachO::parse(&file).map_err(Refusal::Image)?;
    let (base, base_u, reserved, lo) = (p.images[idx].base, p.images[idx].base as usize, p.images[idx].reserved, p.images[idx].lo);
    let at = |seg: &Segment, offset: u64, len: usize| slot(base_u, reserved, lo, seg.vmaddr, offset, len);
    let header_len = image.slice.get(20..24).map_or(0, |b| 32 + u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
    for seg in image.segments.iter().filter(|s| !s.is_pagezero() && s.is_executable()) {
        let len = usize::try_from(seg.filesize).map_err(|_| Refusal::BadFixup("segment outside the reservation"))?;
        let start = at(seg, 0, len).ok_or(Refusal::BadFixup("segment outside the reservation"))?;
        // SAFETY: `slot` proved the range lies inside the reservation; nothing else touches it before publication.
        let mapped = unsafe { core::slice::from_raw_parts(start as *const u8, len) };
        for (from, to) in code_ranges(seg, header_len) {
            if let Some((hit, kind)) = trap::scan_code(&mapped[from..to]) {
                let address = seg.vmaddr + (from + hit) as u64;
                let (section, offset) = match seg.sections.iter().find(|s| s.addr <= address && address < s.addr + s.size) {
                    Some(s) => (format!("{},{}", s.segname, s.sectname), address - s.addr),
                    None => (seg.name.clone(), address - seg.vmaddr),
                };
                return Err(Refusal::RawTrap { section, offset, kind });
            }
        }
    }
    let mut ranges = Vec::new();
    for seg in image.segments.iter().filter(|s| !s.is_pagezero()) {
        if seg.initprot & VM_PROT_WRITE != 0 && seg.initprot & VM_PROT_EXECUTE != 0 {
            return Err(Refusal::Publish("segment is writable and executable"));
        }
        let prot = if seg.initprot & VM_PROT_EXECUTE != 0 {
            Prot::RX
        } else if seg.initprot & VM_PROT_WRITE != 0 {
            Prot::RW
        } else if seg.initprot & VM_PROT_READ != 0 {
            Prot::R
        } else {
            Prot::None
        };
        let offset = usize::try_from(seg.vmaddr - lo).map_err(|_| Refusal::OutOfMemory)?;
        let length = usize::try_from(page_up(seg.vmsize).ok_or(Refusal::OutOfMemory)?).map_err(|_| Refusal::OutOfMemory)?;
        ranges.push((offset, length, prot));
    }
    mem.publish(base, &ranges).map_err(Refusal::Publish)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_normalize_and_the_dyld_variables_expand() {
        assert_eq!(normalize("/a/b/../c/./d//e"), "/a/c/d/e");
        assert_eq!(normalize("a/../../x"), "x");
        assert_eq!(dir_of("/app/MacOS/app"), "/app/MacOS");
        assert_eq!(dir_of("/app"), "/");
        assert_eq!(
            expand("@rpath/Foo.framework/Foo", "/app/MacOS", "/app/MacOS", &[String::from("@executable_path/../Frameworks"), String::from("/opt/lib")]),
            alloc::vec![String::from("/app/Frameworks/Foo.framework/Foo"), String::from("/opt/lib/Foo.framework/Foo")]
        );
        assert_eq!(expand("@loader_path/../lib/x.dylib", "/e", "/app/Frameworks", &[]), alloc::vec![String::from("/app/lib/x.dylib")]);
        assert_eq!(expand("@executable_path/x", "/app/MacOS", "/elsewhere", &[]), alloc::vec![String::from("/app/MacOS/x")]);
        assert_eq!(expand("/usr/lib/libz.1.dylib", "/e", "/l", &[]), alloc::vec![String::from("/usr/lib/libz.1.dylib")]);
        // an @rpath entry that is itself relative to the loader resolves against the LOADER's directory
        assert_eq!(expand("@rpath/x", "/app/MacOS", "/app/Frameworks/Sub", &[String::from("@loader_path/..")]), alloc::vec![String::from("/app/Frameworks/x")]);
    }

    use alloc::vec;
    use heirloom_fmt_macho::Section;

    fn section(name: &str, addr: u64, size: u64, flags: u32) -> Section {
        Section { sectname: name.into(), segname: "__TEXT".into(), addr, size, offset: 0, flags }
    }

    fn segment(sections: Vec<Section>, fileoff: u64, filesize: u64) -> Segment {
        Segment { name: "__TEXT".into(), vmaddr: 0x1000, vmsize: 0x2000, fileoff, filesize, maxprot: 5, initprot: 5, sections }
    }

    #[test]
    fn code_ranges_skip_the_header_and_declared_data() {
        let seg = segment(
            vec![
                section("__text", 0x1400, 0x100, 0x8000_0400),
                section("__const", 0x1500, 0x80, 0),
                section("__stubs", 0x1580, 0x20, 0x8000_0408),
            ],
            0,
            0x1000,
        );
        // header 0..0x400, code 0x400..0x500, const 0x500..0x580 skipped, stubs 0x580.., the rest of the file-backed part
        assert_eq!(code_ranges(&seg, 0x400), vec![(0x400, 0x500), (0x580, 0x1000)]);
    }

    #[test]
    fn a_segment_without_sections_is_swept_whole() {
        let seg = segment(Vec::new(), 0x2000, 0x800);
        assert_eq!(code_ranges(&seg, 0x400), vec![(0, 0x800)], "only a segment at file offset 0 holds the header");
        let seg = segment(Vec::new(), 0, 0x800);
        assert_eq!(code_ranges(&seg, 0x400), vec![(0x400, 0x800)]);
    }

    #[test]
    fn overlapping_data_sections_do_not_underflow() {
        let seg = segment(vec![section("__a", 0x1100, 0x200, 0), section("__b", 0x1200, 0x300, 0)], 0x4000, 0x1000);
        assert_eq!(code_ranges(&seg, 0), vec![(0, 0x100), (0x500, 0x1000)]);
    }
}
