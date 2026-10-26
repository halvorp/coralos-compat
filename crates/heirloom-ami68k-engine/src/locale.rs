//! locale.library with the built-in English locale: OpenLocale(NULL) / CloseLocale over ONE real `struct Locale`, the
//! english.language strings (GetLocaleStr), character classes and conversion, StrnCmp / StrConvert, and the catalog
//! calls with no catalogs on the machine (OpenCatalogA answers NULL, GetCatalogStr the caller's default string).
//! FormatDate / FormatString / ParseDate are in locale_fmt.rs.
//!
//! Defaults and layout come from AROS workbench/libs/locale (defaultlocale.c: the values its comment says were verified
//! on a real Amiga: "united_states.country", "english.language", "english"; the date/time formats, separators and
//! currency of the US locale; english.c: the 51 language strings and the code/collation tables, here generated into
//! locale_tables.rs). `struct Locale` is 168 bytes; offsets by m68k-linux-gnu-gcc -m68020 -mno-align-int over
//! libraries/locale.h (2026-10-18): loc_LocaleName 0, loc_LanguageName 4, loc_PrefLanguages 8 (10 pointers), loc_Flags 48,
//! loc_CodeSet 52, loc_CountryCode 56, loc_TelephoneCode 60, loc_GMTOffset 64, loc_MeasuringSystem 68, loc_CalendarType 69,
//! the six date/time format strings 72..92, loc_DecimalPoint 96, loc_GroupSeparator 100, loc_FracGroupSeparator 104,
//! loc_Grouping 108, loc_FracGrouping 112, the monetary equivalents 116..135, loc_MonFracDigits 136, loc_MonIntFracDigits 137,
//! loc_MonCS 140, loc_MonSmallCS 144, loc_MonIntCS 148, loc_MonPositiveSign 152, SpaceSep/SignPos/CSPos 156..158,
//! loc_MonNegativeSign 160, SpaceSep/SignPos/CSPos 164..166.
//!
//! Only the default locale exists: OpenLocale("file") (a preferences file) is a named refusal, and so is passing any
//! other pointer as a `struct Locale`. A catalog can never be open, so a non-NULL catalog is a named refusal too.
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use crate::libtable::LibraryBuilder;
use crate::locale_tables::*;
use crate::machine::Amiga;
use crate::Refusal;
use heirloom_ui_abi::GuestUi;
use heirloom_cpu_68k::Cpu;

/// english.language strings by id (libraries/localestd.h: DAY_1 = 1, ABDAY_1 = 8, MON_1 = 15, ABMON_1 = 27, YESSTR = 39,
/// NOSTR 40, AM_STR 41, PM_STR 42, SOFTHYPHEN 43 ... FUTURESTR 50).
pub const STRINGS: [&str; 51] = [
    "", "Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday",
    "Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat",
    "January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December",
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    "Yes", "No", "am", "pm", "\u{ad}", "-", "\"", "\"", "Yesterday", "Today", "Tomorrow", "Future",
];
pub const DAY_1: usize = 1;
pub const ABDAY_1: usize = 8;
pub const MON_1: usize = 15;
pub const ABMON_1: usize = 27;
pub const AM_STR: usize = 41;
pub const PM_STR: usize = 42;

const LOCALE_SIZE: u32 = 168;
/// loc_Grouping etc.: "\x03\x00" (groups of three, then the last one repeats).
const GROUPING: &[u8] = &[3, 0];

