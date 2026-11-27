//! iffparse.library, part 1: the handle, the context stack and the DOS stream (AROS workbench/libs/iffparse, APL, read for facts).
//!
//! An `IFFHandle` is a REAL guest struct at AROS's layout (`struct IntIFFHandle`, 88 bytes; m68k gcc -mno-align-int over
//! libraries/iffparse.h and iffparse_intern.h, 2026-10-19): iff_Stream 0, iff_Flags 4, iff_Depth 8, then the built-in default
//! ContextNode at 12 (a ContextNode is cn_Node 0, cn_ID 8, cn_Type 12, cn_Size 16, cn_Scan 20; the private LCI list 24..36 and
//! cn_Composite 36 follow, 40 bytes), the context stack's MinList at 52, the parser state at 64, the stream hook at 72. Every
//! ContextNode is a real guest struct and the stack links (mln_Succ / mln_Pred) are kept current, so `CurrentChunk`/`ParentChunk`
//! and a program reading cn_ID / cn_Type / cn_Size / cn_Scan see exactly what Intuition-era code expects. What the guest must not
//! corrupt (the stack order, which LCIs hang on which context, the parser state, the engine-made handlers) is HOST-side.
//!
//! STREAMS: `InitIFFasDOS` (a dos file handle, host-native), exactly as `DOSStreamHandler`: Read / Write must transfer the whole
//! count or the call fails (IFFERR_READ / IFFERR_WRITE), Seek is relative, CLEANUP seeks to the start. `InitIFFasClip` is accepted
//! and `OpenIFF` then refuses by name (clipboard.device is not modelled) and so does a stream hook of the program's own (a guest
//! stream callback would have to run inside every read; it is a named refusal, IFFERR_NOHOOK). The AROS stream *buffering* for an
//! unknown-size PushChunk on a non-seekable stream is not needed: the DOS stream is random-seekable.
//! Deliberate differences from AROS, both bugs there: `OpenIFF` returns the error of a failed header read (AROS overwrites it with
//! the CLEANUP result, 0), and `ParentChunk` of the outermost chunk is NULL (AROS returns the hidden default node).
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;
use crate::libtable::LibraryBuilder;
use crate::{machine::Amiga, Refusal};
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

// libraries/iffparse.h
pub const IFFF_WRITE: u32 = 1;
pub const IFFF_RWBITS: u32 = 1;
pub const IFFF_RSEEK: u32 = 4;
pub const IFFF_NEWFILE: u32 = 1 << 16;
pub const IFFF_OPEN: u32 = 1 << 19;
pub const IFFERR_EOF: i32 = -1;
pub const IFFERR_EOC: i32 = -2;
pub const IFFERR_NOSCOPE: i32 = -3;
pub const IFFERR_NOMEM: i32 = -4;
pub const IFFERR_READ: i32 = -5;
pub const IFFERR_WRITE: i32 = -6;
pub const IFFERR_SEEK: i32 = -7;
pub const IFFERR_MANGLED: i32 = -8;
pub const IFFERR_SYNTAX: i32 = -9;
pub const IFFERR_NOTIFF: i32 = -10;
pub const IFFERR_NOHOOK: i32 = -11;
pub const IFF_RETURN2CLIENT: i32 = -12;
pub const ID_FORM: u32 = u32::from_be_bytes(*b"FORM");
pub const ID_LIST: u32 = u32::from_be_bytes(*b"LIST");
pub const ID_CAT: u32 = u32::from_be_bytes(*b"CAT ");
pub const ID_PROP: u32 = u32::from_be_bytes(*b"PROP");
pub const IFFSIZE_UNKNOWN: i32 = -1;

