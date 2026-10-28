//! The clock: the `Time` low-memory global (seconds since midnight 1 January 1904) and `_ReadDateTime`.
//!
//! The engine has no wall clock (`no_std`); the host sets one with [`Mac::set_clock`] before launch, and the default is a fixed
//! date ([`DEFAULT_BOOT_SECS`], 19 October 2026) so a run is reproducible. Time then advances with the 60 Hz tick count the host
//! feeds [`Mac::advance_ticks`]: `Time = base + Ticks / 60`.
//!
//! **The host's clock is virtual microseconds** ([`heirloom_common::GuestMachine`]); a host never counts ticks. The conversion is
//! one rule, exact integer arithmetic, no float: the machine runs at exactly 60 Hz (a tick is 1/60 s; the real Mac's 60.15 Hz
//! is not modelled), tick `t` begins at `ceil(t * 1_000_000 / 60)` microseconds, and so `ticks_at(us) = floor(us * 60 / 1_000_000)`
//! ticks have elapsed at `us`. The two are inverse at the boundary: `ticks_at(us_at_tick(t)) == t` and one microsecond earlier
//! is `t - 1`. A guest whose wait ends at tick `d` therefore wakes at `us_at_tick(d)`, the earliest microsecond the clock
//! shows that tick, and not one microsecond before.
use crate::lowmem::TIME;
use crate::machine::{Call, Done, Mac};
use crate::pascal::Arg;
use crate::osconv::Regs;
use crate::Refusal;
use alloc::format;
use alloc::string::String;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

/// 2026-10-19 00:00:00, in Mac seconds (1904 epoch).
pub const DEFAULT_BOOT_SECS: u32 = 3_875_212_800;
/// Seconds from 1904-01-01 (the Mac epoch) to 1970-01-01 (Unix): 66 years, 17 leap days.
pub const UNIX_TO_MAC_SECS: i64 = 2_082_844_800;

#[derive(Default)]
pub struct Clock { pub base: Option<u32> }

const US_PER_SEC: u128 = 1_000_000;
const TICKS_PER_SEC: u128 = 60;

/// The 60 Hz ticks elapsed at virtual microsecond `us` (at most 1.2e15: it cannot overflow).
pub fn ticks_at(us: u64) -> u64 { (u128::from(us) * TICKS_PER_SEC / US_PER_SEC) as u64 }

/// The first virtual microsecond at which tick `t` has begun (saturating: tick 1.1e15 is ~600 years out).
pub fn us_at_tick(t: u64) -> u64 { u64::try_from((u128::from(t) * US_PER_SEC).div_ceil(TICKS_PER_SEC)).unwrap_or(u64::MAX) }

impl<U: GuestUi> Mac<U> {
    /// Set the wall-clock time (Mac seconds) the machine "booted" at; `Time` follows the tick count from here.
    pub fn set_clock(&mut self, secs: u32) -> Result<(), Refusal> {
        self.clock.base = Some(secs.wrapping_sub(self.ev.ticks / 60));
        self.update_time()
    }
    /// Move the clock to virtual microsecond `us` (never backwards): the ticks it has not yet counted pass, each retracing and
    /// timing out the waits it ends.
    pub fn advance_clock_to_us(&mut self, us: u64) {
        let mut owed = ticks_at(us).saturating_sub(self.ev.ticks_total);
        while owed > 0 {
            let n = u32::try_from(owed).unwrap_or(u32::MAX);
            self.advance_ticks(n);
            owed -= u64::from(n);
        }
    }

    /// The virtual microsecond at which the blocked event call times out (`WaitNextEvent`'s sleep, or the next tick for
    /// `GetNextEvent`); `None` when nothing waits on time. A wait whose deadline has passed reports the current tick's start.
    pub fn next_deadline_us(&self) -> Option<u64> {
        let d = self.ev.waiting?.deadline?;
        let ahead = d.wrapping_sub(self.ev.ticks);
        let left = if ahead < 0x8000_0000 { u64::from(ahead) } else { 0 };
        Some(us_at_tick(self.ev.ticks_total + left))
    }

