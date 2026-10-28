//! DateToStr / StrToDate on `struct DateTime` (dos/datetime.h). Layout: m68k gcc -m68020 -mno-align-int over the AROS
//! field list: dat_Stamp 0 (3 longs), dat_Format 12, dat_Flags 13, dat_StrDay 14, dat_StrDate 18, dat_StrTime 22 (26
//! bytes). A DateStamp counts days since 1978-01-01 (a Sunday), minutes into the day, and ticks of 1/50 s.
//! Formats: FORMAT_DOS 0 `dd-mmm-yy`, FORMAT_INT 1 `yy-mmm-dd`, FORMAT_USA 2 `mm-dd-yy`, FORMAT_CDN 3 `dd-mm-yy`,
//! FORMAT_DEF 4 (the locale default: DOS here, there is no locale). DTF_SUBST (flag 1) writes Today / Tomorrow /
//! Yesterday (and weekday names within the last week) in dat_StrDate's place; DTF_FUTURE (2) lets StrToDate read a
//! bare weekday as the next one.
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use crate::dos_err::*;
use crate::fs::*;
use crate::libtable::LibraryBuilder;
use crate::{machine::Amiga, Refusal};
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

const DAT_FORMAT: u32 = 12;
const DAT_FLAGS: u32 = 13;
const DAT_STR_DAY: u32 = 14;
const DAT_STR_DATE: u32 = 18;
const DAT_STR_TIME: u32 = 22;
const DTF_SUBST: u8 = 1;
const DTF_FUTURE: u8 = 2;
const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.named("DateToStr", date_to_str::<U>).named("StrToDate", str_to_date::<U>)
}

fn leap(y: i32) -> bool { y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) }
fn month_len(y: i32, m: usize) -> i32 { [31, if leap(y) { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][m] }

/// (year, month 0-11, day 1-31) of a day count since 1978-01-01.
pub fn civil(mut days: i32) -> (i32, usize, i32) {
    let mut y = 1978;
    while days < 0 { y -= 1; days += if leap(y) { 366 } else { 365 }; }
    while days >= if leap(y) { 366 } else { 365 } { days -= if leap(y) { 366 } else { 365 }; y += 1; }
    let mut m = 0;
    while days >= month_len(y, m) { days -= month_len(y, m); m += 1; }
    (y, m, days + 1)
}

/// The day count of a calendar date (inverse of [`civil`]).
pub fn days_of(y: i32, m: usize, d: i32) -> i32 {
    let mut n = 0;
    if y >= 1978 { for yy in 1978..y { n += if leap(yy) { 366 } else { 365 }; } } else { for yy in y..1978 { n -= if leap(yy) { 366 } else { 365 }; } }
    for mm in 0..m { n += month_len(y, mm); }
    n + d - 1
}

fn rd<U: GuestUi>(a: &Amiga<U>, at: u32) -> Result<u32, Refusal> { a.mem.read_u32(at).map_err(Refusal::mem("DateTime")) }

/// The text of the date part for `format` (two-digit years: 78-99 -> 19xx, 00-77 -> 20xx on the way back in).
fn date_text(days: i32, format: u8) -> String {
    let (y, m, d) = civil(days);
    let yy = y.rem_euclid(100);
    match format {
        1 => format!("{yy:02}-{}-{d:02}", MONTHS[m]),
        2 => format!("{:02}-{d:02}-{yy:02}", m + 1),
        3 => format!("{d:02}-{:02}-{yy:02}", m + 1),
        _ => format!("{d:02}-{}-{yy:02}", MONTHS[m]),
    }
}

/// DateToStr(d1 = DateTime) -> DOSTRUE. dat_StrDay / dat_StrDate / dat_StrTime, each only if its pointer is non-NULL.
fn date_to_str<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let dt = cpu.dreg(1);
    let m = Refusal::mem("DateTime");
    let (days, mins, ticks) = (rd(a, dt)? as i32, rd(a, dt + 4)?, rd(a, dt + 8)?);
    let (format, flags) = (a.mem.read_u8(dt + DAT_FORMAT).map_err(&m)?, a.mem.read_u8(dt + DAT_FLAGS).map_err(&m)?);
    let what = format!("[dos] DateToStr day {days}");
    if mins >= 24 * 60 || ticks >= 60 * 50 || format > 4 { return fail(a, cpu, ERROR_BAD_NUMBER, 0, what); }
    let today = crate::dos_misc::now(a)[0];
    let weekday = DAYS[days.rem_euclid(7) as usize];
    let (day_text, date) = match (flags & DTF_SUBST != 0, days - today) {
        (true, 0) => ("Today".to_string(), "Today".to_string()),
        (true, 1) => ("Tomorrow".to_string(), "Tomorrow".to_string()),
        (true, -1) => ("Yesterday".to_string(), "Yesterday".to_string()),
        (true, d) if (-6..0).contains(&d) => (weekday.to_string(), weekday.to_string()),
        _ => (weekday.to_string(), date_text(days, format)),
    };
    let time = format!("{:02}:{:02}:{:02}", mins / 60, mins % 60, ticks / 50);
    for (off, s) in [(DAT_STR_DAY, day_text), (DAT_STR_DATE, date), (DAT_STR_TIME, time)] {
        let p = rd(a, dt + off)?;
        if p != 0 { crate::guest_str::write_z(&mut a.mem, p, s.as_bytes()).map_err(&m)?; }
    }
    ok(a, cpu, DOSTRUE, what)
}

