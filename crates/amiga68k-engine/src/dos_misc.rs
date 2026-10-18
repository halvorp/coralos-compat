//! Smaller dos.library services programs like Directory Opus rely on: DateStamp / CompareDates / SetFileDate, Info,
//! and the buffered-I/O calls (FGetC, FPutC, UnGetC, FRead, FWrite, FGets), which here go straight to the handle
//! (no host-visible buffering to flush; the files are in memory).
//!
//! `struct DateStamp` is 3 longs (days since 1978-01-01, minutes, ticks of 1/50 s). `struct InfoData` is 36 bytes
//! (m68k-linux-gnu-gcc -mno-align-int over AROS dos/dos.h): id_NumSoftErrors 0, id_UnitNumber 4, id_DiskState 8,
//! id_NumBlocks 12, id_NumBlocksUsed 16, id_BytesPerBlock 20, id_DiskType 24, id_VolumeNode 28, id_InUse 32.
use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;
use crate::dos_err::*;
use crate::dos_io::{handle, write_bytes_to};
use crate::fs::*;
use crate::{dos, machine::Amiga, Refusal};
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

const TICKS_PER_SEC: u64 = 50;
const SECS_PER_DAY: u64 = 86_400;
/// 2026-01-01 as days since 1978-01-01 (the Amiga epoch): 48 years, 12 leap days.
pub const EPOCH_DAYS_2026: i32 = 48 * 365 + 12;
const BLOCK_SIZE: u32 = 512;
const ID_DOS_DISK: u32 = 0x444F_5300; // 'DOS\0'
/// A volume the guest sees as having this much room, in blocks (RAM: is bounded only by the guest's own memory).
const TOTAL_BLOCKS: u32 = 1 << 20;
const ID_VALIDATED: u32 = 82;
const ID_WRITE_PROTECTED: u32 = 80;
const MINUS_ONE: u32 = 0xFFFF_FFFF;

/// The virtual "now" as a DateStamp: the host's chosen start day plus the engine's virtual clock.
pub fn now<U: GuestUi>(a: &Amiga<U>) -> [i32; 3] {
    let secs = a.clock_us / 1_000_000;
    let day = a.fs.start_day + (secs / SECS_PER_DAY) as i32;
    let in_day = secs % SECS_PER_DAY;
    [day, (in_day / 60) as i32, ((in_day % 60) * TICKS_PER_SEC + (a.clock_us % 1_000_000) * TICKS_PER_SEC / 1_000_000) as i32]
}

/// DateStamp(d1 = ds) -> d1.
pub fn date_stamp<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let ds = cpu.dreg(1);
    let n = now(a);
    for (i, v) in n.iter().enumerate() { a.mem.write_u32(ds + 4 * i as u32, *v as u32).map_err(Refusal::mem("DateStamp"))?; }
    cpu.set_d0(ds);
    Ok(format!("[dos] DateStamp -> day {} min {} tick {}", n[0], n[1], n[2]))
}

/// CompareDates(d1 = date1, d2 = date2) -> <0 if date1 is later than date2, 0 if equal, >0 if date2 is later (the autodoc's sign).
pub fn compare_dates<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let rd = |a: &Amiga<U>, p: u32| -> Result<[i32; 3], Refusal> {
        let m = Refusal::mem("DateStamp");
        Ok([a.mem.read_u32(p).map_err(&m)? as i32, a.mem.read_u32(p + 4).map_err(&m)? as i32, a.mem.read_u32(p + 8).map_err(&m)? as i32])
    };
    let (x, y) = (rd(a, cpu.dreg(1))?, rd(a, cpu.dreg(2))?);
    let r: i32 = match y.cmp(&x) { core::cmp::Ordering::Greater => 1, core::cmp::Ordering::Less => -1, _ => 0 };
    cpu.set_d0(r as u32);
    Ok(format!("[dos] CompareDates -> {r}"))
}

/// SetFileDate(d1 = name, d2 = date) -> DOSTRUE.
pub fn set_file_date<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let name = str_arg(a, cpu.dreg(1), "SetFileDate.name")?;
    let p = cpu.dreg(2);
    let m = Refusal::mem("DateStamp");
    let d = [a.mem.read_u32(p).map_err(&m)? as i32, a.mem.read_u32(p + 4).map_err(&m)? as i32, a.mem.read_u32(p + 8).map_err(&m)? as i32];
    let what = format!("[dos] SetFileDate \"{name}\"");
    match a.fs.vol.resolve(cur_dir(a), &name) {
        Err(e) => fail(a, cpu, e, 0, what),
        Ok(n) if !a.fs.vol.is_writable(n) => fail(a, cpu, ERROR_DISK_WRITE_PROTECTED, 0, what),
        Ok(n) => { a.fs.vol.get_mut(n).date = d; ok(a, cpu, DOSTRUE, what) }
    }
}

/// Info(d1 = lock, d2 = InfoData) -> DOSTRUE. Sizes come from the volume's actual file bytes.
pub fn info<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (b, id) = (cpu.dreg(1), cpu.dreg(2));
    let what = format!("[dos] Info {:#x}", b << 2);
    let node = match lock_node(a, b) { Ok(n) => n, Err(e) => return fail(a, cpu, e, 0, what) };
    let dev = a.fs.vol.device_of(node);
    let used: usize = used_bytes(a, dev);
    let used_blocks = (used as u32).div_ceil(BLOCK_SIZE).min(TOTAL_BLOCKS);
    let state = if a.fs.vol.is_writable(dev) { ID_VALIDATED } else { ID_WRITE_PROTECTED };
    let m = Refusal::mem("InfoData");
    a.mem.zero(id, 36).map_err(&m)?;
    // id_VolumeNode points at the device's DLT_VOLUME DosList node; id_InUse says whether anything on it is open.
    crate::doslist::sync(a)?;
    let volume_node = crate::doslist::volume_node(a, &String::from(a.fs.vol.name(dev))).unwrap_or(0);
    let in_use = u32::from(a.fs.files.values().any(|f| a.fs.vol.device_of(f.node) == dev));
    for (off, v) in [(8, state), (12, TOTAL_BLOCKS), (16, used_blocks), (20, BLOCK_SIZE), (24, ID_DOS_DISK), (28, volume_node >> 2), (32, in_use)] { a.mem.write_u32(id + off, v).map_err(&m)?; }
    ok(a, cpu, DOSTRUE, what)
}

