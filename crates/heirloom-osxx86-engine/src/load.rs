//! The Mach-O loader: maps an x86_64 executable into engine-owned pages, applies the slide, binds every import to the native
//! [`libsystem`] table, refuses images that carry a raw trap, and publishes the pages with their final protections.
//!
//! Order matters and is fixed: copy, rebase, bind (pages are read-write and never executable), scan the final code bytes, then
//! publish. Every error after the reservation releases it.
use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;

use heirloom_fmt_macho::header::{LC_LOAD_WEAK_DYLIB, MH_EXECUTE, VM_PROT_EXECUTE, VM_PROT_READ, VM_PROT_WRITE};
use heirloom_fmt_macho::{Entry, MachO, MachoError, Segment};

use crate::{libsystem, trap, CodeMem, Prot, Refusal};

/// `S_ATTR_PURE_INSTRUCTIONS | S_ATTR_SOME_INSTRUCTIONS`: a section with either bit holds machine code.
const SECTION_HOLDS_CODE: u32 = 0x8000_0400;
const PAGE_MASK: u64 = 0xfff;

/// A mapped image.
pub struct Loaded {
    /// The start of the reservation (the lowest non-PAGEZERO segment).
    pub base: *mut u8,
    /// Bytes reserved.
    pub reserved: usize,
    /// The lowest segment address the file asked for.
    pub lo: u64,
    /// `base - lo`, wrapping.
    pub slide: i64,
    /// The absolute address of the entry point.
    pub entry: usize,
    /// Segments mapped (PAGEZERO excluded).
    pub segments: usize,
    /// Bind records written.
    pub imports: usize,
}

impl Loaded {
    /// The address range `[start, end)` of the reservation.
    pub fn span(&self) -> (usize, usize) {
        (self.base as usize, self.base as usize + self.reserved)
    }
}

/// Releases the reservation on every early return; disarmed once the image is handed over.
struct ReleaseGuard<'a> {
    mem: &'a mut dyn CodeMem,
    base: *mut u8,
    len: usize,
    armed: bool,
}

impl Drop for ReleaseGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.mem.release(self.base, self.len);
        }
    }
}

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

