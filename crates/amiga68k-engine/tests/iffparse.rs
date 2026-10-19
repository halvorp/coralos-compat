//! iffparse.library through compiled 68k programs (tests/guest/c/iffread.c, iffwrite.c): a FORM with nested chunks written by the
//! guest is compared with a host parse of the bytes, and a host-built IFF is read, stepped, collected and stopped by the guest.
mod common;
use amiga68k_engine::Amiga;
use common::*;
use guestui_abi::RecordingGuestUi;
use retro_hunk::writer::{build, Hunk};

const READ: &[u8] = include_bytes!("guest/iffread.bin");
const WRITE: &[u8] = include_bytes!("guest/iffwrite.bin");
const CLIP: &[u8] = include_bytes!("guest/iffclip.bin");

type M = Amiga<RecordingGuestUi>;

fn run_prog(a: &mut M, blob: &[u8]) -> Vec<u32> {
    let r = a.run_hunk(&build(&[Hunk::code(blob.to_vec(), vec![])], None), 60_000_000).unwrap();
    assert!(r.stats.halted && r.stats.bus_faults == 0, "{:?} {:?}\n{:#?}", r.stats, a.refused, a.log);
    (0..128).map(|i| a.mem.read_u32(RES + 4 * i).unwrap()).collect()
}
fn put(a: &mut M, path: &str, bytes: Vec<u8>) { amiga68k_engine::fs::write_file(a, path, bytes).unwrap(); }
fn get(a: &M, path: &str) -> Vec<u8> { let n = a.fs.vol.resolve(0, path).unwrap(); a.fs.vol.file_data(n).unwrap().to_vec() }

fn chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut v = id.to_vec();
    v.extend((data.len() as u32).to_be_bytes());
    v.extend(data);
    if data.len() % 2 == 1 { v.push(0); }
    v
}
fn group(kind: &[u8; 4], ty: &[u8; 4], kids: &[Vec<u8>]) -> Vec<u8> {
    let body: Vec<u8> = kids.concat();
    let mut v = kind.to_vec();
    v.extend((4 + body.len() as u32).to_be_bytes());
    v.extend(ty);
    v.extend(body);
    v
}

/// A tiny independent IFF reader: the structure of `bytes`, every size and pad byte checked.
#[derive(Debug, PartialEq)]
enum N { Group { kind: String, ty: String, kids: Vec<N> }, Chunk { id: String, data: Vec<u8> } }
fn parse(b: &[u8]) -> Result<Vec<N>, String> {
    let (mut i, mut out) = (0, vec![]);
    while i < b.len() {
        if i + 8 > b.len() { return Err(format!("truncated header at {i}")); }
        let id = String::from_utf8_lossy(&b[i..i + 4]).into_owned();
        let size = u32::from_be_bytes(b[i + 4..i + 8].try_into().unwrap()) as usize;
        if i + 8 + size > b.len() { return Err(format!("{id} at {i}: size {size} runs past the end")); }
        let body = &b[i + 8..i + 8 + size];
        out.push(if ["FORM", "LIST", "CAT ", "PROP"].contains(&id.as_str()) {
            N::Group { kind: id, ty: String::from_utf8_lossy(&body[..4]).into_owned(), kids: parse(&body[4..])? }
        } else { N::Chunk { id, data: body.to_vec() } });
        i += 8 + size;
        if size % 2 == 1 { if b.get(i) != Some(&0) { return Err(format!("missing pad byte at {i}")); } i += 1; }
    }
    Ok(out)
}
fn ck(id: &str, data: &[u8]) -> N { N::Chunk { id: id.into(), data: data.to_vec() } }

fn neg(v: i32) -> u32 { v as u32 }