// struct IntIFFHandle
pub const HANDLE_SIZE: u32 = 88;
pub const IH_STREAM: u32 = 0;
pub const IH_FLAGS: u32 = 4;
pub const IH_DEPTH: u32 = 8;
pub const IH_DEFAULT_CN: u32 = 12;
pub const IH_CN_STACK: u32 = 52;
pub const IH_STATE: u32 = 64;
pub const IH_STREAM_HANDLER: u32 = 72;
// struct IntContextNode
pub const CN_SIZE: u32 = 40;
pub const CN_ID: u32 = 8;
pub const CN_TYPE: u32 = 12;
pub const CN_SIZE_F: u32 = 16;
pub const CN_SCAN: u32 = 20;
pub const CN_COMPOSITE: u32 = 36;

/// The parser's state (iffparse_intern.h IFFSTATE_*).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum PState { Composite = 0, PushChunk = 1, Atomic = 2, ScanExit = 3, Exit = 4, PopChunk = 5 }

impl PState {
    pub fn from_u32(v: u32) -> PState { match v { 1 => PState::PushChunk, 2 => PState::Atomic, 3 => PState::ScanExit, 4 => PState::Exit, 5 => PState::PopChunk, _ => PState::Composite } }
}

/// What the engine made on behalf of the library (everything else a program allocated itself).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Native { Prop, Collection, Handler }

/// The engine's own "hooks" the library installs for StopChunk / PropChunk / CollectionChunk / StopOnExit and the streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum NativeHook { Stop = 0, Prop = 1, Collection = 2, ExitContext = 3, Dos = 4, Clip = 5 }

pub struct Cn { pub composite: bool, /// LCI addresses, head (most recently stored) first.
    pub lcis: Vec<u32> }

pub struct ClipBuf {
    pub data: Vec<u8>,
    pub pos: usize,
    pub write: bool,
}

pub struct Iff {
    /// Context nodes, the default one first, the top of the stack last.
    pub stack: Vec<u32>,
    /// Transient IFF byte buffer only while an `InitIFFasClip` handle is open (`OpenIFF`..`CloseIFF`);
    /// never persisted across closes — CoralOS `clipboardd` (`Amiga::clip_host`) owns the clipboard.
    pub clip_buf: Option<ClipBuf>,
}

#[derive(Default)]
pub struct State {
    pub handles: BTreeMap<u32, Iff>,
    pub cns: BTreeMap<u32, Cn>,
    /// Every LocalContextItem struct that exists -> Some(kind) when the engine made it for its own handlers.
    pub lcis: BTreeMap<u32, Option<Native>>,
    /// Guest addresses standing for [`NativeHook`]s (allocated at the first AllocIFF).
    pub hooks: [u32; 6],
}

pub fn m(what: &'static str) -> impl Fn(heirloom_mem_flat32::MemError) -> Refusal { Refusal::mem(what) }

pub fn rd<U: GuestUi>(a: &Amiga<U>, at: u32) -> Result<u32, Refusal> { a.mem.read_u32(at).map_err(m("IFF struct")) }
pub fn wr<U: GuestUi>(a: &mut Amiga<U>, at: u32, v: u32) -> Result<(), Refusal> { a.mem.write_u32(at, v).map_err(m("IFF struct")) }

pub fn native_hook<U: GuestUi>(a: &Amiga<U>, which: NativeHook) -> u32 { a.iff.hooks[which as usize] }
pub fn which_hook<U: GuestUi>(a: &Amiga<U>, hook: u32) -> Option<NativeHook> {
    if hook == 0 { return None; }
    [NativeHook::Stop, NativeHook::Prop, NativeHook::Collection, NativeHook::ExitContext, NativeHook::Dos, NativeHook::Clip].into_iter().find(|&h| a.iff.hooks[h as usize] == hook)
}

/// A refusal that also gives the guest an error CODE (the dispatcher's D0 = 0 would read as "no error").
pub fn refuse<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, func: &str, why: String, d0: i32) -> Result<String, Refusal> {
    cpu.set_d0(d0 as u32);
    a.refused.push(format!("iffparse.library/{func}: {why}"));
    Ok(format!("[iffparse] {func} REFUSED: {why}"))
}

pub fn handle<U: GuestUi>(a: &Amiga<U>, func: &str, iff: u32) -> Result<(), Refusal> {
    if a.iff.handles.contains_key(&iff) { Ok(()) } else { Err(Refusal(format!("{func}: {iff:#x} is not an IFFHandle AllocIFF returned"))) }
}