fn used_bytes<U: GuestUi>(a: &Amiga<U>, dir: crate::volume::NodeId) -> usize {
    a.fs.vol.children(dir).iter().map(|&c| a.fs.vol.file_data(c).map_or_else(|| used_bytes(a, c), |d| d.len())).sum()
}

fn read_n<U: GuestUi>(a: &mut Amiga<U>, b: u32, n: u32) -> Result<Vec<u8>, Refusal> {
    if b == dos::STDIN_FH >> 2 { return Ok(Vec::new()); }
    let f = handle(a, b, "read")?;
    let (node, pos) = (f.node, f.pos);
    let data = a.fs.vol.file_data(node).unwrap_or(&[]);
    let s = (pos as usize).min(data.len());
    let chunk = data[s..(s + n as usize).min(data.len())].to_vec();
    handle(a, b, "read")?.pos = (s + chunk.len()) as u32;
    Ok(chunk)
}

fn put<U: GuestUi>(a: &mut Amiga<U>, b: u32, bytes: &[u8]) -> Result<bool, Refusal> {
    if b == dos::STDOUT_FH >> 2 { dos::emit(a, bytes); return Ok(true); }
    write_bytes_to(a, b, bytes)
}

/// FGetC(d1 = file) -> the next byte, or -1 at EOF.
pub fn fgetc<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let b = cpu.dreg(1);
    let c = read_n(a, b, 1)?;
    cpu.set_d0(c.first().map_or(MINUS_ONE, |&x| x as u32));
    Ok(format!("[dos] FGetC {:#x} -> {:?}", b << 2, c.first()))
}

/// FPutC(d1 = file, d2 = char) -> the char, or -1 (EOF) on error.
pub fn fputc<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (b, c) = (cpu.dreg(1), cpu.dreg(2));
    let done = put(a, b, &[c as u8])?;
    cpu.set_d0(if done { c & 0xFF } else { MINUS_ONE });
    Ok(format!("[dos] FPutC {:#x} {:#x}", b << 2, c & 0xFF))
}

/// UnGetC(d1 = file, d2 = char) -> the char: steps the position back one byte (the last one read).
pub fn ungetc<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (b, c) = (cpu.dreg(1), cpu.dreg(2));
    if b != dos::STDIN_FH >> 2 { let f = handle(a, b, "UnGetC")?; f.pos = f.pos.saturating_sub(1); }
    cpu.set_d0(c);
    Ok(format!("[dos] UnGetC {:#x}", b << 2))
}

/// FRead(d1 = file, d2 = buffer, d3 = blockLength, d4 = blocks) -> whole blocks read.
pub fn fread<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (b, buf, bl, nb) = (cpu.dreg(1), cpu.dreg(2), cpu.dreg(3), cpu.dreg(4));
    let want = bl.checked_mul(nb).ok_or_else(|| Refusal("FRead: blockLength * blocks overflows".into()))?;
    let got = read_n(a, b, want)?;
    a.mem.write_bytes(buf, &got).map_err(Refusal::mem("FRead.buffer"))?;
    let blocks = if bl == 0 { 0 } else { got.len() as u32 / bl };
    cpu.set_d0(blocks);
    Ok(format!("[dos] FRead {:#x} {blocks} blocks", b << 2))
}

/// FWrite(d1 = file, d2 = buffer, d3 = blockLength, d4 = blocks) -> blocks written.
pub fn fwrite<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (b, buf, bl, nb) = (cpu.dreg(1), cpu.dreg(2), cpu.dreg(3), cpu.dreg(4));
    let n = bl.checked_mul(nb).ok_or_else(|| Refusal("FWrite: blockLength * blocks overflows".into()))?;
    let bytes: Vec<u8> = (0..n).map(|i| a.mem.read_u8(buf + i).map_err(Refusal::mem("FWrite.buffer"))).collect::<Result<_, _>>()?;
    let done = put(a, b, &bytes)?;
    cpu.set_d0(if done { nb } else { 0 });
    Ok(format!("[dos] FWrite {:#x} {} bytes", b << 2, n))
}

/// FGets(d1 = file, d2 = buffer, d3 = length) -> the buffer, or NULL at EOF: a line including its newline.
pub fn fgets<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (b, buf, len) = (cpu.dreg(1), cpu.dreg(2), cpu.dreg(3));
    let mut line = Vec::new();
    while (line.len() as u32) + 1 < len {
        let c = read_n(a, b, 1)?;
        let Some(&c) = c.first() else { break };
        line.push(c);
        if c == b'\n' { break; }
    }
    if line.is_empty() { cpu.set_d0(0); return Ok(format!("[dos] FGets {:#x} -> EOF", b << 2)); }
    a.mem.write_bytes(buf, &line).map_err(Refusal::mem("FGets.buffer"))?;
    a.mem.write_u8(buf + line.len() as u32, 0).map_err(Refusal::mem("FGets.buffer"))?;
    cpu.set_d0(buf);
    Ok(format!("[dos] FGets {:#x} -> {} bytes", b << 2, line.len()))
}
