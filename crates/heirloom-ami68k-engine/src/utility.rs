//! utility.library: the TagItem helpers (FindTagItem, GetTagData, NextTagItem, PackBoolTags), CallHookPkt, and the
//! integer/string odds and ends C code reaches for (SMult32, UMult32, Stricmp, Strnicmp, ToUpper, ToLower).
//! SDivMod32/UDivMod32 (quotient D0, remainder D1) are below; they used to stay
//! named refusals. Item addresses matter here (FindTagItem returns a pointer), so the walk keeps them; the control tags
//! (TAG_IGNORE/MORE/SKIP/DONE) behave as in tags.rs. Facts: utility/tagitem.h, utility/hooks.h, AROS rom/utility.
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec;
use alloc::format;
use crate::guest_call::{self, Next};
use crate::libtable::LibraryBuilder;
use crate::tags::{TAG_DONE, TAG_IGNORE, TAG_MORE, TAG_SKIP};
use crate::{machine::Amiga, Refusal};
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

/// The next real item at or after `*state`: (address, tag, data); `*state` moves past it. None at TAG_DONE / NULL.
pub(crate) fn next_item<U: GuestUi>(a: &Amiga<U>, state: &mut u32) -> Result<Option<(u32, u32, u32)>, Refusal> {
    let mut seen = alloc::collections::BTreeSet::new();
    loop {
        if *state == 0 { return Ok(None); }
        let at = *state;
        let (tag, data) = (a.mem.read_u32(at).map_err(Refusal::mem("ti_Tag"))?, a.mem.read_u32(at + 4).map_err(Refusal::mem("ti_Data"))?);
        match tag {
            TAG_DONE => { *state = 0; return Ok(None); }
            TAG_IGNORE => *state = at + 8,
            TAG_MORE => *state = data,
            TAG_SKIP => *state = at + 8 * (data + 1),
            _ => { *state = at + 8; return Ok(Some((at, tag, data))); }
        }
        if !seen.insert(*state) { return Err(Refusal(format!("cyclic TagItem list: item {:#x} comes back (TAG_MORE / TAG_SKIP loop)", *state))); }
    }
}

pub(crate) fn find<U: GuestUi>(a: &Amiga<U>, tag: u32, list: u32) -> Result<Option<(u32, u32)>, Refusal> {
    let mut st = list;
    while let Some((at, t, d)) = next_item(a, &mut st)? { if t == tag { return Ok(Some((at, d))); } }
    Ok(None)
}

/// FindTagItem(d0 = tag, a0 = list) -> the item or NULL.
fn find_tag_item<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (tag, list) = (cpu.dreg(0), cpu.areg(0));
    let r = find(a, tag, list)?.map(|x| x.0).unwrap_or(0);
    cpu.set_d0(r);
    Ok(format!("[utility] FindTagItem {tag:#x} -> {r:#x}"))
}

/// GetTagData(d0 = tag, d1 = default, a0 = list).
fn get_tag_data<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (tag, def, list) = (cpu.dreg(0), cpu.dreg(1), cpu.areg(0));
    let r = find(a, tag, list)?.map(|x| x.1).unwrap_or(def);
    cpu.set_d0(r);
    Ok(format!("[utility] GetTagData {tag:#x} -> {r:#x}"))
}

/// NextTagItem(a0 = &state) -> the next item or NULL.
fn next_tag_item<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let pp = cpu.areg(0);
    let mut st = a.mem.read_u32(pp).map_err(Refusal::mem("NextTagItem state"))?;
    let r = next_item(a, &mut st)?.map(|x| x.0).unwrap_or(0);
    a.mem.write_u32(pp, st).map_err(Refusal::mem("NextTagItem state"))?;
    cpu.set_d0(r);
    Ok(format!("[utility] NextTagItem -> {r:#x}"))
}