pub fn flags<U: GuestUi>(a: &Amiga<U>, iff: u32) -> Result<u32, Refusal> { rd(a, iff + IH_FLAGS) }
pub fn depth<U: GuestUi>(a: &Amiga<U>, iff: u32) -> Result<u32, Refusal> { rd(a, iff + IH_DEPTH) }
pub fn state<U: GuestUi>(a: &Amiga<U>, iff: u32) -> Result<PState, Refusal> { Ok(PState::from_u32(rd(a, iff + IH_STATE)?)) }
pub fn set_state<U: GuestUi>(a: &mut Amiga<U>, iff: u32, s: PState) -> Result<(), Refusal> { wr(a, iff + IH_STATE, s as u32) }
pub fn top<U: GuestUi>(a: &Amiga<U>, iff: u32) -> u32 { *a.iff.handles[&iff].stack.last().expect("default context node") }
pub fn root<U: GuestUi>(a: &Amiga<U>, iff: u32) -> u32 { a.iff.handles[&iff].stack[0] }

/// Write the MinList links of the context stack (top first, the default node last) into guest memory.
pub fn relink<U: GuestUi>(a: &mut Amiga<U>, iff: u32) -> Result<(), Refusal> {
    let order: Vec<u32> = a.iff.handles[&iff].stack.iter().rev().copied().collect();
    let (head, tail) = (iff + IH_CN_STACK, iff + IH_CN_STACK + 4);
    wr(a, head, order[0])?;
    wr(a, tail, 0)?;
    wr(a, head + 8, *order.last().expect("root"))?;
    for (i, &n) in order.iter().enumerate() {
        wr(a, n, order.get(i + 1).copied().unwrap_or(tail))?;
        wr(a, n + 4, if i == 0 { head } else { order[i - 1] })?;
    }
    Ok(())
}

/// PushContextNode: GoodType / GoodID checked, composite chunks bring their own type, others inherit the enclosing one.
pub fn push_context<U: GuestUi>(a: &mut Amiga<U>, iff: u32, ty: u32, id: u32, size: i32, scan: i32) -> Result<i32, Refusal> {
    let composite = matches!(id, ID_FORM | ID_LIST | ID_CAT | ID_PROP);
    let ty = if composite { ty } else { rd(a, top(a, iff) + CN_TYPE)? };
    if !(good_type(ty) && good_id(id)) { return Ok(IFFERR_MANGLED); }
    let Some(cn) = a.heap.alloc(CN_SIZE) else { return Ok(IFFERR_NOMEM) };
    a.mem.zero(cn, CN_SIZE).map_err(m("ContextNode"))?;
    for (off, v) in [(CN_TYPE, ty), (CN_ID, id), (CN_SIZE_F, size as u32), (CN_SCAN, scan as u32), (CN_COMPOSITE, composite as u32)] { wr(a, cn + off, v)?; }
    a.iff.cns.insert(cn, Cn { composite, lcis: Vec::new() });
    a.iff.handles.get_mut(&iff).expect("handle").stack.push(cn);
    relink(a, iff)?;
    let d = depth(a, iff)?;
    wr(a, iff + IH_DEPTH, d + 1)?;
    Ok(0)
}

/// PopContextNode: the top node leaves the stack and is freed; its LCIs are returned for purging (a purge hook may be a guest's).
pub fn pop_context<U: GuestUi>(a: &mut Amiga<U>, iff: u32) -> Result<Vec<u32>, Refusal> {
    let cn = a.iff.handles.get_mut(&iff).expect("handle").stack.pop().expect("a context node");
    let lcis = a.iff.cns.remove(&cn).map(|c| c.lcis).unwrap_or_default();
    a.heap.free(cn, CN_SIZE).map_err(|e| Refusal(format!("PopContextNode: {e}")))?;
    relink(a, iff)?;
    let d = depth(a, iff)?;
    wr(a, iff + IH_DEPTH, d.wrapping_sub(1))?;
    Ok(lcis)
}

