//! `_Pack6`, the International Utilities Package (Inside Macintosh I, "The International Utilities Package"; V, "The Script
//! Manager"). Date, time and string-ordering routines for ONE locale: the US English one (the machine this engine is
//! answers `IUMetric` FALSE). The International Resources ('itl0'/'itl1'/'itl2') a real call reads from the System file are
//! not provided: a handle argument is accepted and ignored (named in the log), and `IUGetIntl` finds one only in an open
//! resource file (the application's own), else it is a named refusal answering NIL.
//!
//! * `IUDateString(secs, form, VAR s)`: `shortDate` "2/12/90", `longDate` "Monday, February 12, 1990", `abbrevDate`
//!   "Mon, Feb 12, 1990" (US itl0/itl1 defaults: month and day without leading zeros, two-digit short year).
//! * `IUTimeString(secs, wantSeconds, VAR s)`: "3:45 PM" / "3:45:12 PM" (12-hour cycle, `AM`/`PM`).
//! * `IUMagString` / `IUMagIDString` (and the `P` forms taking an itl2 handle): -1, 0 or 1. Case is ignored; `IUMagString`
//!   also orders an accented letter after its base letter, `IUMagIDString` ignores accents. (The US itl2 collation tables are
//!   approximated by MacRoman's base letters; documented, not a claim of byte-exact itl2 order.)
//!
//! `secs` counts from midnight 1 January 1904 (the Mac epoch); the civil-date arithmetic is the standard days-to-civil
//! algorithm over proleptic Gregorian years.
use crate::machine::{Call, Done, Mac};
use crate::osconv::Regs;
use crate::package::{call, Routine};
use crate::pascal::{write_pstring, Arg};
use crate::Refusal;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

/// The built-in `itl0` / `itl1` handles once made (system-owned, locked).
pub type IntlHandles = [Option<u32>; 2];

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> { m.register_os(0xA9ED, pack6::<U>) }

fn pack6<U: GuestUi>(m: &mut Mac<U>, c: &mut Cpu, r: &mut Regs) -> Result<String, Refusal> { call(m, c, r, "Pack6", lookup::<U>) }

const DATE: &[Arg] = &[Arg::L, Arg::B, Arg::P];
const DATE_P: &[Arg] = &[Arg::L, Arg::B, Arg::P, Arg::P];
const MAG: &[Arg] = &[Arg::P, Arg::P, Arg::W, Arg::W];
const MAG_P: &[Arg] = &[Arg::P, Arg::P, Arg::W, Arg::W, Arg::P];

fn lookup<U: GuestUi>(sel: u16) -> Option<Routine<U>> {
    let r = |name, args, result, run| Some(Routine { name, args, result, run });
    match sel {
        0 => r("IUDateString", DATE, None, date_string::<U>),
        2 => r("IUTimeString", DATE, None, time_string::<U>),
        4 => r("IUMetric", &[], Some(Arg::B), metric::<U>),
        6 => r("IUGetIntl", &[Arg::W], Some(Arg::L), get_intl::<U>),
        10 => r("IUMagString", MAG, Some(Arg::W), mag_string::<U>),
        12 => r("IUMagIDString", MAG, Some(Arg::W), mag_id_string::<U>),
        14 | 0x14 => r(if sel == 14 { "IUDatePString" } else { "IULDateString" }, DATE_P, None, date_string::<U>),
        16 | 0x16 => r(if sel == 16 { "IUTimePString" } else { "IULTimeString" }, DATE_P, None, time_string::<U>),
        0x18 => r("IUClearCache", &[], None, clear_cache::<U>),
        0x1A => r("IUMagPString", MAG_P, Some(Arg::W), mag_string::<U>),
        0x1C => r("IUMagIDPString", MAG_P, Some(Arg::W), mag_id_string::<U>),
        _ => None,
    }
}

const SECS_1904_TO_1970: i64 = -2_082_844_800;
const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