    /// Rewrite the `Time` global from the base and the ticks.
    pub(crate) fn update_time(&mut self) -> Result<(), Refusal> {
        let now = self.clock.base.unwrap_or(DEFAULT_BOOT_SECS).wrapping_add(self.ev.ticks / 60);
        self.mem.write_u32(TIME, now).map_err(Refusal::mem("Time"))
    }
}

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    m.register_os(0xA039, read_date_time::<U>)?;
    m.register(0xA9C6, &[Arg::L, Arg::P], None, seconds_to_date::<U>)?;
    m.register(0xA9C7, &[Arg::P, Arg::P], None, date_to_seconds::<U>)
}

/// `SecondsToDate(secs, VAR d)`: `DateTimeRec` = year, month, day, hour, minute, second, dayOfWeek (seven INTEGERs; Sunday is 1).
fn seconds_to_date<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (secs, at) = (c.args.get(0), c.args.get(1));
    let (y, mo, d, wd, h, mi, s) = crate::intl::civil(secs);
    for (i, v) in [y as u16, mo as u16, d as u16, h as u16, mi as u16, s as u16, wd as u16 + 1].into_iter().enumerate() {
        m.mem.write_u16(at + 2 * i as u32, v).map_err(Refusal::mem("DateTimeRec"))?;
    }
    Ok(Done { result: 0, log: format!("[clock] SecondsToDate {secs} -> {y}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}, weekday {}", wd + 1) })
}

/// `DateToSeconds(d, VAR secs)`: the inverse (the weekday field is ignored). Fields out of range are normalised by the
/// arithmetic (month 13 is next January), as the Toolbox's own conversion does.
fn date_to_seconds<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (at, out) = (c.args.get(0), c.args.get(1));
    let f = |i: u32| m.mem.read_u16(at + 2 * i).map(i64::from).map_err(Refusal::mem("DateTimeRec"));
    let (y, mo, d, h, mi, s) = (f(0)?, f(1)?, f(2)?, f(3)?, f(4)?, f(5)?);
    let secs = crate::intl::mac_seconds(y, mo, d, h, mi, s);
    m.mem.write_u32(out, secs).map_err(Refusal::mem("DateToSeconds result"))?;
    Ok(Done { result: 0, log: format!("[clock] DateToSeconds {y}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02} -> {secs}") })
}

/// `ReadDateTime(VAR secs)`: A0 = the LONGINT to fill; D0 = noErr (the battery clock is always readable).
fn read_date_time<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> Result<String, Refusal> {
    let secs = m.mem.read_u32(TIME).map_err(Refusal::mem("Time"))?;
    m.mem.write_u32(r.a0, secs).map_err(Refusal::mem("ReadDateTime result"))?;
    r.err(0);
    Ok(format!("[clock] ReadDateTime -> {secs}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tick_begins_on_the_first_microsecond_that_shows_it() {
        for t in [0u64, 1, 2, 3, 59, 60, 61, 3600, 1_000_003, u64::from(u32::MAX), u64::from(u32::MAX) + 7] {
            let us = us_at_tick(t);
            assert_eq!(ticks_at(us), t, "tick {t} is on at {us} us");
            if t > 0 { assert_eq!(ticks_at(us - 1), t - 1, "and not one microsecond before"); }
        }
        // the 60 Hz machine: a second is 60 ticks, a tick is 16_667 us rounded up (16_666.67)
        assert_eq!((us_at_tick(60), us_at_tick(1), us_at_tick(2)), (1_000_000, 16_667, 33_334));
        assert_eq!((ticks_at(16_666), ticks_at(16_667), ticks_at(999_999), ticks_at(1_000_000)), (0, 1, 59, 60));
    }

    #[test]
    fn the_conversion_is_monotonic_and_never_overflows() {
        assert_eq!(ticks_at(u64::MAX), (u128::from(u64::MAX) * 60 / 1_000_000) as u64);
        assert_eq!(us_at_tick(u64::MAX), u64::MAX, "saturates rather than wrapping");
        let mut last = 0;
        for us in (0..2_000_000).step_by(997) { let t = ticks_at(us); assert!(t >= last); last = t; }
    }
}