pub fn good_id(id: u32) -> bool {
    let b = id.to_be_bytes();
    if b[0] == 0x20 && id != 0x2020_2020 { return false; }
    b.iter().all(|&c| (0x20..=0x7e).contains(&c))
}
pub fn good_type(id: u32) -> bool {
    good_id(id) && id.to_be_bytes().iter().all(|&c| c <= b'Z' && (c >= b'A' || c.is_ascii_digit() || c == b' '))
}

// ---- the DOS stream ---------------------------------------------------------------------------------------------------

/// How a handle reads and writes: set by InitIFF*, checked at OpenIFF.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream { Dos, Clip, Custom(u32), None }

pub fn stream_kind<U: GuestUi>(a: &Amiga<U>, iff: u32) -> Result<Stream, Refusal> {
    let h = rd(a, iff + IH_STREAM_HANDLER)?;
    Ok(match which_hook(a, h) { Some(NativeHook::Dos) => Stream::Dos, Some(NativeHook::Clip) => Stream::Clip, _ if h == 0 => Stream::None, _ => Stream::Custom(h) })
}

/// Encode UTF-8 text as an Amiga IFF `FORM FTXT` containing one `CHRS` chunk (with even-byte pad).
pub fn encode_ftxt(text: &[u8]) -> Vec<u8> {
    let padded = text.len() + (text.len() & 1);
    let form_size = (4 + 8 + padded) as u32;
    let mut out = Vec::with_capacity(8 + form_size as usize);
    out.extend_from_slice(b"FORM");
    out.extend_from_slice(&form_size.to_be_bytes());
    out.extend_from_slice(b"FTXT");
    out.extend_from_slice(b"CHRS");
    out.extend_from_slice(&(text.len() as u32).to_be_bytes());
    out.extend_from_slice(text);
    if text.len() & 1 == 1 {
        out.push(0);
    }
    out
}

/// Extract concatenated `CHRS` bytes from an Amiga IFF `FORM FTXT`, or `None` if not a valid `FORM FTXT`.
pub fn decode_ftxt(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.len() < 12 || &bytes[0..4] != b"FORM" || &bytes[8..12] != b"FTXT" {
        return None;
    }
    let form_len = u32::from_be_bytes(bytes[4..8].try_into().ok()?) as usize;
    let end = (8usize.checked_add(form_len)?).min(bytes.len());
    let mut i = 12usize;
    let mut text = Vec::new();
    while i + 8 <= end {
        let id = &bytes[i..i + 4];
        let sz = u32::from_be_bytes(bytes[i + 4..i + 8].try_into().ok()?) as usize;
        let data_end = i.checked_add(8)?.checked_add(sz)?;
        if data_end > end {
            break;
        }
        if id == b"CHRS" {
            text.extend_from_slice(&bytes[i + 8..data_end]);
        }
        i = data_end + (sz & 1);
    }
    Some(text)
}

/// ReadStream: `n` bytes or an IFFERR code (a short read is IFFERR_READ, as `Read() != NBytes` is in DOSStreamHandler).
pub fn read_stream<U: GuestUi>(a: &mut Amiga<U>, iff: u32, n: u32) -> Result<Vec<u8>, i32> {
    if n == 0 { return Ok(Vec::new()); }
    if matches!(stream_kind(a, iff), Ok(Stream::Clip)) {
        let Some(cb) = a.iff.handles.get_mut(&iff).and_then(|h| h.clip_buf.as_mut()) else { return Err(IFFERR_READ) };
        if cb.write { return Err(IFFERR_READ); }
        let start = cb.pos.min(cb.data.len());
        let end = start.saturating_add(n as usize).min(cb.data.len());
        let out = cb.data[start..end].to_vec();
        cb.pos = end;
        return if out.len() != n as usize { Err(IFFERR_READ) } else { Ok(out) };
    }
    let Ok(bptr) = rd(a, iff + IH_STREAM) else { return Err(IFFERR_READ) };
    let Some(f) = a.fs.files.get(&(bptr << 2)) else { return Err(IFFERR_READ) };
    let (node, pos, readable) = (f.node, f.pos as usize, f.read);
    let data = a.fs.vol.file_data(node).unwrap_or(&[]);
    let start = pos.min(data.len());
    let end = start.saturating_add(n as usize).min(data.len());
    let out = data[start..end].to_vec();
    if let Some(f) = a.fs.files.get_mut(&(bptr << 2)) { f.pos = end as u32; }
    if !readable || out.len() != n as usize { Err(IFFERR_READ) } else { Ok(out) }
}

