use retro_guest_mem::GuestMem;
use retro_hunk::*;
use retro_hunk::writer::{be16, be32, pad4};

fn header(sizes: &[u32]) -> Vec<u8> {
    let mut f = Vec::new();
    let n = sizes.len() as u32;
    for x in [HUNK_HEADER, 0, n, 0, n.saturating_sub(1)] { be32(&mut f, x); }
    for &s in sizes { be32(&mut f, s); }
    f
}

fn section(f: &mut Vec<u8>, id: u32, payload: &[u8]) {
    let mut b = payload.to_vec();
    pad4(&mut b);
    be32(f, id);
    be32(f, (b.len() / 4) as u32);
    f.extend_from_slice(&b);
}

fn reloc32short(f: &mut Vec<u8>, target: u16, offsets: &[u16]) {
    be32(f, HUNK_RELOC32SHORT);
    be16(f, offsets.len() as u16);
    be16(f, target);
    for &o in offsets { be16(f, o); }
    be16(f, 0); // terminator
    let words = offsets.len() as u32 + 3;
    if words % 2 == 1 { be16(f, 0); }
}

fn drel32(f: &mut Vec<u8>, target: u16, offsets: &[u16]) {
    be32(f, HUNK_DREL32);
    be16(f, offsets.len() as u16);
    be16(f, target);
    for &o in offsets { be16(f, o); }
    be16(f, 0); // terminator
    let words = offsets.len() as u32 + 3;
    if words % 2 == 1 { be16(f, 0); }
}

fn hunk_name(f: &mut Vec<u8>, longwords: &[u32]) {
    be32(f, HUNK_NAME);
    be32(f, longwords.len() as u32);
    for &w in longwords { be32(f, w); }
}

fn hunk_symbol(f: &mut Vec<u8>, entries: &[(Vec<u8>, u32)]) {
    be32(f, HUNK_SYMBOL);
    for (name, value) in entries {
        let mut padded = name.clone();
        pad4(&mut padded);
        be32(f, (padded.len() / 4) as u32);
        for chunk in padded.chunks(4) {
            be32(f, u32::from_be_bytes(chunk.try_into().unwrap()));
        }
        be32(f, *value);
    }
    be32(f, 0); // terminator
}

fn hunk_debug(f: &mut Vec<u8>, longwords: &[u32]) {
    be32(f, HUNK_DEBUG);
    be32(f, longwords.len() as u32);
    for &w in longwords { be32(f, w); }
}

#[test]
fn reloc32short_even() {
    let code = 0x12345678u32.to_be_bytes().to_vec();
    let data = 0xAABBCCDDu32.to_be_bytes().to_vec();
    let mut f = header(&[1, 1]);
    section(&mut f, HUNK_CODE, &code);
    reloc32short(&mut f, 1, &[0]);
    be32(&mut f, HUNK_END);
    section(&mut f, HUNK_DATA, &data);
    be32(&mut f, HUNK_END);

    let mut mem = GuestMem::new(0x10000);
    let loaded = load(&mut mem, &f, 0x100).unwrap();
    assert_eq!(loaded.bases.len(), 2);
    let patched = mem.read_u32(loaded.bases[0]).unwrap();
    assert_eq!(patched, 0x12345678u32.wrapping_add(loaded.bases[1]));
}

#[test]
fn reloc32short_odd() {
    let mut code = vec![0; 8];
    code[0..4].copy_from_slice(&0x11111111u32.to_be_bytes());
    code[4..8].copy_from_slice(&0x22222222u32.to_be_bytes());
    let data = 0xAABBCCDDu32.to_be_bytes().to_vec();
    let mut f = header(&[2, 1]);
    section(&mut f, HUNK_CODE, &code);
    reloc32short(&mut f, 1, &[0, 4]);
    be32(&mut f, HUNK_END);
    section(&mut f, HUNK_DATA, &data);
    be32(&mut f, HUNK_END);

    let mut mem = GuestMem::new(0x10000);
    let loaded = load(&mut mem, &f, 0x100).unwrap();
    assert_eq!(mem.read_u32(loaded.bases[0]).unwrap(), 0x11111111u32.wrapping_add(loaded.bases[1]));
    assert_eq!(mem.read_u32(loaded.bases[0] + 4).unwrap(), 0x22222222u32.wrapping_add(loaded.bases[1]));
}