/// Maps `file` into pages from `mem`; see the module documentation for the order of the steps.
pub fn load(file: &[u8], mem: &mut dyn CodeMem) -> Result<Loaded, Refusal> {
    let image = MachO::parse(file).map_err(Refusal::Image)?;
    if image.has_chained_fixups {
        return Err(Refusal::UnsupportedFixups);
    }
    if image.filetype != MH_EXECUTE {
        return Err(bad_image("not an executable"));
    }
    let entryoff = match image.entry {
        Some(Entry::Main { entryoff, .. }) => entryoff,
        _ => return Err(Refusal::NoEntry),
    };
    let (lo, hi) = image.vm_span().ok_or(Refusal::NoEntry)?;
    // a segment starts on a page; its size may stop short of one (the linker leaves __LINKEDIT unrounded), and the pages are whole
    let hi = page_up(hi).ok_or_else(|| bad_image("segment range overflows"))?;
    for seg in image.segments.iter().filter(|s| !s.is_pagezero()) {
        if seg.vmaddr & PAGE_MASK != 0 {
            return Err(bad_image("segment not page aligned"));
        }
    }
    if !image.is_pie() {
        return Err(Refusal::FixedBase { vmaddr: lo });
    }

    let reserved = usize::try_from(hi - lo).map_err(|_| Refusal::OutOfMemory)?;
    let base = mem.reserve(reserved).ok_or(Refusal::OutOfMemory)?;
    let slide = (base as u64).wrapping_sub(lo) as i64;
    let mut guard = ReleaseGuard { mem, base, len: reserved, armed: true };
    let at = |seg: &Segment, offset: u64, len: usize| slot(base as usize, reserved, lo, seg.vmaddr, offset, len);

    // 1. copy the file-backed part of every segment; the rest of the reservation is already zero
    let mut segments = 0;
    for (index, seg) in image.segments.iter().enumerate() {
        if seg.is_pagezero() {
            continue;
        }
        let bytes = image.segment_bytes(index);
        let dest = at(seg, 0, bytes.len()).ok_or(Refusal::BadFixup("segment outside the reservation"))?;
        // SAFETY: `slot` proved [dest, dest + len) lies inside the reservation, which is writable; the source is a file slice.
        unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), dest as *mut u8, bytes.len()) };
        segments += 1;
    }

    // 2. rebase: add the slide to every pointer that holds an address of the image
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

    // 2b. a library the image links must be served whether or not it binds anything from it (weak links may be missing)
    for d in image.dylibs.iter().filter(|d| d.kind != LC_LOAD_WEAK_DYLIB) {
        if served_library(&d.path).is_none() {
            return Err(Refusal::ForeignDylib { path: d.path.clone() });
        }
    }

    // 3. bind: eager, for the bind and the lazy-bind streams alike
    let mut unbound: Vec<String> = Vec::new();
    let mut imports = 0;
    for b in image.binds().map_err(Refusal::Image)? {
        let named_dylib = usize::try_from(b.dylib_ordinal).ok().filter(|o| *o >= 1 && *o <= image.dylibs.len());
        let Some(ordinal) = named_dylib else {
            // flat lookup, main-executable and self ordinals name no library this table serves
            if !unbound.contains(&b.symbol) {
                unbound.push(b.symbol.clone());
            }
            continue;
        };
        let path = &image.dylibs[ordinal - 1].path;
        let resolver = match served_library(path) {
            Some(r) => r,
            // a weak library this engine does not serve is absent: its weak symbols bind to null, its strong ones stay unbound
            None if image.dylibs[ordinal - 1].kind == LC_LOAD_WEAK_DYLIB => |_: &str| None,
            None => return Err(Refusal::ForeignDylib { path: path.clone() }),
        };
        let seg = &image.segments[b.segment];
        let word = at(seg, b.offset, 8).ok_or(Refusal::BadFixup("bind outside the reservation"))?;
        let value = match resolver(&b.symbol) {
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
    if !unbound.is_empty() {
        return Err(Refusal::Unbound { symbols: unbound });
    }

    // 3b. Objective-C: unique selectors, realized classes and attached categories, while the pages are still writable
    let meta = objc_meta(&image, base as usize, lo);
    if meta != heirloom_objc::ImageMeta::default() {
        // SAFETY: the sections were copied, rebased and bound above; the pages are read-write until published.
        unsafe { heirloom_objc::register_image(&meta) };
    }

    // 4. admission: sweep the final code of every executable segment for raw traps
    let header_len = image.slice.get(20..24).map_or(0, |b| 32 + u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
    for seg in image.segments.iter().filter(|s| !s.is_pagezero() && s.is_executable()) {
        let len = usize::try_from(seg.filesize).map_err(|_| Refusal::BadFixup("segment outside the reservation"))?;
        let start = at(seg, 0, len).ok_or(Refusal::BadFixup("segment outside the reservation"))?;
        // SAFETY: `slot` proved the range lies inside the reservation; nothing else touches it while the engine holds the guard.
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

    // 5. publish with the final protections; a segment that asks for write and execute is refused
    let mut ranges = Vec::with_capacity(segments);
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
    guard.mem.publish(base, &ranges).map_err(Refusal::Publish)?;

    // 6. the entry point: LC_MAIN's offset is a file offset into __TEXT
    let text = &image.segments[image.segment_index("__TEXT").ok_or(Refusal::NoEntry)?];
    if entryoff >= text.filesize {
        return Err(Refusal::NoEntry);
    }
    let entry = at(text, entryoff, 1).ok_or(Refusal::NoEntry)?;
    guard.armed = false;
    Ok(Loaded { base, reserved, lo, slide, entry, segments, imports })
}

#[cfg(test)]
mod tests {
    use super::*;
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