use alloc::string::ToString;
const ERROR_BAD_NUMBER: DosErr = DosErr(115);

fn num(s: &str) -> Option<i32> { if s.is_empty() || s.len() > 4 || !s.bytes().all(|c| c.is_ascii_digit()) { None } else { s.parse().ok() } }

fn month_of(s: &str) -> Option<usize> { MONTHS.iter().position(|m| m.eq_ignore_ascii_case(s)) }

fn year(yy: i32) -> i32 { if yy >= 100 { yy } else if yy >= 78 { 1900 + yy } else { 2000 + yy } }

/// Parse the date text into a day count.
fn parse_date(s: &str, format: u8, flags: u8, today: i32) -> Option<i32> {
    let t = s.trim();
    for (word, delta) in [("today", 0), ("tomorrow", 1), ("yesterday", -1)] { if t.eq_ignore_ascii_case(word) { return Some(today + delta); }}
    if let Some(wd) = DAYS.iter().position(|d| d.eq_ignore_ascii_case(t)) {
        let back = (today.rem_euclid(7) - wd as i32).rem_euclid(7);
        return Some(if flags & DTF_FUTURE != 0 { today + (7 - back) % 7 } else { today - back });
    }
    let parts: Vec<&str> = t.split('-').collect();
    let [p0, p1, p2] = parts[..] else { return None };
    let (y, m, d) = match format {
        1 => (year(num(p0)?), month_of(p1).or_else(|| num(p1).map(|n| n as usize - 1))?, num(p2)?),
        2 => (year(num(p2)?), (num(p0)? - 1) as usize, num(p1)?),
        3 => (year(num(p2)?), (num(p1)? - 1) as usize, num(p0)?),
        _ => (year(num(p2)?), month_of(p1).or_else(|| num(p1).map(|n| n as usize - 1))?, num(p0)?),
    };
    if m >= 12 || d < 1 || d > month_len(y, m) { return None; }
    Some(days_of(y, m, d))
}