#[test]
fn drel32_even() {
    let code = 0x12345678u32.to_be_bytes().to_vec();
    let data = 0xAABBCCDDu32.to_be_bytes().to_vec();
    let mut f = header(&[1, 1]);
    section(&mut f, HUNK_CODE, &code);
    drel32(&mut f, 1, &[0]);
    be32(&mut f, HUNK_END);
    section(&mut f, HUNK_DATA, &data);
    be32(&mut f, HUNK_END);

    let mut mem = GuestMem::new(0x10000);
    let loaded = load(&mut mem, &f, 0x100).unwrap();
    assert_eq!(loaded.bases.len(), 2);
    let patched = mem.read_u32(loaded.bases[0]).unwrap();
    assert_eq!(patched, 0x12345678u32.wrapping_add(loaded.bases[1]));
}

#[test]
fn drel32_odd() {
    let mut code = vec![0; 8];
    code[0..4].copy_from_slice(&0x11111111u32.to_be_bytes());
    code[4..8].copy_from_slice(&0x22222222u32.to_be_bytes());
    let data = 0xAABBCCDDu32.to_be_bytes().to_vec();
    let mut f = header(&[2, 1]);
    section(&mut f, HUNK_CODE, &code);
    drel32(&mut f, 1, &[0, 4]);
    be32(&mut f, HUNK_END);
    section(&mut f, HUNK_DATA, &data);
    be32(&mut f, HUNK_END);

    let mut mem = GuestMem::new(0x10000);
    let loaded = load(&mut mem, &f, 0x100).unwrap();
    assert_eq!(mem.read_u32(loaded.bases[0]).unwrap(), 0x11111111u32.wrapping_add(loaded.bases[1]));
    assert_eq!(mem.read_u32(loaded.bases[0] + 4).unwrap(), 0x22222222u32.wrapping_add(loaded.bases[1]));
}

#[test]
fn reloc32short_outside_ram() {
    let code = 0x12345678u32.to_be_bytes().to_vec();
    let data = 0xAABBCCDDu32.to_be_bytes().to_vec();
    let mut f = header(&[1, 1]);
    section(&mut f, HUNK_CODE, &code);
    reloc32short(&mut f, 1, &[4]);
    be32(&mut f, HUNK_END);
    section(&mut f, HUNK_DATA, &data);
    be32(&mut f, HUNK_END);

    let mut mem = GuestMem::new(0x10000);
    let err = load(&mut mem, &f, 0x100).unwrap_err();
    assert!(matches!(err, HunkError::RelocOutsideRam { hunk: 0, offset: 4 }));
}

#[test]
fn reloc32short_unknown_target() {
    let code = 0x12345678u32.to_be_bytes().to_vec();
    let data = 0xAABBCCDDu32.to_be_bytes().to_vec();
    let mut f = header(&[1, 1]);
    section(&mut f, HUNK_CODE, &code);
    reloc32short(&mut f, 99, &[0]);
    be32(&mut f, HUNK_END);
    section(&mut f, HUNK_DATA, &data);
    be32(&mut f, HUNK_END);

    let mut mem = GuestMem::new(0x10000);
    let err = load(&mut mem, &f, 0x100).unwrap_err();
    assert!(matches!(err, HunkError::RelocToUnknownHunk { target: 99 }));
}

