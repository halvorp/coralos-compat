//! dos.library pattern calls: ParsePattern / MatchPattern (+NoCase) over pattern.rs, and MatchFirst / MatchNext /
//! MatchEnd over the volume with a real `struct AnchorPath` and `struct AChain` chain.
//!
//! Offsets: m68k gcc -m68020 -mno-align-int over the AROS dos/dosasl.h field lists (2026-10-18): AnchorPath ap_Base 0,
//! ap_Last 4, ap_BreakBits 8, ap_FoundBreak 12, ap_Flags 16, ap_Reserved 17, ap_Strlen 18, ap_Info 20 (a 260-byte
//! FileInfoBlock), ap_Buf 280; AChain an_Child 0, an_Parent 4, an_Lock 8, an_Info 12, an_Flags 272, an_String 273
//! (274 bytes).
//!
//! MatchFirst walks the pattern's path components, one AChain per directory level. A component without wildcards is
//! a lookup, one with wildcards is matched (case-insensitively) against the directory's entries. A directory that is
//! returned sets APF_DODIR; if the program leaves it set, the next MatchNext enters the directory and returns ALL of
//! its entries (recursively, each directory again honouring APF_DODIR), then returns the directory a second time with
//! APF_DIDDIR. A program clears APF_DODIR to list one level. An assign with several directories is searched in its
//! first directory only. ap_BreakBits are not polled: one task, nothing can raise a break.
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use crate::dos_err::*;
use crate::fs::*;
use crate::libtable::LibraryBuilder;
use crate::pattern::{plain_text, tokenize, tokenised_len, Pattern};
use crate::volume::NodeId;
use crate::{machine::Amiga, Refusal};
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

const ERROR_BAD_TEMPLATE: DosErr = DosErr(114);
const ERROR_BUFFER_OVERFLOW: DosErr = DosErr(303);
const AP_BASE: u32 = 0;
const AP_LAST: u32 = 4;
const AP_FLAGS: u32 = 16;
const AP_STRLEN: u32 = 18;
const AP_INFO: u32 = 20;
const AP_BUF: u32 = 280;
const AN_CHILD: u32 = 0;
const AN_PARENT: u32 = 4;
const AN_LOCK: u32 = 8;
const AN_INFO: u32 = 12;
const AN_FLAGS: u32 = 272;
const AN_SIZE: u32 = 274;
const DDF_PATTERN: u8 = 1;
const APF_ITSWILD: u8 = 1 << 1;
const APF_DODIR: u8 = 1 << 2;
const APF_DIDDIR: u8 = 1 << 3;

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.named("ParsePattern", parse_pattern::<U>).named("ParsePatternNoCase", parse_pattern::<U>)
        .named("MatchPattern", match_pattern::<U>).named("MatchPatternNoCase", match_pattern_nocase::<U>)
        .named("MatchFirst", match_first::<U>).named("MatchNext", match_next::<U>).named("MatchEnd", match_end::<U>)
}

/// ParsePattern[NoCase](d1 = source, d2 = dest buffer, d3 = its size) -> 1 if the source has wildcards, 0 if not, -1 on
/// error (IoErr ERROR_BAD_TEMPLATE for bad syntax, ERROR_BUFFER_OVERFLOW if the tokenised form does not fit).
fn parse_pattern<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (src, dest, size) = (cpu.dreg(1), cpu.dreg(2), cpu.dreg(3));
    let text = crate::guest_str::read_bytes_z(&a.mem, src, 0x1_0000).map_err(Refusal::mem("ParsePattern.source"))?;
    let what = format!("[dos] ParsePattern \"{}\"", String::from_utf8_lossy(&text));
    let (tokens, wild) = match tokenize(&text) { Ok(t) => t, Err(e) => return fail(a, cpu, ERROR_BAD_TEMPLATE, u32::MAX, format!("{what} ({e:?})")) };
    if tokens.len() as u32 > size { return fail(a, cpu, ERROR_BUFFER_OVERFLOW, u32::MAX, format!("{what} (needs {} bytes, buffer is {size})", tokens.len())); }
    a.mem.write_bytes(dest, &tokens).map_err(Refusal::mem("ParsePattern.dest"))?;
    ok(a, cpu, u32::from(wild), format!("{what} -> {} wildcards", if wild { "has" } else { "no" }))
}

