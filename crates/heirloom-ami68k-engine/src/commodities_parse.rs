//! commodities ParseIX and InvertKeyMap: the input-expression description parser (AROS workbench/libs/commodities/
//! parseix.c, documented in cxparse.doc: `[class] {[-] (qualifier|synonym)} [[-] upstroke] [HighMap|ANSICode]`) with the word
//! tables of cx_tables.rs. Words match by case-insensitive PREFIX in table order, exactly as pMatch does (so "alt" is the
//! synonym ALT only because LALT/RALT/LEFT_ALT come first and need the L/R prefix). A single ANSI character goes through
//! the US keymap (key_for_char): the unshifted and shifted legends of the ANSI keys, the raw key codes of the Amiga
//! keyboard. Only the US keymap exists, so InvertKeyMap ignores its keymap argument (NULL = the default one anyway).
//! ParseIX returns 0, -1 (tokens after the end) or -2 (NULL description).
use alloc::format;
use alloc::string::String;
use crate::cx_tables::*;
use crate::libtable::LibraryBuilder;
use crate::machine::Amiga;
use crate::Refusal;
use heirloom_ui_abi::GuestUi;
use heirloom_cpu_68k::Cpu;

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.named("ParseIX", parse_ix::<U>).named("InvertKeyMap", invert_key_map::<U>)
}

/// The IX ParseIX builds (ix_Class .. ix_QualSame); ix_Version is the caller's.
#[derive(Debug, PartialEq, Eq)]
pub struct Ix { pub class: u8, pub code: u16, pub code_mask: u16, pub qualifier: u16, pub qual_mask: u16, pub qual_same: u16 }

const IECLASS_RAWKEY: u8 = 1;
const IEQUALIFIER_LSHIFT: u16 = 1;

fn sep(c: u8) -> bool { matches!(c, b' ' | b'\n' | b'\t' | b',' | 0) }

/// pMatch: optional '-' (when `dash` asks for it), then the first table word that is a case-insensitive prefix of `s`.
fn p_match(words: &[(&str, u16)], s: &[u8], dash: &mut bool) -> Option<u16> {
    let mut rest = s;
    if *dash { if rest.first() == Some(&b'-') { rest = &rest[1..]; } else { *dash = false; } }
    words.iter().find(|(n, _)| rest.len() >= n.len() && rest[..n.len()].eq_ignore_ascii_case(n.as_bytes())).map(|&(_, v)| v)
}

/// The Amiga raw key code (and shift) of an ANSI character on the US keyboard.
pub fn key_for_char(c: u8) -> Option<(u16, u16)> {
    const ROWS: [(&[u8], &[u8], u16); 6] = [
        (b"`1234567890-=\\", b"~!@#$%^&*()_+|", 0x00),
        (b"qwertyuiop[]", b"QWERTYUIOP{}", 0x10),
        (b"asdfghjkl;'", b"ASDFGHJKL:\"", 0x20),
        (b"zxcvbnm,./", b"ZXCVBNM<>?", 0x31),
        (b" ", b" ", 0x40),
        (b"", b"", 0),
    ];
    for (plain, shifted, base) in ROWS {
        if let Some(i) = plain.iter().position(|&p| p == c) { return Some((base + i as u16, 0)); }
        if let Some(i) = shifted.iter().position(|&p| p == c) { return Some((base + i as u16, IEQUALIFIER_LSHIFT)); }
    }
    None
}

