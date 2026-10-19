//! The clock: the `Time` low-memory global (seconds since midnight 1 January 1904) and `_ReadDateTime`.
//!
//! The engine has no wall clock (`no_std`); the host sets one with [`Mac::set_clock`] before launch, and the default is a fixed
//! date ([`DEFAULT_BOOT_SECS`], 19 October 2026) so a run is reproducible. Time then advances with the 60 Hz tick count the host
//! feeds [`Mac::advance_ticks`]: `Time = base + Ticks / 60`.
use crate::lowmem::TIME;
use crate::machine::{Call, Done, Mac};
use crate::pascal::Arg;
use crate::osconv::Regs;
use crate::Refusal;
use alloc::format;
use alloc::string::String;
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

/// 2026-10-19 00:00:00, in Mac seconds (1904 epoch).
pub const DEFAULT_BOOT_SECS: u32 = 3_875_212_800;

#[derive(Default)]
pub struct Clock { pub base: Option<u32> }

impl<U: GuestUi> Mac<U> {
    /// Set the wall-clock time (Mac seconds) the machine "booted" at; `Time` follows the tick count from here.
    pub fn set_clock(&mut self, secs: u32) -> Result<(), Refusal> {
        self.clock.base = Some(secs.wrapping_sub(self.ev.ticks / 60));
        self.update_time()
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
