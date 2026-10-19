//! utility.library date conversion: Amiga2Date, Date2Amiga, CheckDate over `struct ClockData` (utility/date.h: seven UWORDs
//! sec 0, min 2, hour 4, mday 6, month 8, year 10, wday 12 = 14 bytes). Seconds count from 1978-01-01 00:00:00 (a Sunday,
//! `wday` 0), the same calendar dos_date.rs uses for DateStamp days. Facts: AROS rom/utility/amiga2date.c, date2amiga.c, checkdate.c
//! (a date is valid when its fields are in range and it falls in 1978..=2113's seconds range; CheckDate answers 0 otherwise).
use alloc::string::String;
use alloc::format;
use crate::dos_date::{civil, days_of};
use crate::libtable::LibraryBuilder;
use crate::{machine::Amiga, Refusal};
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

const DAY: u32 = 86_400;

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.named("Amiga2Date", amiga_to_date::<U>).named("Date2Amiga", date_to_amiga::<U>).named("CheckDate", check_date::<U>)
}

fn field<U: GuestUi>(a: &Amiga<U>, cd: u32, i: u32) -> Result<u32, Refusal> { a.mem.read_u16(cd + 2 * i).map(u32::from).map_err(Refusal::mem("ClockData")) }

/// sec, min, hour, mday, month (1-12), year, wday of `secs` seconds since 1978.
fn fields_of(secs: u32) -> [u32; 7] {
    let (days, rest) = ((secs / DAY) as i32, secs % DAY);
    let (y, m, d) = civil(days);
    [rest % 60, (rest / 60) % 60, rest / 3600, d as u32, m as u32 + 1, y as u32, days.rem_euclid(7) as u32]
}

/// Amiga2Date(d0 = seconds, a0 = ClockData *): fills sec, min, hour, mday, month (1-12), year, wday.
fn amiga_to_date<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (secs, cd) = (cpu.dreg(0), cpu.areg(0));
    let vals = fields_of(secs);
    for (i, v) in vals.into_iter().enumerate() { a.mem.write_u16(cd + 2 * i as u32, v as u16).map_err(Refusal::mem("ClockData"))?; }
    Ok(format!("[utility] Amiga2Date {secs} -> {}-{:02}-{:02} {:02}:{:02}:{:02}", vals[5], vals[4], vals[3], vals[2], vals[1], vals[0]))
}

/// The seconds of a ClockData, or None when a field is out of range or the date precedes 1978.
fn seconds_of<U: GuestUi>(a: &Amiga<U>, cd: u32) -> Result<Option<u32>, Refusal> {
    let [sec, min, hour, mday, month, year] = [0, 1, 2, 3, 4, 5].map(|i| field(a, cd, i));
    let (sec, min, hour, mday, month, year) = (sec?, min?, hour?, mday?, month?, year?);
    if sec > 59 || min > 59 || hour > 23 || !(1..=12).contains(&month) || year < 1978 || mday == 0 { return Ok(None); }
    let (y, m) = (year as i32, month as usize - 1);
    let days = days_of(y, m, mday as i32);
    if civil(days) != (y, m, mday as i32) { return Ok(None); } // a day the month does not have
    Ok(u32::try_from(days).ok().and_then(|d| d.checked_mul(DAY)).and_then(|s| s.checked_add(hour * 3600 + min * 60 + sec)))
}

/// Date2Amiga(a0 = ClockData *) -> seconds since 1978 (the fields are not validated: 0 when they cannot be a date).
fn date_to_amiga<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let s = seconds_of(a, cpu.areg(0))?.unwrap_or(0);
    cpu.set_d0(s);
    Ok(format!("[utility] Date2Amiga -> {s}"))
}

/// CheckDate(a0 = ClockData *) -> the seconds when the date is valid, else 0.
fn check_date<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let s = seconds_of(a, cpu.areg(0))?.unwrap_or(0);
    cpu.set_d0(s);
    Ok(format!("[utility] CheckDate -> {s}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_epoch_is_a_sunday_and_a_leap_day_is_a_day() {
        assert_eq!(fields_of(0), [0, 0, 0, 1, 1, 1978, 0], "1978-01-01 00:00:00, a Sunday");
        assert_eq!(fields_of(2 * 365 * DAY + 2 * 3600 + 3 * 60 + 4), [4, 3, 2, 1, 1, 1980, 2], "1980-01-01 02:03:04 was a Tuesday");
        let leap_day = days_of(1980, 1, 29) as u32 * DAY;
        assert_eq!(&fields_of(leap_day)[3..6], &[29, 2, 1980]);
        // NEGATIVE CONTROL: 1981 has no Feb 29: days_of rolls into March, which the validity check (civil(days_of(..)) == date) refuses.
        assert_ne!(civil(days_of(1981, 1, 29)), (1981, 1, 29));
    }
}
