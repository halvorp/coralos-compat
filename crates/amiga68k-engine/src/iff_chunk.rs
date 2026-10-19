//! iffparse.library, part 2: chunks. PushChunk / PopChunk (reading and writing), ReadChunkBytes / WriteChunkBytes, the record
//! forms, CurrentChunk and ParentChunk. The rules, errors and size arithmetic are AROS pushchunk.c / popchunk.c / readchunkbytes.c /
//! writechunkbytes.c / readchunkrecords.c / writechunkrecords.c (APL, read for facts only): a write-mode PushChunk checks the syntax of
//! the file it builds (the first chunk must be FORM, LIST or CAT; PROP only inside a LIST; plain chunks only inside a FORM or PROP),
//! writes the chunk header (and a composite chunk's type), and a chunk pushed with IFFSIZE_UNKNOWN gets its real size patched in
//! by PopChunk, which also pads an odd chunk to an even length.
use alloc::vec::Vec;
use alloc::string::String;
use alloc::format;
use crate::iff::*;
use crate::libtable::LibraryBuilder;
use crate::{machine::Amiga, Refusal};
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.named("PushChunk", push_chunk::<U>).named("PopChunk", pop_chunk::<U>).named("ReadChunkBytes", read_chunk_bytes::<U>)
        .named("WriteChunkBytes", write_chunk_bytes::<U>).named("ReadChunkRecords", read_chunk_records::<U>).named("WriteChunkRecords", write_chunk_records::<U>)
        .named("CurrentChunk", current_chunk::<U>).named("ParentChunk", parent_chunk::<U>)
}

fn write_long<U: GuestUi>(a: &mut Amiga<U>, iff: u32, v: u32) -> i32 { write_stream(a, iff, &v.to_be_bytes()) }

/// PushChunk(a0 = iff, d0 = type, d1 = id, d2 = size) -> 0 or an IFFERR code.
fn push_chunk<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (iff, ty, id, size) = (cpu.areg(0), cpu.dreg(0), cpu.dreg(1), cpu.dreg(2) as i32);
    handle(a, "PushChunk", iff)?;
    let err = push(a, iff, ty, id, size)?;
    cpu.set_d0(err as u32);
    Ok(format!("[iffparse] PushChunk {iff:#x} {} {} size={size} -> {err}", fourcc(ty), fourcc(id)))
}

pub fn fourcc(id: u32) -> String { id.to_be_bytes().iter().map(|&c| if (0x20..0x7f).contains(&c) { c as char } else { '?' }).collect() }

fn push<U: GuestUi>(a: &mut Amiga<U>, iff: u32, ty: u32, id: u32, size: i32) -> Result<i32, Refusal> {
    if flags(a, iff)? & IFFF_WRITE == 0 { return get_chunk_header(a, iff); }
    let mut scan = 0;
    let pcn = if depth(a, iff)? != 0 { Some(top(a, iff)) } else { None };
    if pcn.is_none() {
        let fl = flags(a, iff)?;
        if fl & IFFF_NEWFILE != 0 { wr(a, iff + IH_FLAGS, fl & !IFFF_NEWFILE)?; } else { return Ok(IFFERR_EOF); }
    }
    if !good_id(id) { return Ok(IFFERR_SYNTAX); }
    let composite = matches!(id, ID_FORM | ID_LIST | ID_CAT | ID_PROP);
    match pcn {
        None if !matches!(id, ID_FORM | ID_LIST | ID_CAT) => return Ok(IFFERR_NOTIFF),
        None => {}
        Some(p) => {
            let pid = rd(a, p + CN_ID)?;
            if id == ID_PROP { if pid != ID_LIST { return Ok(IFFERR_SYNTAX); } }
            else if composite { if !good_type(ty) { return Ok(IFFERR_NOTIFF); } }
            else if pid != ID_FORM && pid != ID_PROP { return Ok(IFFERR_SYNTAX); }
        }
    }
    for v in [id, size as u32] { let e = write_long(a, iff, v); if e < 0 { return Ok(e); } }
    if composite {
        let e = write_long(a, iff, ty);
        if e < 0 { return Ok(e); }
        scan = 4;
    }
    push_context(a, iff, ty, id, size, scan)
}