#[test]
fn name_symbol_debug_between_code_and_end() {
    let code = vec![0x01, 0x02, 0x03, 0x04];
    let mut f = header(&[1]);
    section(&mut f, HUNK_CODE, &code);
    hunk_name(&mut f, &[0xABCDEF01]);
    hunk_symbol(&mut f, &[(b"foo\0".to_vec(), 0xDEAD), (b"bar\0".to_vec(), 0xBEEF)]);
    hunk_debug(&mut f, &[0x11111111, 0x22222222]);
    be32(&mut f, HUNK_END);

    let mut mem = GuestMem::new(0x10000);
    let loaded = load(&mut mem, &f, 0x100).unwrap();
    assert_eq!(mem.read_u32(loaded.bases[0]).unwrap(), 0x01020304);
}

fn sweep(file: &[u8]) {
    for pos in 0..file.len() {
        let mut mem = GuestMem::new(0x10000);
        let result = load(&mut mem, &file[..pos], 0x100);
        assert!(result.is_err(), "truncation at byte {} succeeded unexpectedly", pos);
    }
}

#[test]
fn truncation_sweep_reloc32short_even() {
    let code = 0x12345678u32.to_be_bytes().to_vec();
    let data = 0xAABBCCDDu32.to_be_bytes().to_vec();
    let mut f = header(&[1, 1]);
    section(&mut f, HUNK_CODE, &code);
    reloc32short(&mut f, 1, &[0]);
    be32(&mut f, HUNK_END);
    section(&mut f, HUNK_DATA, &data);
    be32(&mut f, HUNK_END);
    sweep(&f);
}

#[test]
fn truncation_sweep_reloc32short_odd() {
    let mut code = vec![0; 8];
    code[0..4].copy_from_slice(&0x11111111u32.to_be_bytes());
    code[4..8].copy_from_slice(&0x22222222u32.to_be_bytes());
    let data = 0xAABBCCDDu32.to_be_bytes().to_vec();
    let mut f = header(&[2, 1]);
    section(&mut f, HUNK_CODE, &code);
    reloc32short(&mut f, 1, &[0, 4]);
    be32(&mut f, HUNK_END);
    section(&mut f, HUNK_DATA, &data);
    be32(&mut f, HUNK_END);
    sweep(&f);
}

#[test]
fn truncation_sweep_drel32() {
    let code = 0x12345678u32.to_be_bytes().to_vec();
    let data = 0xAABBCCDDu32.to_be_bytes().to_vec();
    let mut f = header(&[1, 1]);
    section(&mut f, HUNK_CODE, &code);
    drel32(&mut f, 1, &[0]);
    be32(&mut f, HUNK_END);
    section(&mut f, HUNK_DATA, &data);
    be32(&mut f, HUNK_END);
    sweep(&f);
}

#[test]
fn truncation_sweep_name() {
    let code = vec![0x01, 0x02, 0x03, 0x04];
    let mut f = header(&[1]);
    section(&mut f, HUNK_CODE, &code);
    hunk_name(&mut f, &[0xABCDEF01]);
    be32(&mut f, HUNK_END);
    sweep(&f);
}

#[test]
fn truncation_sweep_symbol() {
    let code = vec![0x01, 0x02, 0x03, 0x04];
    let mut f = header(&[1]);
    section(&mut f, HUNK_CODE, &code);
    hunk_symbol(&mut f, &[(b"foo\0".to_vec(), 0xDEAD), (b"bar\0".to_vec(), 0xBEEF)]);
    be32(&mut f, HUNK_END);
    sweep(&f);
}

#[test]
fn truncation_sweep_debug() {
    let code = vec![0x01, 0x02, 0x03, 0x04];
    let mut f = header(&[1]);
    section(&mut f, HUNK_CODE, &code);
    hunk_debug(&mut f, &[0x11111111, 0x22222222]);
    be32(&mut f, HUNK_END);
    sweep(&f);
}

#[test]
fn constant_sanity() {
    assert_eq!(HUNK_NAME, 0x3E8);
    assert_eq!(HUNK_SYMBOL, 0x3F0);
    assert_eq!(HUNK_DEBUG, 0x3F1);
    assert_eq!(HUNK_DREL32, 0x3F7);
    assert_eq!(HUNK_RELOC32SHORT, 0x3FC);
}