fn do_match<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, nocase: bool) -> Result<String, Refusal> {
    let (pat, s) = (cpu.dreg(1), cpu.dreg(2));
    let name = if nocase { "MatchPatternNoCase" } else { "MatchPattern" };
    let n = tokenised_len(|i| a.mem.read_u8(pat + i as u32).ok().filter(|_| i < 0x2_0000)).ok_or_else(|| Refusal(format!("{name}: {pat:#x} is not a ParsePattern result (no P_STOP)")))?;
    let toks: Vec<u8> = (0..n as u32).map(|i| a.mem.read_u8(pat + i).map_err(Refusal::mem("MatchPattern.pat"))).collect::<Result<_, _>>()?;
    let text = crate::guest_str::read_bytes_z(&a.mem, s, 0x1_0000).map_err(Refusal::mem("MatchPattern.string"))?;
    let p = Pattern::from_tokens(&toks, nocase).map_err(|e| Refusal(format!("{name}: {pat:#x} is not a ParsePattern result ({e:?})")))?;
    let hit = p.matches(&text).map_err(|e| Refusal(format!("{name}: {e:?}")))?;
    ok(a, cpu, if hit { DOSTRUE } else { 0 }, format!("[dos] {name} \"{}\" -> {hit}", String::from_utf8_lossy(&text)))
}
fn match_pattern<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> { do_match(a, cpu, false) }
fn match_pattern_nocase<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> { do_match(a, cpu, true) }

#[derive(Clone)]
enum Sel { Lookup(String), Wild(Pattern), All }

struct Level {
    entries: Vec<NodeId>, idx: usize, chain: u32, sel: Sel,
    /// Index of the pattern component this level matches (usize::MAX for a directory entered by APF_DODIR).
    depth: usize,
    /// The directory a DODIR level was entered for (leaving it returns the directory with APF_DIDDIR).
    recursive: Option<NodeId>,
    final_level: bool,
}

/// MatchFirst/MatchNext state of one AnchorPath, keyed by its address.
pub struct Anchor { levels: Vec<Level>, comps: Vec<(Sel, bool)>, pending: Option<NodeId> }

const ALL_DEPTH: usize = usize::MAX;

#[derive(Default)]
pub struct Anchors(alloc::collections::BTreeMap<u32, Anchor>);

impl Anchors { pub fn live(&self) -> usize { self.0.len() } }

fn scan_dir<U: GuestUi>(a: &Amiga<U>, dir: NodeId, sel: &Sel) -> Vec<NodeId> {
    let v = &a.fs.vol;
    match sel {
        Sel::Lookup(n) => v.find_child(dir, n).into_iter().collect(),
        Sel::All => v.children(dir).to_vec(),
        Sel::Wild(p) => v.children(dir).iter().copied().filter(|&c| p.matches(v.name(c).as_bytes()).unwrap_or(false)).collect(),
    }
}

/// Push the level that scans `dir`: an AChain with a lock on it, linked after the current last one.
fn push_level<U: GuestUi>(a: &mut Amiga<U>, ap: u32, dir: NodeId, sel: Sel, depth: usize, recursive: Option<NodeId>, final_level: bool, wild: bool) -> Result<bool, Refusal> {
    let Some(chain) = alloc_zeroed(a, AN_SIZE)? else { return Ok(false) };
    let Some(lock) = new_lock(a, dir, false)? else { a.heap.free(chain, AN_SIZE).ok(); return Ok(false) };
    let m = Refusal::mem("AChain");
    crate::dos_files::fill_fib(a, chain + AN_INFO, dir)?;
    a.mem.write_u32(chain + AN_LOCK, lock >> 2).map_err(&m)?;
    a.mem.write_u8(chain + AN_FLAGS, if wild { DDF_PATTERN } else { 0 }).map_err(&m)?;
    let prev = a.mem.read_u32(ap + AP_LAST).map_err(&m)?;
    if prev != 0 { a.mem.write_u32(prev + AN_CHILD, chain).map_err(&m)?; a.mem.write_u32(chain + AN_PARENT, prev).map_err(&m)?; } else { a.mem.write_u32(ap + AP_BASE, chain).map_err(&m)?; }
    a.mem.write_u32(ap + AP_LAST, chain).map_err(&m)?;
    let entries = scan_dir(a, dir, &sel);
    a.fs.anchors.0.get_mut(&ap).expect("anchor exists").levels.push(Level { entries, idx: 0, chain, sel, depth, recursive, final_level });
    Ok(true)
}

fn pop_level<U: GuestUi>(a: &mut Amiga<U>, ap: u32) -> Result<Level, Refusal> {
    let l = a.fs.anchors.0.get_mut(&ap).and_then(|x| x.levels.pop()).expect("a level to pop");
    let m = Refusal::mem("AChain");
    let parent = a.mem.read_u32(l.chain + AN_PARENT).map_err(&m)?;
    a.mem.write_u32(ap + AP_LAST, parent).map_err(&m)?;
    if parent != 0 { a.mem.write_u32(parent + AN_CHILD, 0).map_err(&m)?; } else { a.mem.write_u32(ap + AP_BASE, 0).map_err(&m)?; }
    let lock = a.mem.read_u32(l.chain + AN_LOCK).map_err(&m)?;
    crate::dos_files::free_lock(a, lock, "MatchNext")?;
    a.heap.free(l.chain, AN_SIZE).map_err(|e| Refusal(format!("AChain: {e}")))?;
    Ok(l)
}

