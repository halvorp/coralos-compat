//! Small string services: StrToLong and SplitName.
use alloc::format;
use alloc::string::String;
use crate::libtable::LibraryBuilder;
use crate::{machine::Amiga, Refusal};
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.named("StrToLong", str_to_long::<U>).named("SplitName", split_name::<U>)
}

/// StrToLong(d1 = string, d2 = LONG *value) -> characters used, or -1 (and *value = 0) if there is no number.
/// Leading blanks and one sign are accepted; the number is decimal.
fn str_to_long<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (sp, vp) = (cpu.dreg(1), cpu.dreg(2));
    let s = crate::guest_str::read_bytes_z(&a.mem, sp, 0x1_0000).map_err(Refusal::mem("StrToLong.string"))?;
    let blanks = s.iter().take_while(|&&c| c == b' ' || c == b'\t').count();
    let neg = s.get(blanks) == Some(&b'-');
    let from = blanks + usize::from(neg || s.get(blanks) == Some(&b'+'));
    let digits = s[from..].iter().take_while(|c| c.is_ascii_digit()).count();
    let (value, used) = if digits == 0 { (0i32, -1i32) } else {
        let v = s[from..from + digits].iter().fold(0i64, |v, &c| (v * 10 + i64::from(c - b'0')).min(1 << 40));
        ((if neg { -v } else { v }) as i32, (from + digits) as i32)
    };
    a.mem.write_u32(vp, value as u32).map_err(Refusal::mem("StrToLong.value"))?;
    cpu.set_d0(used as u32);
    Ok(format!("[dos] StrToLong \"{}\" -> {value} ({used} chars)", String::from_utf8_lossy(&s)))
}

/// SplitName(d1 = name, d2 = separator, d3 = buffer, d4 = oldpos, d5 = size) -> the position after the separator the
/// piece ended at, or -1 when the piece ended at the end of the string (or did not fit: it is then cut to size-1).
fn split_name<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (np, sep, buf, old, size) = (cpu.dreg(1), cpu.dreg(2) as u8, cpu.dreg(3), cpu.dreg(4) as usize, cpu.dreg(5) as usize);
    let s = crate::guest_str::read_bytes_z(&a.mem, np, 0x1_0000).map_err(Refusal::mem("SplitName.name"))?;
    let rest = s.get(old..).unwrap_or(&[]);
    let end = rest.iter().position(|&c| c == sep);
    let piece = &rest[..end.unwrap_or(rest.len())];
    let cut = piece.len().min(size.saturating_sub(1));
    if size > 0 { crate::guest_str::write_z(&mut a.mem, buf, &piece[..cut]).map_err(Refusal::mem("SplitName.buffer"))?; }
    let next = match end { Some(e) if cut == piece.len() => (old + e + 1) as i32, _ => -1 };
    cpu.set_d0(next as u32);
    Ok(format!("[dos] SplitName \"{}\" from {old} -> {next}", String::from_utf8_lossy(&s)))
}