/// WriteStream: all of `bytes` or IFFERR_WRITE.
pub fn write_stream<U: GuestUi>(a: &mut Amiga<U>, iff: u32, bytes: &[u8]) -> i32 {
    if bytes.is_empty() { return 0; }
    if matches!(stream_kind(a, iff), Ok(Stream::Clip)) {
        let Some(cb) = a.iff.handles.get_mut(&iff).and_then(|h| h.clip_buf.as_mut()) else { return IFFERR_WRITE };
        if !cb.write { return IFFERR_WRITE; }
        let end = cb.pos.saturating_add(bytes.len());
        if end > guestsvc_abi::CLIP_PAYLOAD_MAX { return IFFERR_WRITE; }
        if end > cb.data.len() { cb.data.resize(end, 0); }
        cb.data[cb.pos..end].copy_from_slice(bytes);
        cb.pos = end;
        return 0;
    }
    let Ok(bptr) = rd(a, iff + IH_STREAM) else { return IFFERR_WRITE };
    if !a.fs.files.contains_key(&(bptr << 2)) { return IFFERR_WRITE; }
    match crate::dos_io::write_bytes_to(a, bptr, bytes) { Ok(true) => 0, _ => IFFERR_WRITE }
}

/// SeekStream relative to the current position: 0 or IFFERR_SEEK (a position outside the file is a seek error, as Seek() gives).
pub fn seek_stream<U: GuestUi>(a: &mut Amiga<U>, iff: u32, off: i32) -> i32 {
    if off == 0 { return 0; }
    if matches!(stream_kind(a, iff), Ok(Stream::Clip)) {
        let Some(cb) = a.iff.handles.get_mut(&iff).and_then(|h| h.clip_buf.as_mut()) else { return IFFERR_SEEK };
        let new = cb.pos as i64 + off as i64;
        if new < 0 || new > cb.data.len() as i64 { return IFFERR_SEEK; }
        cb.pos = new as usize;
        return 0;
    }
    let Ok(bptr) = rd(a, iff + IH_STREAM) else { return IFFERR_SEEK };
    let Some(f) = a.fs.files.get(&(bptr << 2)) else { return IFFERR_SEEK };
    let (node, pos) = (f.node, f.pos as i64);
    let size = a.fs.vol.file_data(node).map_or(0, |d| d.len() as i64);
    let new = pos + off as i64;
    if new < 0 || new > size { return IFFERR_SEEK; }
    a.fs.files.get_mut(&(bptr << 2)).expect("file").pos = new as u32;
    0
}