/// What the machine built for the default locale (lazily, on first use).
#[derive(Default)]
pub struct State {
    /// The `struct Locale` in guest memory; 0 until first use.
    pub locale: u32,
    /// Guest address of each language string, by id.
    pub strings: Vec<u32>,
    /// OpenLocale(NULL) calls not yet matched by CloseLocale.
    pub opens: u32,
}

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.named("OpenLocale", open_locale::<U>).named("CloseLocale", close_locale::<U>).named("GetLocaleStr", get_locale_str::<U>)
        .named("OpenCatalogA", open_catalog::<U>).named("CloseCatalog", close_catalog::<U>).named("GetCatalogStr", get_catalog_str::<U>)
        .named("ConvToLower", conv_to_lower::<U>).named("ConvToUpper", conv_to_upper::<U>)
        .named("IsAlNum", |a, c| is_class(a, c, "IsAlNum", 0x0005)).named("IsAlpha", |a, c| is_class(a, c, "IsAlpha", 0x0001))
        .named("IsCntrl", |a, c| is_class(a, c, "IsCntrl", 0x0002)).named("IsDigit", |a, c| is_class(a, c, "IsDigit", 0x0004))
        .named("IsGraph", |a, c| is_class(a, c, "IsGraph", 0x0008)).named("IsLower", |a, c| is_class(a, c, "IsLower", 0x0010))
        .named("IsPrint", |a, c| is_class(a, c, "IsPrint", 0x0020)).named("IsPunct", |a, c| is_class(a, c, "IsPunct", 0x0040))
        .named("IsSpace", |a, c| is_class(a, c, "IsSpace", 0x0080)).named("IsUpper", |a, c| is_class(a, c, "IsUpper", 0x0100))
        .named("IsXDigit", |a, c| is_class(a, c, "IsXDigit", 0x0200))
        .named("StrnCmp", str_n_cmp::<U>).named("StrConvert", str_convert::<U>)
        .with(crate::locale_fmt::register::<U>)
}

fn put(img: &mut Vec<u8>, s: &[u8]) -> u32 { let at = img.len() as u32; img.extend_from_slice(s); img.push(0); at }

/// Build the default Locale and the string table in the engine's arena (once).
pub fn ensure<U: GuestUi>(a: &mut Amiga<U>) -> Result<u32, Refusal> {
    if a.locale.locale != 0 { return Ok(a.locale.locale); }
    // One block: the struct, then every string; pointers are patched in once the base is known.
    let mut img = alloc::vec![0u8; LOCALE_SIZE as usize];
    let mut ptrs: Vec<(u32, u32)> = Vec::new(); // (field offset, image offset of its string)
    let mut s = |img: &mut Vec<u8>, field: u32, text: &[u8]| { let o = put(img, text); ptrs.push((field, o)); };
    s(&mut img, 0, b"united_states.country");
    s(&mut img, 4, b"english.language");
    s(&mut img, 8, b"english");
    // loc_PrefLanguages[1..9], loc_Flags, loc_CodeSet stay zero.
    for (field, text) in [(72, "%A %B %e %Y %Q:%M %p"), (76, "%A %B %e"), (80, "%Q:%M:%S %p"), (84, "%m/%d/%y %Q:%M %p"), (88, "%m/%d/%y"), (92, "%Q:%M %p"),
        (96, "."), (100, ","), (104, "."), (116, "."), (120, ","), (124, ","), (140, "$"), (144, "\u{a2}"), (148, "USD"), (152, ""), (160, "-")] {
        // the small currency symbol is the single byte 0xA2 (ISOASCII_CENT), not UTF-8
        let bytes: Vec<u8> = text.chars().map(|c| c as u32 as u8).collect();
        s(&mut img, field, &bytes);
    }
    let group_off = img.len() as u32; img.extend_from_slice(GROUPING);
    for f in [108u32, 112, 128, 132] { ptrs.push((f, group_off)); }
    let mut strings = Vec::new();
    for t in STRINGS.iter() { let bytes: Vec<u8> = t.chars().map(|c| c as u32 as u8).collect(); strings.push(put(&mut img, &bytes)); }
    let base = crate::fs::alloc_zeroed_sys(a, img.len() as u32)?.ok_or_else(|| Refusal("locale.library: no memory for the default Locale (NOMEM)".into()))?;
    let m = Refusal::mem("Locale");
    a.mem.write_bytes(base, &img).map_err(&m)?;
    for (field, off) in ptrs { a.mem.write_u32(base + field, base + off).map_err(&m)?; }
    for (field, v) in [(56u32, 0x5553_4100u32), (60, 1)] { a.mem.write_u32(base + field, v).map_err(&m)?; } // MAKE_ID('U','S','A',0), 1
    a.mem.write_u8(base + 68, 1).map_err(&m)?;  // MS_AMERICAN
    a.mem.write_u8(base + 69, 0).map_err(&m)?;  // CT_7SUN
    a.mem.write_u8(base + 136, 2).map_err(&m)?; // loc_MonFracDigits
    a.mem.write_u8(base + 137, 2).map_err(&m)?; // loc_MonIntFracDigits
    // Positive: SS_NOSPACE, SP_PREC_ALL, CSP_PRECEDES. Negative: the same, with sign "-".
    for (off, v) in [(156u32, 0u8), (157, 1), (158, 0), (164, 0), (165, 1), (166, 0)] { a.mem.write_u8(base + off, v).map_err(&m)?; }
    a.locale.locale = base;
    a.locale.strings = strings.into_iter().map(|o| base + o).collect();
    Ok(base)
}