fn teardown<U: GuestUi>(a: &mut Amiga<U>, ap: u32) -> Result<(), Refusal> {
    while a.fs.anchors.0.get(&ap).map_or(false, |x| !x.levels.is_empty()) { pop_level(a, ap)?; }
    a.fs.anchors.0.remove(&ap);
    Ok(())
}

/// Report `node` as the match: ap_Info, ap_Buf and the flags.
fn report<U: GuestUi>(a: &mut Amiga<U>, ap: u32, node: NodeId, flags: u8, what: &str) -> Result<Result<(), DosErr>, Refusal> {
    let m = Refusal::mem("AnchorPath");
    crate::dos_files::fill_fib(a, ap + AP_INFO, node)?;
    let path = a.fs.vol.full_path(node);
    let cap = a.mem.read_i16(ap + AP_STRLEN).map_err(&m)?;
    if cap > 0 {
        if path.len() + 1 > cap as usize { return Ok(Err(ERROR_BUFFER_OVERFLOW)); }
        crate::guest_str::write_z(&mut a.mem, ap + AP_BUF, path.as_bytes()).map_err(&m)?;
    }
    let is_dir = a.fs.vol.is_dir(node);
    let old = a.mem.read_u8(ap + AP_FLAGS).map_err(&m)?;
    let new = (old & APF_ITSWILD) | (flags & APF_DIDDIR) | if is_dir && flags & APF_DIDDIR == 0 { APF_DODIR } else { 0 };
    a.mem.write_u8(ap + AP_FLAGS, new).map_err(&m)?;
    let _ = what;
    Ok(Ok(()))
}

/// MatchFirst(d1 = pattern, d2 = AnchorPath) -> 0 or an error code (also in IoErr).
fn match_first<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (pat, ap) = (cpu.dreg(1), cpu.dreg(2));
    let text = crate::guest_str::read_bytes_z(&a.mem, pat, 0x1_0000).map_err(Refusal::mem("MatchFirst.pattern"))?;
    let what = format!("[dos] MatchFirst \"{}\"", String::from_utf8_lossy(&text));
    if ap & 3 != 0 { return Err(Refusal(format!("MatchFirst: AnchorPath {ap:#x} is not longword aligned"))); }
    teardown(a, ap)?;
    let m = Refusal::mem("AnchorPath");
    for off in [AP_BASE, AP_LAST] { a.mem.write_u32(ap + off, 0).map_err(&m)?; }
    // Split "Dev:dir/pat/pat" into the directory the walk starts in and its components.
    let (head, rest) = match text.iter().rposition(|&c| c == b':') { Some(i) => (&text[..=i], &text[i + 1..]), None => (&text[..0], &text[..]) };
    let cwd = cur_dir(a);
    let mut root = if head.is_empty() { cwd } else {
        let h = String::from_utf8_lossy(head).into_owned();
        match a.fs.vol.resolve(cwd, &h) { Ok(n) => n, Err(e) => return finish(a, cpu, ap, e, what) }
    };
    let mut comps = Vec::new();
    let mut wild_any = false;
    let mut rest = rest;
    while let Some(r) = rest.strip_prefix(b"/") { // a leading '/' is the parent directory
        match a.fs.vol.parent_of(root) { Ok(p) => root = p, Err(e) => return finish(a, cpu, ap, e, what) }
        rest = r;
    }
    for c in rest.split(|&c| c == b'/').filter(|c| !c.is_empty()) {
        match plain_text(c) {
            Err(_) => return finish(a, cpu, ap, ERROR_BAD_TEMPLATE, what),
            Ok(Some(t)) => comps.push((Sel::Lookup(String::from_utf8_lossy(&t).into_owned()), false)),
            Ok(None) => {
                let (tok, _) = tokenize(c).map_err(|_| Refusal(format!("{what}: bad pattern")))?;
                wild_any = true;
                comps.push((Sel::Wild(Pattern::from_tokens(&tok, true).map_err(|e| Refusal(format!("{what}: {e:?}")))?), true));
            }
        }
    }
    let flags = a.mem.read_u8(ap + AP_FLAGS).map_err(&m)?;
    a.mem.write_u8(ap + AP_FLAGS, (flags & !(APF_ITSWILD | APF_DODIR | APF_DIDDIR)) | if wild_any { APF_ITSWILD } else { 0 }).map_err(&m)?;
    if comps.is_empty() {
        // "SYS:" or "": the match is the starting directory itself.
        return match report(a, ap, root, 0, &what)? { Ok(()) => { a.fs.anchors.0.insert(ap, Anchor { levels: vec![], comps: vec![], pending: Some(root) }); ok(a, cpu, 0, what) } Err(e) => finish(a, cpu, ap, e, what) };
    }
    let (sel, wild) = comps[0].clone();
    let single = comps.len() == 1;
    a.fs.anchors.0.insert(ap, Anchor { levels: vec![], comps, pending: None });
    if !push_level(a, ap, root, sel, 0, None, single, wild)? { teardown(a, ap)?; return fail(a, cpu, ERROR_NO_FREE_STORE, ERROR_NO_FREE_STORE.0, what); }
    advance(a, cpu, ap, what)
}