#[test]
fn a_guest_writes_a_nested_form_and_reads_it_back_and_a_host_parse_agrees() {
    let mut a = machine();
    let r = run_prog(&mut a, WRITE);
    assert_eq!((r[0], r[1], r[2], r[3], r[4], r[5], r[6]), (0, 0, 0, 6, 6, 0, 0), "OpenIFF/PushChunk/WriteChunkBytes: the size-6 chunk takes 6 bytes (scan 6) and refuses a 7th");
    assert_eq!((r[7], r[8], r[9], r[10], r[11], r[12]), (0, 5, 0, 0, 3, 0), "the unknown-size chunks; WriteChunkRecords reports whole records");
    assert_eq!((r[13], r[14], r[15], r[16], r[17], r[18]), (0, 0, 3, 0, 0, 0), "inner FORM, DATA, pops");
    assert_eq!(r[19], neg(-3), "PopChunk with nothing open is IFFERR_NOSCOPE");
    let bytes = get(&a, "RAM:out.iff");
    let expected = group(b"FORM", b"TEST", &[chunk(b"HEAD", b"abcdef"), chunk(b"ODDX", b"hello"), chunk(b"RECS", &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]), group(b"FORM", b"SUBF", &[chunk(b"DATA", b"xyz")])]);
    assert_eq!(bytes.len(), 84);
    assert_eq!(bytes, expected, "byte for byte the IFF the original library writes: patched sizes, even padding");
    assert_eq!(parse(&bytes).unwrap(), vec![N::Group { kind: "FORM".into(), ty: "TEST".into(), kids: vec![ck("HEAD", b"abcdef"), ck("ODDX", b"hello"), ck("RECS", &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]),
        N::Group { kind: "FORM".into(), ty: "SUBF".into(), kids: vec![ck("DATA", b"xyz")] }] }], "the independent host parser agrees on the structure");
    // A LIST with a PROP.
    assert_eq!((r[20], r[21], r[22], r[23], r[24]), (0, 0, 0, 2, 2));
    assert_eq!(get(&a, "RAM:list.iff"), group(b"LIST", b"TEST", &[group(b"PROP", b"TEST", &[chunk(b"HEAD", b"hi")]), group(b"FORM", b"TEST", &[chunk(b"DATA", b"yo")])]));
    // The write-mode syntax checks.
    assert_eq!((r[30], r[39], r[31], r[32], r[33], r[34], r[35], r[36]), (neg(-10), neg(-1), 0, neg(-9), neg(-9), neg(-10), 0, neg(-9)), "NOTIFF, then EOF (NEWFILE was used up), ok, SYNTAX, SYNTAX, NOTIFF, ok, SYNTAX");
    let bad = get(&a, "RAM:bad.iff");
    assert!(!bad.windows(4).any(|w| w == b"PROP" || w == b"ABCD"), "a refused PushChunk writes nothing: {bad:?}");
    assert_eq!(r[37], 1 + 8 + 16, "GoodID('FORM') yes, (' ABC') no, GoodType('ilbm') no, ('ILBM') yes, all-spaces is a valid ID");
    assert_eq!(r[38], 1, "IDtoStr");
    // The guest reads its own file back with ParseIFF(STEP).
    assert_eq!(r[40], 0);
    assert_eq!(r[41], 13, "FORM entry, HEAD/ODDX/RECS entry+exit, the inner FORM entry, DATA entry+exit, inner FORM exit, FORM exit, EOF");
    let seq: Vec<(i32, String)> = (0..13).map(|k| (r[50 + 2 * k] as i32, String::from_utf8_lossy(&r[51 + 2 * k].to_be_bytes()).into_owned())).collect();
    let want = [(0, "FORM"), (0, "HEAD"), (-2, "HEAD"), (0, "ODDX"), (-2, "ODDX"), (0, "RECS"), (-2, "RECS"), (0, "FORM"), (0, "DATA"), (-2, "DATA"), (-2, "FORM"), (-2, "FORM"), (-1, "\0\0\0\0")];
    assert_eq!(seq, want.iter().map(|(c, s)| (*c, s.to_string())).collect::<Vec<_>>(), "(code, current chunk) at every stop: 0 = entry, IFFERR_EOC = exit, IFFERR_EOF at the end");
    assert_eq!((r[42], r[43]), (5, 1), "ReadChunkBytes at the ODDX entry returned its 5 bytes, which are 'hello'");
    assert!(a.refused.is_empty(), "{:?}", a.refused);
}

fn in_file() -> Vec<u8> {
    group(b"FORM", b"TEST", &[chunk(b"NAME", b"Alpha"), chunk(b"CMNT", b"c1"), chunk(b"CMNT", b"c2"), chunk(b"NAME", b"Beta"), chunk(b"CMNT", b"c3"), chunk(b"BODY", b"01234567"), chunk(b"TAIL", b"end")])
}