/// `locale` as a function argument: NULL means the default locale (as the Is*/Conv* autodocs say); the default itself is
/// fine; anything else is not a Locale this library made.
pub(crate) fn locale_arg<U: GuestUi>(a: &mut Amiga<U>, p: u32, call: &str) -> Result<u32, Refusal> {
    let d = ensure(a)?;
    if p == 0 || p == d { Ok(d) } else { Err(Refusal(format!("{call}: {p:#x} is not a struct Locale OpenLocale returned"))) }
}

fn open_locale<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let name = cpu.areg(0);
    if name != 0 {
        let n = a.mem.read_cstr(name).map_err(Refusal::mem("OpenLocale.name"))?;
        return Err(Refusal(format!("OpenLocale \"{n}\": locale preference files are not read; only OpenLocale(NULL), the built-in English locale")));
    }
    let l = ensure(a)?;
    a.locale.opens += 1;
    cpu.set_d0(l);
    Ok(format!("[locale] OpenLocale(NULL) -> {l:#x}"))
}

fn close_locale<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let p = cpu.areg(0);
    if p == 0 { return Ok("[locale] CloseLocale NULL".into()); }
    locale_arg(a, p, "CloseLocale")?;
    a.locale.opens = a.locale.opens.checked_sub(1).ok_or_else(|| Refusal("CloseLocale: the Locale is not open (more CloseLocale than OpenLocale)".into()))?;
    Ok(format!("[locale] CloseLocale {p:#x}"))
}

/// GetLocaleStr(a0 = locale, d0 = id) -> the string, or NULL for an id with no string (0..50 have one; AROS's
/// LANG_NAME 51 has no table entry, so NULL).
fn get_locale_str<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    locale_arg(a, cpu.areg(0), "GetLocaleStr")?;
    let id = cpu.dreg(0) as usize;
    let p = a.locale.strings.get(id).copied().unwrap_or(0);
    cpu.set_d0(p);
    Ok(format!("[locale] GetLocaleStr {id} -> {}", if p == 0 { "NULL".into() } else { format!("\"{}\"", STRINGS[id]) }))
}

/// OpenCatalogA(a0 = locale, a1 = name, a2 = tags) -> NULL: no catalogs exist on this machine, which is the documented
/// "use your built-in strings" outcome (the tag list is ignored).
fn open_catalog<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    locale_arg(a, cpu.areg(0), "OpenCatalogA")?;
    let name = if cpu.areg(1) == 0 { String::new() } else { a.mem.read_cstr(cpu.areg(1)).map_err(Refusal::mem("OpenCatalogA.name"))? };
    cpu.set_d0(0);
    Ok(format!("[locale] OpenCatalogA \"{name}\" -> NULL (no catalogs: the built-in strings are used)"))
}

fn close_catalog<U: GuestUi>(_: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    if cpu.areg(0) != 0 { return Err(Refusal(format!("CloseCatalog: {:#x} is not a catalog OpenCatalogA returned (none can be open)", cpu.areg(0)))); }
    Ok("[locale] CloseCatalog NULL".into())
}

/// GetCatalogStr(a0 = catalog, d0 = id, a1 = default) -> the default string.
fn get_catalog_str<U: GuestUi>(_: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    if cpu.areg(0) != 0 { return Err(Refusal(format!("GetCatalogStr: {:#x} is not a catalog OpenCatalogA returned (none can be open)", cpu.areg(0)))); }
    cpu.set_d0(cpu.areg(1));
    Ok(format!("[locale] GetCatalogStr {} -> the default string", cpu.dreg(0)))
}