/// StrToDate(d1 = DateTime) -> DOSTRUE: reads dat_StrDate and dat_StrTime (each if non-NULL) into dat_Stamp.
fn str_to_date<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let dt = cpu.dreg(1);
    let m = Refusal::mem("DateTime");
    let (format, flags) = (a.mem.read_u8(dt + DAT_FORMAT).map_err(&m)?, a.mem.read_u8(dt + DAT_FLAGS).map_err(&m)?);
    let (dp, tp) = (rd(a, dt + DAT_STR_DATE)?, rd(a, dt + DAT_STR_TIME)?);
    let what = format!("[dos] StrToDate fmt {format}");
    if dp == 0 && tp == 0 { return fail(a, cpu, ERROR_BAD_NUMBER, 0, what); }
    let today = crate::dos_misc::now(a)[0];
    let (mut days, mut mins, mut ticks) = (today, 0u32, 0u32);
    if dp != 0 {
        let s = a.mem.read_cstr(dp).map_err(&m)?;
        match parse_date(&s, if format > 4 { 0 } else { format }, flags, today) { Some(d) => days = d, None => return fail(a, cpu, ERROR_BAD_NUMBER, 0, format!("{what} \"{s}\"")) }
    }
    if tp != 0 {
        let s = a.mem.read_cstr(tp).map_err(&m)?;
        let p: Vec<Option<u32>> = s.trim().split(':').map(|x| x.parse().ok()).collect();
        match p[..] {
            [Some(h), Some(mi)] if h < 24 && mi < 60 => { mins = h * 60 + mi; }
            [Some(h), Some(mi), Some(se)] if h < 24 && mi < 60 && se < 60 => { mins = h * 60 + mi; ticks = se * 50; }
            _ => return fail(a, cpu, ERROR_BAD_NUMBER, 0, format!("{what} time \"{s}\"")),
        }
    }
    for (i, v) in [days as u32, mins, ticks].into_iter().enumerate() { a.mem.write_u32(dt + 4 * i as u32, v).map_err(&m)?; }
    ok(a, cpu, DOSTRUE, what)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_round_trips_across_leap_years_and_the_epoch() {
        assert_eq!(civil(0), (1978, 0, 1));
        assert_eq!(civil(364), (1978, 11, 31));
        assert_eq!(civil(365), (1979, 0, 1));
        for d in (-800..20000).step_by(37) { let (y, m, dd) = civil(d); assert_eq!(days_of(y, m, dd), d, "day {d}"); }
        assert_eq!(civil(days_of(2000, 1, 29)), (2000, 1, 29), "2000 is a leap year");
        assert_eq!(civil(days_of(2100, 2, 1)), (2100, 2, 1));
        assert_eq!(date_text(days_of(2026, 9, 18), 0), "18-Oct-26");
        assert_eq!(date_text(days_of(2026, 9, 18), 1), "26-Oct-18");
        assert_eq!(date_text(days_of(2026, 9, 18), 2), "10-18-26");
        assert_eq!(date_text(days_of(2026, 9, 18), 3), "18-10-26");
    }

    #[test]
    fn dates_parse_in_every_format_and_reject_nonsense() {
        let t = days_of(2026, 9, 18);
        assert_eq!(parse_date("18-Oct-26", 0, 0, t), Some(t));
        assert_eq!(parse_date("18-oct-26", 0, 0, t), Some(t));
        assert_eq!(parse_date("26-Oct-18", 1, 0, t), Some(t));
        assert_eq!(parse_date("10-18-26", 2, 0, t), Some(t));
        assert_eq!(parse_date("18-10-26", 3, 0, t), Some(t));
        assert_eq!(parse_date("01-Jan-78", 0, 0, t), Some(0));
        assert_eq!(parse_date("yesterday", 0, 0, t), Some(t - 1));
        assert_eq!(parse_date("30-Feb-26", 0, 0, t), None, "no such day");
        assert_eq!(parse_date("18-Foo-26", 0, 0, t), None);
        assert_eq!(parse_date("18-Oct", 0, 0, t), None);
        let wd = DAYS[t.rem_euclid(7) as usize];
        assert_eq!(parse_date(wd, 0, 0, t), Some(t), "today's weekday name is today");
        assert_eq!(parse_date(wd, 0, DTF_FUTURE, t), Some(t));
    }
}