/// ReadStreamLong: a big-endian long, or an IFFERR code (a short read is IFFERR_EOF here, as in AROS).
pub fn read_long<U: GuestUi>(a: &mut Amiga<U>, iff: u32) -> Result<u32, i32> {
    read_stream(a, iff, 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// GetChunkHeader: read a chunk header from the stream and push its context node.
pub fn get_chunk_header<U: GuestUi>(a: &mut Amiga<U>, iff: u32) -> Result<i32, Refusal> {
    let id = match read_long(a, iff) { Ok(v) => v, Err(e) => return Ok(e) };
    let size = match read_long(a, iff) { Ok(v) => v, Err(e) => return Ok(e) };
    let (ty, scan) = if matches!(id, ID_FORM | ID_LIST | ID_CAT | ID_PROP) {
        match read_long(a, iff) { Ok(t) => (t, 4), Err(e) => return Ok(e) }
    } else { (0, 0) };
    push_context(a, iff, ty, id, size as i32, scan)
}

// ---- the API of this part ---------------------------------------------------------------------------------------------

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.named("AllocIFF", alloc_iff::<U>).named("FreeIFF", crate::iff_lci::free_iff::<U>).named("OpenIFF", open_iff::<U>).named("CloseIFF", close_iff::<U>)
        .named("InitIFF", init_iff::<U>).named("InitIFFasDOS", init_as_dos::<U>).named("InitIFFasClip", init_as_clip::<U>)
        .named("OpenClipboard", open_clipboard::<U>).named("CloseClipboard", close_clipboard::<U>)
        .named("GoodID", good_id_lvo::<U>).named("GoodType", good_type_lvo::<U>).named("IDtoStr", id_to_str::<U>)
        .with(crate::iff_chunk::register::<U>).with(crate::iff_lci::register::<U>).with(crate::iff_parse::register::<U>)
}

fn ensure_hooks<U: GuestUi>(a: &mut Amiga<U>) -> Result<(), Refusal> {
    if a.iff.hooks[0] != 0 { return Ok(()); }
    for i in 0..6 {
        let h = crate::fs::alloc_zeroed_sys(a, 20)?.ok_or_else(|| Refusal("iffparse.library: no memory for its hooks (NOMEM)".into()))?;
        a.iff.hooks[i] = h;
    }
    Ok(())
}

/// AllocIFF() -> a cleared IFFHandle (IFFF_READ) with its default context node, or NULL.
fn alloc_iff<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    ensure_hooks(a)?;
    let Some(h) = a.heap.alloc(HANDLE_SIZE) else { cpu.set_d0(0); return Ok("[iffparse] AllocIFF -> NULL (no memory)".into()) };
    a.mem.zero(h, HANDLE_SIZE).map_err(m("IFFHandle"))?;
    let root = h + IH_DEFAULT_CN;
    a.iff.cns.insert(root, Cn { composite: false, lcis: Vec::new() });
    a.iff.handles.insert(h, Iff { stack: alloc::vec![root], clip_buf: None });
    relink(a, h)?;
    cpu.set_d0(h);
    Ok(format!("[iffparse] AllocIFF -> {h:#x}"))
}

/// InitIFF(a0 = iff, d0 = flags, a1 = streamHook).
fn init_iff<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (iff, fl, hook) = (cpu.areg(0), cpu.dreg(0), cpu.areg(1));
    handle(a, "InitIFF", iff)?;
    let old = flags(a, iff)?;
    wr(a, iff + IH_FLAGS, old | fl)?;
    wr(a, iff + IH_STREAM_HANDLER, hook)?;
    Ok(format!("[iffparse] InitIFF {iff:#x} flags={fl:#x} hook={hook:#x}"))
}

fn init_as(a: &mut Amiga<impl GuestUi>, iff: u32, which: NativeHook, func: &str) -> Result<String, Refusal> {
    handle(a, func, iff)?;
    let old = flags(a, iff)?;
    wr(a, iff + IH_FLAGS, old | IFFF_RSEEK)?;
    let h = native_hook(a, which);
    wr(a, iff + IH_STREAM_HANDLER, h)?;
    Ok(format!("[iffparse] {func} {iff:#x}"))
}
fn init_as_dos<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> { init_as(a, cpu.areg(0), NativeHook::Dos, "InitIFFasDOS") }
fn init_as_clip<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> { init_as(a, cpu.areg(0), NativeHook::Clip, "InitIFFasClip") }