#[test]
fn a_guest_scans_steps_collects_and_handles_a_host_built_iff() {
    let mut a = machine();
    put(&mut a, "RAM:in.iff", in_file());
    put(&mut a, "RAM:garbage.iff", b"hello world, this is not an IFF file at all, no.".to_vec());
    put(&mut a, "RAM:empty.iff", vec![]);
    put(&mut a, "RAM:badtype.iff", { let mut v = b"FORM".to_vec(); v.extend(12u32.to_be_bytes()); v.extend(b"ab!d"); v.extend([0u8; 8]); v });
    put(&mut a, "RAM:cut.iff", { let mut v = b"FORM".to_vec(); v.extend(16u32.to_be_bytes()); v.extend(b"TEST"); v.extend(b"NAME"); v.extend(100u32.to_be_bytes()); v });
    let r = run_prog(&mut a, READ);
    let i = |k: usize| r[k] as i32;
    assert_eq!((i(0), i(1), i(2), i(3), i(4)), (0, 0, 0, 0, 0), "OpenIFF, PropChunk, CollectionChunk, StopChunk; ParseIFF(SCAN) stops at BODY with 0");
    assert_eq!((i(5), i(6), i(7), i(8)), (1, 8, 0, 1), "CurrentChunk: BODY, size 8, scan 0, type inherited from the FORM");
    assert_eq!((i(9), i(10), i(11), i(12)), (1, 1, 1, 1), "ParentChunk is the FORM TEST; the outermost chunk has no parent; FindPropContext is that FORM");
    assert_eq!((i(13), i(14)), (4, 1), "FindProp: the later NAME ('Beta') replaced 'Alpha'");
    assert_eq!((i(15), i(16), i(17)), (3, 1, 1), "FindCollection: all three CMNT chunks, most recent first");
    assert_eq!(i(18), 1, "negative control: FindProp of a chunk never stored is NULL");
    assert_eq!((i(19), i(20), i(21), i(22), i(23)), (5, 5, 3, 0, 1), "ReadChunkBytes: 5, then the 3 that are left (not 10), then 0");
    assert_eq!((i(24), i(25), i(26)), (-1, 0, -1), "the rest of the file ends with IFFERR_EOF and no chunk open; parsing again stays at EOF");
    // The program's own hooks.
    assert_eq!((i(30), i(31), i(32)), (0, 0, -1));
    assert_eq!((i(61), i(64)), (2, 3), "the entry hook ran at both NAME chunks, the exit hook at all three CMNT chunks");
    assert_eq!((i(70), i(71)), (0xBEEF, 5), "A2 = the object given to EntryHandler, A1 = IFFCMD_ENTRY");
    assert_eq!((i(79), i(80)), (0xCAFE, 6), "A2 = the ExitHandler object, A1 = IFFCMD_EXIT");
    assert_eq!((i(33), i(34), i(62)), (0, 1, 1), "a hook answering IFF_RETURN2CLIENT stops ParseIFF with 0 inside BODY");
    assert_eq!((i(35), i(65)), (-77, 1), "any other answer is ParseIFF's result");
    // STEP and RAWSTEP.
    assert_eq!(i(36), 17);
    let seq: Vec<(i32, String)> = (0..17).map(|k| (r[84 + 2 * k] as i32, String::from_utf8_lossy(&r[85 + 2 * k].to_be_bytes()).into_owned())).collect();
    let mut want = vec![(0, "FORM")];
    for c in ["NAME", "CMNT", "CMNT", "NAME", "CMNT", "BODY", "TAIL"] { want.push((0, c)); want.push((-2, c)); }
    want.extend([(-2, "FORM"), (-1, "\0\0\0\0")]);
    assert_eq!(seq, want.iter().map(|(c, s)| (*c, s.to_string())).collect::<Vec<_>>(), "STEP stops at every entry (0) and exit (IFFERR_EOC)");
    assert_eq!(i(37), 17, "RAWSTEP stops at the same places");
    assert_eq!(i(61), 2, "negative control: RAWSTEP runs no handler (the entry hook count stayed 2)");
    // Errors.
    assert_eq!((i(40), i(41), i(42), i(43), i(44)), (-10, -10, -5, -10, -5), "garbage: NOTIFF (OpenIFF and ParseIFF); empty: READ; bad type: NOTIFF; a cut chunk: READ");
    assert_eq!((i(45), i(46), i(47)), (-11, -10, -3), "no InitIFF: NOHOOK; ParseIFF unopened: NOTIFF; PopChunk with nothing open: NOSCOPE");
    // Local items.
    assert_eq!((i(50), i(51), i(52), i(53), i(54), i(55)), (1, 1, 0, 1, 1, -3), "AllocLocalItem, LocalItemData, StoreLocalItem(ROOT), FindLocalItem; IFFSLI_PROP outside a FORM is NOSCOPE");
    assert_eq!(i(63), 1, "the program's purge hook ran once, when the handle was freed");
    assert_eq!((r[76], r[77]), (r[59], 7), "A2 = the item, A1 = IFFCMD_PURGELCI");
    assert!(a.refused.is_empty(), "{:?}", a.refused);
    assert_eq!(a.iff.handles.len(), 0, "every handle was freed");
    assert!(a.iff.lcis.is_empty() && a.iff.cns.is_empty(), "and nothing is left of its items or contexts: {:?} {:?}", a.iff.lcis, a.iff.cns.len());
}

#[test]
fn clipboard_and_custom_streams_are_named_refusals() {
    let mut a = machine();
    let r = run_prog(&mut a, CLIP);
    assert_eq!((r[0] as i32, r[1], r[2] as i32, r[3] as i32), (-11, 4, -11, -4), "NOHOOK for the clipboard stream and for a hook of the program's own; NOMEM for a NULL handle");
    assert_eq!(a.refused.len(), 2, "{:?}", a.refused);
    assert!(a.refused[0].starts_with("iffparse.library/OpenIFF:") && a.refused[0].contains("clipboard.device"), "{:?}", a.refused);
    assert!(a.refused[1].starts_with("iffparse.library/OpenIFF:") && a.refused[1].contains("guest stream callbacks are not modelled"), "{:?}", a.refused);
}