fn conv<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, name: &str, table: &[u8; 256]) -> Result<String, Refusal> {
    locale_arg(a, cpu.areg(0), name)?;
    let c = cpu.dreg(0);
    // Like english.language: only the low byte is converted, the rest of the word is kept.
    let r = (c & !0xFF) | table[(c & 0xFF) as usize] as u32;
    cpu.set_d0(r);
    Ok(format!("[locale] {name} {c:#x} -> {r:#x}"))
}
fn conv_to_lower<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> { conv(a, cpu, "ConvToLower", &TO_LOWER) }
fn conv_to_upper<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> { conv(a, cpu, "ConvToUpper", &TO_UPPER) }

/// IsAlNum .. IsXDigit: TRUE (1) when the ISO 8859-1 class table has any of `mask`'s bits for the low byte.
fn is_class<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, name: &str, mask: u16) -> Result<String, Refusal> {
    locale_arg(a, cpu.areg(0), name)?;
    let c = cpu.dreg(0);
    let yes = CTYPE[(c & 0xFF) as usize] & mask != 0;
    cpu.set_d0(yes as u32);
    Ok(format!("[locale] {name} {c:#x} -> {yes}"))
}

const SC_ASCII: u32 = 0;
const SC_COLLATE1: u32 = 1;
const SC_COLLATE2: u32 = 2;

fn coll(ty: u32, ch: u8) -> Option<i32> {
    match ty {
        SC_ASCII => Some(TO_UPPER[ch as usize] as i32),
        SC_COLLATE1 => Some(SHORT_ORDER[ch as usize] as i32),
        SC_COLLATE2 => Some(LONG_ORDER[ch as usize] as i32),
        _ => None,
    }
}

/// StrnCmp(a0 = locale, a1 = s1, a2 = s2, d0 = length (-1: whole strings), d1 = type) -> <0, 0, >0 (english.c strcompare).
fn str_n_cmp<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    locale_arg(a, cpu.areg(0), "StrnCmp")?;
    let (p1, p2, len, ty) = (cpu.areg(1), cpu.areg(2), cpu.dreg(0), cpu.dreg(1));
    if coll(ty, 0).is_none() { cpu.set_d0(0); return Ok(format!("[locale] StrnCmp type {ty}: not SC_ASCII/COLLATE1/COLLATE2 -> 0")); }
    let r = if p1 == 0 || p2 == 0 {
        if len != 0 && p1 != 0 && a.mem.read_u8(p1).map_err(Refusal::mem("StrnCmp.s1"))? != 0 { 1 }
        else if len != 0 && p2 != 0 && a.mem.read_u8(p2).map_err(Refusal::mem("StrnCmp.s2"))? != 0 { -1 } else { 0 }
    } else {
        let mut n = len;
        let (mut i, mut diff) = (0u32, 0i32);
        while n != 0 {
            n = n.wrapping_sub(1);
            let (c1, c2) = (a.mem.read_u8(p1 + i).map_err(Refusal::mem("StrnCmp.s1"))?, a.mem.read_u8(p2 + i).map_err(Refusal::mem("StrnCmp.s2"))?);
            diff = coll(ty, c1).unwrap_or(0) - coll(ty, c2).unwrap_or(0);
            if diff != 0 || c1 == 0 { break; }
            i += 1;
        }
        diff
    };
    cpu.set_d0(r as u32);
    Ok(format!("[locale] StrnCmp type {ty} -> {r}"))
}

/// StrConvert(a0 = locale, a1 = string, a2 = buffer, d0 = size, d1 = type) -> the number of characters stored.
fn str_convert<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    locale_arg(a, cpu.areg(0), "StrConvert")?;
    let (src, dst, size, ty) = (cpu.areg(1), cpu.areg(2), cpu.dreg(0), cpu.dreg(1));
    if coll(ty, 0).is_none() || size == 0 || dst == 0 { cpu.set_d0(0); return Ok(format!("[locale] StrConvert type {ty} size {size} -> 0")); }
    let mut n = 0;
    while n + 1 < size {
        let c = a.mem.read_u8(src + n).map_err(Refusal::mem("StrConvert.string"))?;
        if c == 0 { break; }
        a.mem.write_u8(dst + n, coll(ty, c).unwrap_or(0) as u8).map_err(Refusal::mem("StrConvert.buffer"))?;
        n += 1;
    }
    a.mem.write_u8(dst + n, 0).map_err(Refusal::mem("StrConvert.buffer"))?;
    cpu.set_d0(n);
    Ok(format!("[locale] StrConvert type {ty} -> {n} characters"))
}