/// OpenClipboard(d0 = unitNumber) -> ClipboardHandle token (non-zero) when host clipboard is attached & granted, else 0.
fn open_clipboard<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let unit = cpu.dreg(0);
    let Some(clip) = a.clip_host.as_mut() else {
        cpu.set_d0(0);
        return Ok(format!("[iffparse] OpenClipboard({unit}) -> NULL (no host clipboard)"));
    };
    match clip.generation() {
        Ok(_) => {
            let tok = 0xC110_0000 | (unit & 0xFFFF) | 1;
            cpu.set_d0(tok);
            Ok(format!("[iffparse] OpenClipboard({unit}) -> {tok:#x}"))
        }
        Err(e) => {
            cpu.set_d0(0);
            a.refused.push(format!("iffparse.library/OpenClipboard: host clipboard {}", e.name()));
            Ok(format!("[iffparse] OpenClipboard({unit}) REFUSED: {}", e.name()))
        }
    }
}

/// CloseClipboard(a0 = clipHandle): stateless token release.
fn close_clipboard<U: GuestUi>(_a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let h = cpu.areg(0);
    Ok(format!("[iffparse] CloseClipboard({h:#x})"))
}

/// OpenIFF(a0 = iff, d0 = rwMode) -> 0 or an IFFERR code. A read-mode stream must start with a FORM, LIST or CAT header.
fn open_iff<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (iff, rw) = (cpu.areg(0), cpu.dreg(0));
    if iff == 0 { cpu.set_d0(IFFERR_NOMEM as u32); return Ok("[iffparse] OpenIFF NULL -> NOMEM".into()); }
    handle(a, "OpenIFF", iff)?;
    let kind = stream_kind(a, iff)?;
    match kind {
        Stream::None => { cpu.set_d0(IFFERR_NOHOOK as u32); return Ok(format!("[iffparse] OpenIFF {iff:#x} -> NOHOOK (no InitIFF)")); }
        Stream::Clip => {
            let Some(clip) = a.clip_host.as_mut() else {
                return refuse(a, cpu, "OpenIFF", "InitIFFasClip: the clipboard stream needs clipboard.device (no host clipboard attached)".into(), IFFERR_NOHOOK);
            };
            if rw & IFFF_RWBITS == 0 {
                let entry = match clip.paste() {
                    Ok(v) => v,
                    Err(e) => return refuse(a, cpu, "OpenIFF", format!("InitIFFasClip: host clipboard {}", e.name()), IFFERR_NOHOOK),
                };
                let data = match entry {
                    None => Vec::new(),
                    Some((mime, payload)) if mime == "application/x-amiga-iff" => payload,
                    Some((mime, payload)) if mime == "text/plain" || mime.starts_with("text/plain;") => encode_ftxt(&payload),
                    Some(_) => Vec::new(),
                };
                a.iff.handles.get_mut(&iff).expect("handle").clip_buf = Some(ClipBuf { data, pos: 0, write: false });
            } else {
                if let Err(e) = clip.generation() {
                    return refuse(a, cpu, "OpenIFF", format!("InitIFFasClip: host clipboard {}", e.name()), IFFERR_NOHOOK);
                }
                a.iff.handles.get_mut(&iff).expect("handle").clip_buf = Some(ClipBuf { data: Vec::new(), pos: 0, write: true });
            }
        }
        Stream::Custom(h) => return refuse(a, cpu, "OpenIFF", format!("the stream hook {h:#x} is the program's own; guest stream callbacks are not modelled (InitIFFasDOS is)"), IFFERR_NOHOOK),
        Stream::Dos => {}
    }
    let mut err = 0;
    let stream = rd(a, iff + IH_STREAM)?;
    let fl = flags(a, iff)?;
    if rw & IFFF_RWBITS == 0 && (stream != 0 || kind == Stream::Clip) {
        err = get_chunk_header(a, iff)?;
        if err == 0 {
            set_state(a, iff, PState::Composite)?;
            let cn = top(a, iff);
            if a.iff.cns[&cn].composite {
                wr(a, iff + IH_FLAGS, (fl & !IFFF_RWBITS) | (rw & IFFF_RWBITS) | IFFF_OPEN | IFFF_NEWFILE)?;
            } else {
                err = IFFERR_NOTIFF;
                pop_context(a, iff)?; // a plain chunk has no LCIs yet
            }
        } else {
            if err == IFFERR_MANGLED { err = IFFERR_NOTIFF; }
            seek_to_start(a, iff); // IFFCMD_CLEANUP
            if kind == Stream::Clip {
                if let Some(h) = a.iff.handles.get_mut(&iff) { h.clip_buf = None; }
            }
        }
    } else {
        wr(a, iff + IH_FLAGS, (fl & !IFFF_RWBITS) | (rw & IFFF_RWBITS) | IFFF_OPEN | IFFF_NEWFILE)?;
    }
    cpu.set_d0(err as u32);
    Ok(format!("[iffparse] OpenIFF {iff:#x} mode={rw} -> {err}"))
}