/// PackBoolTags(d0 = initial flags, a0 = tags, a1 = boolmap {tag, flag}* ended by TAG_DONE) -> flags.
fn pack_bool_tags<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (mut flags, list, map) = (cpu.dreg(0), cpu.areg(0), cpu.areg(1));
    let mut st = list;
    while let Some((_, t, d)) = next_item(a, &mut st)? {
        let mut m = map;
        loop { // forward only: ends at TAG_DONE or at the end of guest memory (a named refusal)
            let (mt, mf) = (a.mem.read_u32(m).map_err(Refusal::mem("boolmap"))?, a.mem.read_u32(m + 4).map_err(Refusal::mem("boolmap"))?);
            if mt == TAG_DONE { break; }
            if mt == t { if d != 0 { flags |= mf } else { flags &= !mf } break; }
            m += 8;
        }
    }
    cpu.set_d0(flags);
    Ok(format!("[utility] PackBoolTags -> {flags:#x}"))
}

/// CallHookPkt(a0 = hook, a2 = object, a1 = message) -> the hook's D0 (h_Entry called with A0/A1/A2 as given).
fn call_hook_pkt<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (hook, obj, msg) = (cpu.areg(0), cpu.areg(2), cpu.areg(1));
    let entry = a.mem.read_u32(hook + 8).map_err(Refusal::mem("Hook.h_Entry"))?;
    if entry == 0 { return Err(Refusal(format!("CallHookPkt: hook {hook:#x} has a NULL h_Entry"))); }
    guest_call::start(a, cpu, Next::Call { target: entry, d0: 0, aregs: vec![(0, hook), (1, msg), (2, obj)], then: Box::new(|_, _, d0| Ok(Next::Return(d0))) })?;
    Ok(format!("[utility] CallHookPkt hook={hook:#x} entry={entry:#x}"))
}

fn s_mult32<U: GuestUi>(_: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let r = (cpu.dreg(0) as i32).wrapping_mul(cpu.dreg(1) as i32) as u32;
    cpu.set_d0(r);
    Ok(format!("[utility] SMult32 -> {r:#x}"))
}
fn u_mult32<U: GuestUi>(_: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let r = cpu.dreg(0).wrapping_mul(cpu.dreg(1));
    cpu.set_d0(r);
    Ok(format!("[utility] UMult32 -> {r:#x}"))
}

/// SDivMod32 / UDivMod32(d0 = dividend, d1 = divisor) -> d0 = quotient, d1 = remainder (utility_lib.fd: "for m68k assembly programmers"; AROS's C body returns
/// only the quotient, the m68k-native assembly returns both, and every classic binary reads D1). Signed: the remainder takes the dividend's sign (DIVS).
/// A zero divisor is a divide-by-zero exception on the 68000: refused by name, not answered.
fn div_mod32<U: GuestUi>(cpu: &mut Cpu, signed: bool) -> Result<String, Refusal> {
    let (n, d) = (cpu.dreg(0), cpu.dreg(1));
    let name = if signed { "SDivMod32" } else { "UDivMod32" };
    if d == 0 { return Err(Refusal(format!("{name}({n:#x}, 0): division by zero (a 68000 exception)"))); }
    let (q, r) = if signed { ((n as i32).wrapping_div(d as i32) as u32, (n as i32).wrapping_rem(d as i32) as u32) } else { (n / d, n % d) };
    cpu.set_d0(q);
    cpu.set_dreg(1, r);
    Ok(format!("[utility] {name} {n:#x} / {d:#x} -> {q:#x} rem {r:#x}"))
}