pub fn parse(desc: Option<&[u8]>) -> (Ix, i32) {
    let mut ix = Ix { class: IECLASS_RAWKEY, code: 0, code_mask: 0xFFFF, qualifier: 0, qual_mask: IX_NORMALQUALS & !(IEQ_INTERRUPT | IEQ_MULTIBROADCAST), qual_same: 0 };
    let Some(d) = desc else { ix.code = 0xFFFF; return (ix, -2) };
    let mut s = d;
    let skip_seps = |s: &mut &[u8]| while let Some(&c) = s.first() { if sep(c) { *s = &s[1..]; } else { break; } };
    let next = |s: &mut &[u8]| {
        while let Some(&c) = s.first() { if sep(c) { break; } *s = &s[1..]; }
        while let Some(&c) = s.first() { if sep(c) { *s = &s[1..]; } else { break; } }
    };
    skip_seps(&mut s);
    let mut dash = false;
    if let Some(v) = p_match(CLASS, s, &mut dash) { ix.class = v as u8; next(&mut s); }
    loop {
        let mut dash = true;
        if let Some(v) = p_match(QUALIFIER, s, &mut dash) {
            if dash { ix.qual_mask &= !v; } else { ix.qualifier |= v; }
            next(&mut s);
            continue;
        }
        let mut dash = true;
        let Some(v) = p_match(SYNONYM, s, &mut dash) else { break };
        ix.qual_same |= v;
        let mask = match v { IXSYM_SHIFT => IXSYM_SHIFTMASK, IXSYM_ALT => IXSYM_ALTMASK, IXSYM_CAPS => IXSYM_CAPSMASK, _ => 0 };
        if dash { ix.qual_mask &= !mask; } else { ix.qualifier |= mask; }
        next(&mut s);
    }
    let mut up_dash = true;
    let upstroke = p_match(UPSTROKE, s, &mut up_dash).is_some();
    if upstroke { next(&mut s); }
    let mut hd = false;
    if let Some(v) = p_match(HIGHMAP, s, &mut hd) { ix.code = v; }
    else if let Some(&c) = s.first() { if c != 0 { if let Some((code, _)) = key_for_char(c) { ix.code = code; } } }
    if upstroke { if up_dash { ix.code_mask &= !IECODE_UP_PREFIX; } else { ix.code |= IECODE_UP_PREFIX; } }
    while let Some(&c) = s.first() { if sep(c) { break; } s = &s[1..]; }
    (ix, if s.is_empty() { 0 } else { -1 })
}

/// ParseIX(a0 = description, a1 = IX) -> 0, -1 or -2.
fn parse_ix<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (dp, ixp) = (cpu.areg(0), cpu.areg(1));
    let d = if dp == 0 { None } else { Some(crate::guest_str::read_bytes_z(&a.mem, dp, 4096).map_err(Refusal::mem("ParseIX.description"))?) };
    let (ix, rc) = parse(d.as_deref());
    let m = Refusal::mem("IX");
    a.mem.write_u8(ixp + 1, ix.class).map_err(&m)?;
    for (off, v) in [(2u32, ix.code), (4, ix.code_mask), (6, ix.qualifier), (8, ix.qual_mask), (10, ix.qual_same)] { a.mem.write_u16(ixp + off, v).map_err(&m)?; }
    cpu.set_d0(rc as u32);
    Ok(format!("[commodities] ParseIX \"{}\" -> {rc} (class {} code {:#x} qual {:#x} mask {:#x})", d.map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default(), ix.class, ix.code, ix.qualifier, ix.qual_mask))
}

/// InvertKeyMap(d0 = ANSI code, a0 = InputEvent, a1 = KeyMap or NULL) -> TRUE if a key makes the character; sets ie_Code and
/// ie_Qualifier (shift).
fn invert_key_map<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (c, ie) = (cpu.dreg(0), cpu.areg(0));
    let r = if c < 256 { key_for_char(c as u8) } else { None };
    if let Some((code, q)) = r {
        a.mem.write_u16(ie + 6, code).map_err(Refusal::mem("ie_Code"))?;
        a.mem.write_u16(ie + 8, q).map_err(Refusal::mem("ie_Qualifier"))?;
    }
    cpu.set_d0(r.is_some() as u32);
    Ok(format!("[commodities] InvertKeyMap {c:#x} -> {r:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn descriptions() {
        let (ix, rc) = parse(Some(b"ctrl alt f1"));
        assert_eq!(rc, 0);
        assert_eq!((ix.class, ix.code, ix.qualifier), (1, 0x50, 0x08 | 0x30));
        // negative control: trailing junk after the key is -1; a NULL description is -2
        assert_eq!(parse(Some(b"f1 f2")).1, -1);
        assert_eq!(parse(None).1, -2);
        let (ix, _) = parse(Some(b"rawkey -lshift upstroke a"));
        assert_eq!((ix.code & 0x7F, ix.code & 0x80, ix.qual_mask & 1), (0x20, 0x80, 0));
    }
}