/// PopChunk's work without the purge hooks: (error, the popped node's LCIs for the caller to purge).
pub fn pop_chunk_core<U: GuestUi>(a: &mut Amiga<U>, iff: u32) -> Result<(i32, Vec<u32>), Refusal> {
    let cn = top(a, iff);
    let writing = flags(a, iff)? & IFFF_WRITE != 0;
    let mut size = 0u32;
    if writing {
        let (csize, scan) = (rd(a, cn + CN_SIZE_F)? as i32, rd(a, cn + CN_SCAN)? as i32);
        if csize == IFFSIZE_UNKNOWN {
            let e = seek_stream(a, iff, -(scan + 4));
            if e != 0 { return Ok((e, Vec::new())); }
            size = scan as u32;
            let e = write_long(a, iff, size);
            if e < 0 { return Ok((e, Vec::new())); }
            let e = seek_stream(a, iff, size as i32);
            if e != 0 { return Ok((e, Vec::new())); }
        } else { size = csize as u32; }
        if size % 2 == 1 {
            let e = write_stream(a, iff, &[0]);
            if e < 0 { return Ok((e, Vec::new())); }
            size += 1;
        }
    }
    let purge = pop_context(a, iff)?;
    if writing {
        let parent = top(a, iff);
        let scan = rd(a, parent + CN_SCAN)?;
        wr(a, parent + CN_SCAN, scan.wrapping_add(size).wrapping_add(8))?;
    }
    Ok((0, purge))
}

/// PopChunk(a0 = iff) -> 0 or an IFFERR code.
fn pop_chunk<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let iff = cpu.areg(0);
    handle(a, "PopChunk", iff)?;
    if depth(a, iff)? == 0 { cpu.set_d0(IFFERR_NOSCOPE as u32); return Ok(format!("[iffparse] PopChunk {iff:#x} with no chunk open -> NOSCOPE")); }
    let (err, purge) = pop_chunk_core(a, iff)?;
    let next = crate::iff_lci::purge_chain(a, cpu, purge, alloc::boxed::Box::new(move |_, _, _| Ok(crate::guest_call::Next::Return(err as u32))))?;
    crate::guest_call::start(a, cpu, next)?;
    Ok(format!("[iffparse] PopChunk {iff:#x} -> {err}"))
}

/// The bytes of the top chunk still to read.
fn left<U: GuestUi>(a: &Amiga<U>, cn: u32) -> Result<i32, Refusal> { Ok((rd(a, cn + CN_SIZE_F)? as i32).wrapping_sub(rd(a, cn + CN_SCAN)? as i32)) }
fn add_scan<U: GuestUi>(a: &mut Amiga<U>, cn: u32, n: i32) -> Result<(), Refusal> { let s = rd(a, cn + CN_SCAN)?; wr(a, cn + CN_SCAN, s.wrapping_add(n as u32)) }

/// Read `n` (already truncated) bytes of the top chunk into `buf`: the count read, or an IFFERR code.
fn read_into<U: GuestUi>(a: &mut Amiga<U>, iff: u32, buf: u32, n: i32) -> Result<i32, Refusal> {
    if n < 0 { return Ok(IFFERR_READ); }
    match read_stream(a, iff, n as u32) {
        Ok(bytes) => { a.mem.write_bytes(buf, &bytes).map_err(m("ReadChunkBytes buffer"))?; Ok(n) }
        Err(e) => Ok(e),
    }
}

/// ReadChunkBytes without the register plumbing (PropChunk and CollectionChunk read their chunk with it).
pub fn read_chunk_core<U: GuestUi>(a: &mut Amiga<U>, iff: u32, buf: u32, want: i32) -> Result<i32, Refusal> {
    let cn = top(a, iff);
    let n = want.min(left(a, cn)?);
    let got = read_into(a, iff, buf, n)?;
    if got > 0 { add_scan(a, cn, got)?; }
    Ok(got)
}

/// ReadChunkBytes(a0 = iff, a1 = buffer, d0 = bytes) -> bytes read (cut to what is left of the chunk) or an IFFERR code.
fn read_chunk_bytes<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (iff, buf, want) = (cpu.areg(0), cpu.areg(1), cpu.dreg(0) as i32);
    handle(a, "ReadChunkBytes", iff)?;
    let got = read_chunk_core(a, iff, buf, want)?;
    cpu.set_d0(got as u32);
    Ok(format!("[iffparse] ReadChunkBytes {iff:#x} {want} -> {got}"))
}