fn cmp(a: &[u8], b: &[u8]) -> i32 {
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0).to_ascii_uppercase(), b.get(i).copied().unwrap_or(0).to_ascii_uppercase());
        if x != y { return x as i32 - y as i32; }
    }
    0
}
/// Stricmp(a0 = s1, a1 = s2) -> <0, 0, >0 (ISO-8859 case folding is ASCII here).
fn stricmp<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (s1, s2) = (a.mem.read_cstr(cpu.areg(0)).map_err(Refusal::mem("Stricmp"))?, a.mem.read_cstr(cpu.areg(1)).map_err(Refusal::mem("Stricmp"))?);
    let r = cmp(s1.as_bytes(), s2.as_bytes());
    cpu.set_d0(r as u32);
    Ok(format!("[utility] Stricmp \"{s1}\" \"{s2}\" -> {r}"))
}
/// Strnicmp(a0 = s1, a1 = s2, d0 = n).
fn strnicmp<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (s1, s2, n) = (a.mem.read_cstr(cpu.areg(0)).map_err(Refusal::mem("Strnicmp"))?, a.mem.read_cstr(cpu.areg(1)).map_err(Refusal::mem("Strnicmp"))?, cpu.dreg(0) as usize);
    let r = cmp(&s1.as_bytes()[..s1.len().min(n)], &s2.as_bytes()[..s2.len().min(n)]);
    cpu.set_d0(r as u32);
    Ok(format!("[utility] Strnicmp \"{s1}\" \"{s2}\" {n} -> {r}"))
}
fn to_upper<U: GuestUi>(_: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let c = cpu.dreg(0) & 0xFF;
    cpu.set_d0((c as u8).to_ascii_uppercase() as u32);
    Ok("[utility] ToUpper".into())
}
fn to_lower<U: GuestUi>(_: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let c = cpu.dreg(0) & 0xFF;
    cpu.set_d0((c as u8).to_ascii_lowercase() as u32);
    Ok("[utility] ToLower".into())
}

/// PackStructureTags(a0 = pack, a1 = packTable, a2 = tagList) -> d0 = count of packed fields.
/// Facts: AmigaOS 3.0 / AROS `utility/pack.h` (PACK_STARTTABLE, PACK_NEWOFFSET, PACK_ENDTABLE, PKCTRL_*).
fn pack_structure_tags<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (pack, table, tags) = (cpu.areg(0), cpu.areg(1), cpu.areg(2));
    let count = pack_or_unpack(a, pack, table, tags, true)?;
    cpu.set_d0(count);
    Ok(format!("[utility] PackStructureTags pack={pack:#x} table={table:#x} -> {count}"))
}

/// UnpackStructureTags(a0 = pack, a1 = packTable, a2 = tagList) -> d0 = count of unpacked fields.
fn unpack_structure_tags<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (pack, table, tags) = (cpu.areg(0), cpu.areg(1), cpu.areg(2));
    let count = pack_or_unpack(a, pack, table, tags, false)?;
    cpu.set_d0(count);
    Ok(format!("[utility] UnpackStructureTags pack={pack:#x} table={table:#x} -> {count}"))
}