fn seek_to_start<U: GuestUi>(a: &mut Amiga<U>, iff: u32) {
    if matches!(stream_kind(a, iff), Ok(Stream::Clip)) {
        if let Some(cb) = a.iff.handles.get_mut(&iff).and_then(|h| h.clip_buf.as_mut()) { cb.pos = 0; }
        return;
    }
    if let Ok(bptr) = rd(a, iff + IH_STREAM) { if let Some(f) = a.fs.files.get_mut(&(bptr << 2)) { f.pos = 0; } }
}

/// CloseIFF(a0 = iff): pops every open chunk (stopping at the first error), then flushes clip write buffer / CLEANUP.
fn close_iff<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let iff = cpu.areg(0);
    if iff == 0 { return Ok("[iffparse] CloseIFF NULL".into()); }
    handle(a, "CloseIFF", iff)?;
    let fl = flags(a, iff)?;
    if fl & IFFF_OPEN == 0 { return Ok(format!("[iffparse] CloseIFF {iff:#x} (not open)")); }
    wr(a, iff + IH_FLAGS, fl & !IFFF_OPEN)?;
    let mut purge = Vec::new();
    loop {
        if depth(a, iff)? == 0 { break; }
        let (err, p) = crate::iff_chunk::pop_chunk_core(a, iff)?;
        purge.extend(p);
        if err != 0 { break; }
    }
    if matches!(stream_kind(a, iff), Ok(Stream::Clip)) {
        if let Some(cb) = a.iff.handles.get_mut(&iff).and_then(|h| h.clip_buf.take()) {
            if cb.write && !cb.data.is_empty() {
                if let Some(clip) = a.clip_host.as_mut() {
                    let res = if let Some(text) = decode_ftxt(&cb.data) {
                        clip.copy("text/plain;charset=utf-8", &text)
                    } else {
                        clip.copy("application/x-amiga-iff", &cb.data)
                    };
                    if let Err(e) = res {
                        a.refused.push(format!("iffparse.library/CloseIFF: host clipboard {}", e.name()));
                    }
                }
            }
        }
    } else {
        seek_to_start(a, iff);
    }
    let next = crate::iff_lci::purge_chain(a, cpu, purge, alloc::boxed::Box::new(|_, _, _| Ok(crate::guest_call::Next::Return(0))))?;
    crate::guest_call::start(a, cpu, next)?;
    Ok(format!("[iffparse] CloseIFF {iff:#x}"))
}

fn good_id_lvo<U: GuestUi>(_: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> { let id = cpu.dreg(0); cpu.set_d0(good_id(id) as u32); Ok(format!("[iffparse] GoodID {id:#x}")) }
fn good_type_lvo<U: GuestUi>(_: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> { let id = cpu.dreg(0); cpu.set_d0(good_type(id) as u32); Ok(format!("[iffparse] GoodType {id:#x}")) }

/// IDtoStr(d0 = id, a0 = buffer of 5) -> the buffer: the four characters and a NUL.
fn id_to_str<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (id, buf) = (cpu.dreg(0), cpu.areg(0));
    let mut b = id.to_be_bytes().to_vec();
    b.push(0);
    a.mem.write_bytes(buf, &b).map_err(m("IDtoStr buffer"))?;
    cpu.set_d0(buf);
    Ok(format!("[iffparse] IDtoStr {id:#x}"))
}