/// WriteChunkBytes(a0 = iff, a1 = buffer, d0 = bytes) -> bytes written (cut to the chunk's size when it is known) or an IFFERR code.
fn write_chunk_bytes<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (iff, buf, want) = (cpu.areg(0), cpu.areg(1), cpu.dreg(0) as i32);
    handle(a, "WriteChunkBytes", iff)?;
    let cn = top(a, iff);
    let n = if rd(a, cn + CN_SIZE_F)? as i32 != IFFSIZE_UNKNOWN { want.min(left(a, cn)?) } else { want };
    let got = write_from(a, iff, buf, n)?;
    if got > 0 { add_scan(a, cn, got)?; }
    cpu.set_d0(got as u32);
    Ok(format!("[iffparse] WriteChunkBytes {iff:#x} {want} -> {got}"))
}

fn write_from<U: GuestUi>(a: &mut Amiga<U>, iff: u32, buf: u32, n: i32) -> Result<i32, Refusal> {
    if n < 0 { return Ok(IFFERR_WRITE); }
    let bytes: Vec<u8> = (0..n as u32).map(|i| a.mem.read_u8(buf + i).map_err(m("WriteChunkBytes buffer"))).collect::<Result<_, _>>()?;
    let e = write_stream(a, iff, &bytes);
    Ok(if e < 0 { e } else { n })
}

/// ReadChunkRecords(a0 = iff, a1 = buffer, d0 = bytesPerRecord, d1 = numRecords) -> whole records read, or an IFFERR code.
fn read_chunk_records<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (iff, buf, bpr, num) = (cpu.areg(0), cpu.areg(1), cpu.dreg(0) as i32, cpu.dreg(1) as i32);
    handle(a, "ReadChunkRecords", iff)?;
    if bpr <= 0 { return Err(Refusal(format!("ReadChunkRecords: bytesPerRecord {bpr} (the original divides by it)"))); }
    let cn = top(a, iff);
    let mut want = bpr.wrapping_mul(num);
    let l = left(a, cn)?;
    if want > l { want = l - l % bpr; }
    let got = read_into(a, iff, buf, want)?;
    let r = if got < 0 { got } else { add_scan(a, cn, got)?; got / bpr };
    cpu.set_d0(r as u32);
    Ok(format!("[iffparse] ReadChunkRecords {iff:#x} {num}x{bpr} -> {r}"))
}

/// WriteChunkRecords(a0 = iff, a1 = buffer, d0 = bytesPerRecord, d1 = numRecords) -> whole records written, or an IFFERR code.
fn write_chunk_records<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (iff, buf, bpr, num) = (cpu.areg(0), cpu.areg(1), cpu.dreg(0) as i32, cpu.dreg(1) as i32);
    handle(a, "WriteChunkRecords", iff)?;
    if bpr <= 0 { return Err(Refusal(format!("WriteChunkRecords: bytesPerRecord {bpr} (the original divides by it)"))); }
    let cn = top(a, iff);
    let mut want = bpr.wrapping_mul(num);
    if rd(a, cn + CN_SIZE_F)? as i32 != IFFSIZE_UNKNOWN {
        let l = left(a, cn)?;
        if want > l { want = l - l % bpr; }
    }
    let got = write_from(a, iff, buf, want)?;
    let r = if got < 0 { got } else { add_scan(a, cn, got)?; got / bpr };
    cpu.set_d0(r as u32);
    Ok(format!("[iffparse] WriteChunkRecords {iff:#x} {num}x{bpr} -> {r}"))
}

/// CurrentChunk(a0 = iff) -> the innermost open ContextNode, or NULL when no chunk is open.
fn current_chunk<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let iff = cpu.areg(0);
    handle(a, "CurrentChunk", iff)?;
    let cn = if depth(a, iff)? != 0 { top(a, iff) } else { 0 };
    cpu.set_d0(cn);
    Ok(format!("[iffparse] CurrentChunk {iff:#x} -> {cn:#x}"))
}

/// The ContextNode enclosing `cn` (NULL for the outermost chunk: the hidden default node is not the program's).
pub fn parent_of<U: GuestUi>(a: &Amiga<U>, cn: u32) -> Option<u32> {
    a.iff.handles.values().find_map(|h| h.stack.iter().position(|&n| n == cn).and_then(|i| if i >= 2 { Some(h.stack[i - 1]) } else { None }))
}

/// ParentChunk(a0 = contextNode) -> its parent, or NULL.
fn parent_chunk<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let cn = cpu.areg(0);
    if !a.iff.cns.contains_key(&cn) { return Err(Refusal(format!("ParentChunk: {cn:#x} is not a ContextNode of an open IFFHandle"))); }
    let p = parent_of(a, cn).unwrap_or(0);
    cpu.set_d0(p);
    Ok(format!("[iffparse] ParentChunk {cn:#x} -> {p:#x}"))
}