fn pack_or_unpack<U: GuestUi>(a: &mut Amiga<U>, pack: u32, table: u32, tags: u32, is_pack: bool) -> Result<u32, Refusal> {
    if pack == 0 || table == 0 || tags == 0 {
        return Ok(0);
    }
    let m = Refusal::mem("PackStructureTags");
    let mut ptr = table;
    let mut tag_base = a.mem.read_u32(ptr).map_err(&m)?;
    ptr += 4;
    let mut count = 0u32;
    for _ in 0..4096 {
        let entry = a.mem.read_u32(ptr).map_err(&m)?;
        ptr += 4;
        if entry == 0 {
            break;
        }
        if entry == 0xFFFF_FFFF {
            tag_base = a.mem.read_u32(ptr).map_err(&m)?;
            ptr += 4;
            continue;
        }
        let skip = if is_pack { entry & (1 << 29) != 0 } else { entry & (1 << 30) != 0 };
        if skip {
            continue;
        }
        let tag = tag_base.wrapping_add((entry >> 16) & 0x3FF);
        let Some((_, data)) = find(a, tag, tags)? else {
            continue;
        };
        let off = entry & 0x1FFF;
        let bit = (entry >> 13) & 7;
        let exists = entry & (1 << 26) != 0;
        let signed_or_flip = entry & (1 << 31) != 0;
        let addr = pack.wrapping_add(off);
        match (entry >> 27) & 3 {
            0 => {
                // BYTE / UBYTE
                if is_pack {
                    a.mem.write_u8(addr, data as u8).map_err(&m)?;
                } else if data != 0 {
                    let b = a.mem.read_u8(addr).map_err(&m)?;
                    let v = if signed_or_flip { (b as i8) as i32 as u32 } else { u32::from(b) };
                    a.mem.write_u32(data, v).map_err(&m)?;
                }
            }
            1 => {
                // WORD / UWORD
                if is_pack {
                    a.mem.write_u16(addr, data as u16).map_err(&m)?;
                } else if data != 0 {
                    let w = a.mem.read_u16(addr).map_err(&m)?;
                    let v = if signed_or_flip { (w as i16) as i32 as u32 } else { u32::from(w) };
                    a.mem.write_u32(data, v).map_err(&m)?;
                }
            }
            2 => {
                // LONG / ULONG
                if is_pack {
                    a.mem.write_u32(addr, data).map_err(&m)?;
                } else if data != 0 {
                    let v = a.mem.read_u32(addr).map_err(&m)?;
                    a.mem.write_u32(data, v).map_err(&m)?;
                }
            }
            _ => {
                // BIT / FLIPBIT
                let mask = 1u8 << bit;
                if is_pack {
                    let raw = if exists { true } else { data != 0 };
                    let set = raw ^ signed_or_flip;
                    let cur = a.mem.read_u8(addr).map_err(&m)?;
                    let next = if set { cur | mask } else { cur & !mask };
                    a.mem.write_u8(addr, next).map_err(&m)?;
                } else if data != 0 {
                    let cur = a.mem.read_u8(addr).map_err(&m)?;
                    let set = ((cur & mask) != 0) ^ signed_or_flip;
                    a.mem.write_u32(data, u32::from(set)).map_err(&m)?;
                }
            }
        }
        count += 1;
    }
    Ok(count)
}

/// GetUniqueID() -> d0: returns a unique non-zero 32-bit identifier each call (AmigaOS 3.0 utility.library).
fn get_unique_id<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    a.unique_id = a.unique_id.wrapping_add(1);
    if a.unique_id == 0 {
        a.unique_id = 1;
    }
    cpu.set_d0(a.unique_id);
    Ok(format!("[utility] GetUniqueID -> {}", a.unique_id))
}

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.named("FindTagItem", find_tag_item::<U>).named("GetTagData", get_tag_data::<U>).named("NextTagItem", next_tag_item::<U>)
        .named("PackBoolTags", pack_bool_tags::<U>).named("CallHookPkt", call_hook_pkt::<U>)
        .named("PackStructureTags", pack_structure_tags::<U>).named("UnpackStructureTags", unpack_structure_tags::<U>)
        .named("GetUniqueID", get_unique_id::<U>)
        .named("SMult32", s_mult32::<U>).named("UMult32", u_mult32::<U>).named("SDivMod32", |_, c| div_mod32::<U>(c, true)).named("UDivMod32", |_, c| div_mod32::<U>(c, false))
        .named("Stricmp", stricmp::<U>).named("Strnicmp", strnicmp::<U>).named("ToUpper", to_upper::<U>).named("ToLower", to_lower::<U>)
}

#[cfg(test)]
mod tests {
    use super::*;
    use heirloom_ui_abi::RecordingGuestUi;