/// (year, month 1-12, day 1-31, weekday 0 = Sunday, hour, minute, second) of a Mac-epoch second count.
pub fn civil(secs: u32) -> (i64, u32, u32, usize, u32, u32, u32) {
    let unix = i64::from(secs) + SECS_1904_TO_1970;
    let (days, tod) = (unix.div_euclid(86_400), unix.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    let wd = (days + 4).rem_euclid(7) as usize; // 1970-01-01 was a Thursday
    (y, m, d, wd, (tod / 3600) as u32, (tod % 3600 / 60) as u32, (tod % 60) as u32)
}

/// Mac seconds of a civil date (the inverse of [`civil`]); out-of-range months carry into the year.
pub fn mac_seconds(y: i64, mo: i64, d: i64, h: i64, mi: i64, s: i64) -> u32 {
    let (y, mo) = (y + (mo - 1).div_euclid(12), (mo - 1).rem_euclid(12) + 1);
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2.rem_euclid(400);
    let doy = (153 * (if mo > 2 { mo - 3 } else { mo + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    (days * 86_400 + h * 3600 + mi * 60 + s - SECS_1904_TO_1970) as u32
}

/// `DateForm`: 0 short, 1 long, 2 abbreviated.
pub fn date_text(secs: u32, form: u8) -> Result<String, String> {
    let (y, mo, d, wd, ..) = civil(secs);
    let (month, day) = (MONTHS[mo as usize - 1], DAYS[wd]);
    match form {
        0 => Ok(format!("{mo}/{d}/{:02}", y.rem_euclid(100))),
        1 => Ok(format!("{day}, {month} {d}, {y}")),
        2 => Ok(format!("{}, {} {d}, {y}", &day[..3], &month[..3])),
        f => Err(format!("DateForm {f} is not shortDate (0), longDate (1) or abbrevDate (2)")),
    }
}

pub fn time_text(secs: u32, seconds: bool) -> String {
    let (.., h, mi, s) = civil(secs);
    let (h12, ampm) = (if h % 12 == 0 { 12 } else { h % 12 }, if h < 12 { "AM" } else { "PM" });
    if seconds { format!("{h12}:{mi:02}:{s:02} {ampm}") } else { format!("{h12}:{mi:02} {ampm}") }
}

fn note_handle(c: &Call, text: &str, what: &str) -> String {
    let itl = if c.args.len() > 3 && c.args.get(3) != 0 { " (the intl handle is not consulted: US conventions)" } else { "" };
    format!("[pack6] {what} -> \"{text}\"{itl}")
}

fn date_string<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let text = date_text(c.args.get(0), c.args.get(1) as u8).map_err(Refusal)?;
    write_pstring(&mut m.mem, c.args.get(2), text.as_bytes())?;
    Ok(Done { result: 0, log: note_handle(c, &text, &format!("{} {}", c.name, c.args.get(0))) })
}

fn time_string<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let text = time_text(c.args.get(0), c.args.get(1) as u8 != 0);
    write_pstring(&mut m.mem, c.args.get(2), text.as_bytes())?;
    Ok(Done { result: 0, log: note_handle(c, &text, &format!("{} {}", c.name, c.args.get(0))) })
}

fn metric<U: GuestUi>(_: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    Ok(Done { result: 0, log: "[pack6] IUMetric -> FALSE (US English: inches)".into() })
}

fn clear_cache<U: GuestUi>(_: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    Ok(Done { result: 0, log: "[pack6] IUClearCache (nothing cached)".into() })
}

/// `IUGetIntl(theID)`: the Roman `itl0` (0) / `itl1` (1) from an open resource file when one has it, else the engine's own
/// US English record ([`itl0_us`], [`itl1_us`]); `itl2` (2, the collation tables) is a named refusal answering NIL: this engine's
/// string ordering is [`compare_with`], not a table a program could read.
fn get_intl<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let id = c.args.get(0) as u16 as i16;
    let (ty, builtin) = match id {
        0 => (*b"itl0", itl0_us().to_vec()),
        1 => (*b"itl1", itl1_us()),
        2 => return Err(Refusal("IUGetIntl 2: the itl2 collation table is not provided (string ordering is built in); NIL returned".into())),
        n => return Err(Refusal(format!("IUGetIntl {n}: only itl0, itl1 and itl2 (IDs 0-2) are known"))),
    };
    if let Ok(h) = m.get_resource(ty, 0, false) { return Ok(Done { result: h, log: format!("[pack6] IUGetIntl {id} -> {h:#x} (the application's own '{}')", core::str::from_utf8(&ty).unwrap_or("itl?")) }); }
    let slot = id as usize;
    let cached = m.intl[slot].filter(|&h| m.mm.block(h).is_some());
    let h = match cached {
        Some(h) => h,
        None => {
            let h = m.mm.new_handle(&mut m.mem, &mut m.heap, builtin.len() as u32, false).map_err(|e| Refusal(format!("IUGetIntl: no memory (memFullErr {e})")))?;
            let p = m.mm.block(h).map(|b| b.ptr).ok_or_else(|| Refusal("IUGetIntl: handle vanished".into()))?;
            m.mem.write_bytes(p, &builtin).map_err(Refusal::mem("itl"))?;
            if let Some(b) = m.mm.block_mut(h) { b.locked = true; } // system-owned: it must never move
            let _ = m.mm.sync_master(&mut m.mem, h);
            m.intl[slot] = Some(h);
            h
        }
    };
    Ok(Done { result: h, log: format!("[pack6] IUGetIntl {id} -> {h:#x} (the engine's built-in US '{}')", core::str::from_utf8(&ty).unwrap_or("itl?")) })
}

/// The US `Intl0Rec` (32 bytes, Inside Macintosh V "The Script Manager"): decimalPt 0, thousSep 1, listSep 2, currSym1-3 3-5,
/// currFmt 6, dateOrder 7, shrtDateFmt 8, dateSep 9, timeCycle 10, timeFmt 11, mornStr 12-15, eveStr 16-19, timeSep 20, time1Suff-
/// time8Suff 21-28, metricSys 29, intl0Vers 30-31. Fields filled: the separators, "$", month-day-year with "/", the 12-hour cycle
/// with "AM"/"PM" and ":" and inches. The format-flag bytes (currFmt, shrtDateFmt, timeFmt) and the suffixes are 0: their US bit
/// values were not checked against a real record, so they are left at "no flags" rather than invented.
pub fn itl0_us() -> [u8; 32] {
    let mut b = [0u8; 32];
    b[..4].copy_from_slice(b".,;$");
    b[9] = b'/';
    b[12..16].copy_from_slice(b"AM  ");
    b[16..20].copy_from_slice(b"PM  ");
    b[20] = b':';
    b
}

/// The US `Intl1Rec`: seven 16-byte day names (Sunday first), twelve 16-byte month names, then suppressDay, lngDateFmt,
/// dayLeading0, abbrLen (3) and the rest zero. Names are Pascal strings in 16-byte cells (`STRING[15]`).
pub fn itl1_us() -> alloc::vec::Vec<u8> {
    let mut b = alloc::vec![0u8; (7 + 12) * 16 + 4 + 5 * 4 + 4];
    for (i, n) in DAYS.iter().chain(MONTHS.iter()).enumerate() { b[i * 16] = n.len() as u8; b[i * 16 + 1..i * 16 + 1 + n.len()].copy_from_slice(n.as_bytes()); }
    b[19 * 16 + 3] = 3; // abbrLen
    b
}

/// MacRoman 0x80..=0x9F: the unaccented letter each accented one is built on.
const BASE: &[u8; 32] = b"AACENOUaaaaaaceeeeiiiinooooouuuu";

fn bytes_at<U: GuestUi>(m: &Mac<U>, p: u32, len: u32) -> Result<Vec<u8>, Refusal> {
    let sp = m.mem.span(p, len).map_err(Refusal::mem("IUMagString text"))?;
    m.mem.bytes(sp).map(<[u8]>::to_vec).map_err(Refusal::mem("IUMagString text"))
}

/// -1, 0, 1 under a rule: `case_sensitive` compares letters exactly, else they fold; `marks` makes an accented letter differ
/// from its base letter (and sort after it), else accents are ignored.
pub fn compare_with(a: &[u8], b: &[u8], case_sensitive: bool, marks: bool) -> i32 {
    let key = |x: u8, strip: bool| {
        let x = if strip && (0x80..=0x9F).contains(&x) { BASE[(x - 0x80) as usize] } else { x };
        if case_sensitive { x } else { x.to_ascii_uppercase() }
    };
    let ord = |o: core::cmp::Ordering| o as i32;
    let by_base = ord(a.iter().map(|&x| key(x, true)).cmp(b.iter().map(|&x| key(x, true))));
    if by_base != 0 || !marks { return by_base; }
    ord(a.iter().map(|&x| key(x, false)).cmp(b.iter().map(|&x| key(x, false))))
}

/// -1, 0, 1: by letters (case folded), then (unless `ignore_accents`) by accent.
pub fn compare(a: &[u8], b: &[u8], ignore_accents: bool) -> i32 { compare_with(a, b, false, !ignore_accents) }

fn mag<U: GuestUi>(m: &mut Mac<U>, c: &Call, ignore_accents: bool) -> Result<Done, Refusal> {
    let (a, b) = (bytes_at(m, c.args.get(0), c.args.get(2) as u16 as u32)?, bytes_at(m, c.args.get(1), c.args.get(3) as u16 as u32)?);
    let r = compare(&a, &b, ignore_accents);
    Ok(Done { result: r as i16 as u16 as u32, log: format!("[pack6] {} \"{}\" \"{}\" -> {r}", c.name, a.iter().map(|&x| char::from(x)).collect::<String>(), b.iter().map(|&x| char::from(x)).collect::<String>()) })
}
fn mag_string<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { mag(m, c, false) }
fn mag_id_string<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { mag(m, c, true) }