fn finish<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, ap: u32, e: DosErr, what: String) -> Result<String, Refusal> {
    teardown(a, ap)?;
    fail(a, cpu, e, e.0, what)
}

/// MatchNext(d1 = AnchorPath).
fn match_next<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let ap = cpu.dreg(1);
    let what = format!("[dos] MatchNext {ap:#x}");
    if !a.fs.anchors.0.contains_key(&ap) { return Err(Refusal(format!("MatchNext: {ap:#x} was not set up by MatchFirst"))); }
    // The directory returned last is entered only if the program left APF_DODIR set.
    let flags = a.mem.read_u8(ap + AP_FLAGS).map_err(Refusal::mem("AnchorPath"))?;
    if let Some(dir) = a.fs.anchors.0.get_mut(&ap).and_then(|x| x.pending.take()) {
        if flags & APF_DODIR != 0 && a.fs.vol.is_dir(dir) {
            let last_final = a.fs.anchors.0[&ap].levels.last().map_or(true, |l| l.final_level);
            if last_final && !push_level(a, ap, dir, Sel::All, ALL_DEPTH, Some(dir), true, false)? { teardown(a, ap)?; return fail(a, cpu, ERROR_NO_FREE_STORE, ERROR_NO_FREE_STORE.0, what); }
        }
    }
    advance(a, cpu, ap, what)
}

/// Find the next match from the current state, or run out.
fn advance<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, ap: u32, what: String) -> Result<String, Refusal> {
    loop {
        let Some(top) = a.fs.anchors.0[&ap].levels.last() else { return finish(a, cpu, ap, ERROR_NO_MORE_ENTRIES, what) };
        if top.idx >= top.entries.len() {
            let l = pop_level(a, ap)?;
            if let Some(dir) = l.recursive {
                // Leaving a directory the program entered: report it again, flagged APF_DIDDIR.
                return match report(a, ap, dir, APF_DIDDIR, &what)? {
                    Ok(()) => ok(a, cpu, 0, format!("{what} -> left {}", a.fs.vol.full_path(dir))),
                    Err(e) => finish(a, cpu, ap, e, what),
                };
            }
            if a.fs.anchors.0[&ap].levels.is_empty() { return finish(a, cpu, ap, if l.entries.is_empty() && matches!(l.sel, Sel::Lookup(_)) { ERROR_OBJECT_NOT_FOUND } else { ERROR_NO_MORE_ENTRIES }, what); }
            continue;
        }
        let (node, final_level) = (top.entries[top.idx], top.final_level);
        a.fs.anchors.0.get_mut(&ap).expect("anchor").levels.last_mut().expect("level").idx += 1;
        if !final_level {
            // A directory matching a non-final component is entered to match the next one.
            if a.fs.vol.is_dir(node) {
                let depth = a.fs.anchors.0[&ap].levels.last().expect("level").depth + 1;
                let (sel, wild) = a.fs.anchors.0[&ap].comps[depth].clone();
                let last = depth + 1 == a.fs.anchors.0[&ap].comps.len();
                if !push_level(a, ap, node, sel, depth, None, last, wild)? { teardown(a, ap)?; return fail(a, cpu, ERROR_NO_FREE_STORE, ERROR_NO_FREE_STORE.0, what); }
            }
            continue;
        }
        return match report(a, ap, node, 0, &what)? {
            Ok(()) => { a.fs.anchors.0.get_mut(&ap).expect("anchor").pending = Some(node); ok(a, cpu, 0, format!("{what} -> {}", a.fs.vol.full_path(node))) }
            Err(e) => finish(a, cpu, ap, e, what),
        };
    }
}

/// MatchEnd(d1 = AnchorPath): releases the locks and chains MatchFirst made.
fn match_end<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let ap = cpu.dreg(1);
    teardown(a, ap)?;
    Ok(format!("[dos] MatchEnd {ap:#x}"))
}