    #[test]
    fn pack_and_unpack_structure_tags_handle_sizes_signed_bits_and_newoffset() {
        let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
        let pack = 0x1F000u32;
        let table = 0x1F100u32;
        let tags = 0x1F200u32;
        let out_vals = 0x1F300u32;
        a.mem.zero(pack, 32).unwrap();
        let base1 = 0x8000_1000u32;
        let base2 = 0x8000_2000u32;
        // Table:
        //   tag base1+0 -> BYTE at off 0 (PKCTRL_BYTE = 0x8000_0000)
        //   tag base1+1 -> WORD at off 2 (PKCTRL_UWORD = 0x0800_0000)
        //   tag base1+2 -> LONG at off 4 (PKCTRL_ULONG = 0x1000_0000)
        //   tag base1+3 -> BIT 2 at off 8 (PKCTRL_BIT = 0x1800_0000)
        //   PACK_NEWOFFSET(base2)
        //   tag base2+1 -> FLIPBIT 5 at off 8 (PKCTRL_FLIPBIT = 0x9800_0000)
        //   PACK_ENDTABLE (0)
        let entries = [
            base1,
            0x8000_0000 | (0 << 16) | 0,
            0x0800_0000 | (1 << 16) | 2,
            0x1000_0000 | (2 << 16) | 4,
            0x1800_0000 | (3 << 16) | (2 << 13) | 8,
            0xFFFF_FFFF,
            base2,
            0x9800_0000 | (1 << 16) | (5 << 13) | 8,
            0,
        ];
        for (i, &w) in entries.iter().enumerate() {
            a.mem.write_u32(table + i as u32 * 4, w).unwrap();
        }
        let pack_tags = [
            (base1 + 0, (-5i8) as u8 as u32),
            (base1 + 1, 0xBEEF),
            (base1 + 2, 0x1234_5678),
            (base1 + 3, 1),
            (base2 + 1, 0), // FLIPBIT with false -> sets bit 5!
            (TAG_DONE, 0),
        ];
        for (i, &(t, d)) in pack_tags.iter().enumerate() {
            a.mem.write_u32(tags + i as u32 * 8, t).unwrap();
            a.mem.write_u32(tags + i as u32 * 8 + 4, d).unwrap();
        }
        assert_eq!(pack_or_unpack(&mut a, pack, table, tags, true).unwrap(), 5);
        assert_eq!(a.mem.read_u8(pack).unwrap() as i8, -5);
        assert_eq!(a.mem.read_u16(pack + 2).unwrap(), 0xBEEF);
        assert_eq!(a.mem.read_u32(pack + 4).unwrap(), 0x1234_5678);
        assert_eq!(a.mem.read_u8(pack + 8).unwrap(), (1 << 2) | (1 << 5));

        // Unpack back into out_vals[0..5]
        let unpack_tags = [
            (base1 + 0, out_vals),
            (base1 + 1, out_vals + 4),
            (base1 + 2, out_vals + 8),
            (base1 + 3, out_vals + 12),
            (base2 + 1, out_vals + 16),
            (TAG_DONE, 0),
        ];
        for (i, &(t, d)) in unpack_tags.iter().enumerate() {
            a.mem.write_u32(tags + i as u32 * 8, t).unwrap();
            a.mem.write_u32(tags + i as u32 * 8 + 4, d).unwrap();
        }
        assert_eq!(pack_or_unpack(&mut a, pack, table, tags, false).unwrap(), 5);
        assert_eq!(a.mem.read_u32(out_vals).unwrap() as i32, -5, "PKCTRL_BYTE sign-extends on unpack");
        assert_eq!(a.mem.read_u32(out_vals + 4).unwrap(), 0xBEEF);
        assert_eq!(a.mem.read_u32(out_vals + 8).unwrap(), 0x1234_5678);
        assert_eq!(a.mem.read_u32(out_vals + 12).unwrap(), 1);
        assert_eq!(a.mem.read_u32(out_vals + 16).unwrap(), 0, "PKCTRL_FLIPBIT un-flips on unpack");
    }

    #[test]
    fn get_unique_id_returns_strictly_monotonic_nonzero_ids() {
        let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
        // Verify wrap-around skips 0 as required by utility.library/GetUniqueID.
        a.unique_id = u32::MAX - 1;
        let mut cpu = crate::cpu::Cpu::detached();
        let _ = get_unique_id(&mut a, &mut cpu).unwrap();
        assert_eq!(a.unique_id, u32::MAX);
        let _ = get_unique_id(&mut a, &mut cpu).unwrap();
        assert_eq!(a.unique_id, 1, "GetUniqueID must never return 0 on wrap");
        let _ = get_unique_id(&mut a, &mut cpu).unwrap();
        assert_eq!(a.unique_id, 2);
    }
}
