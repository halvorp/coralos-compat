//! Stateless Symbian time zone server client (`RTz`, `tz.h`) backed by CoralOS's `"eka-tz"` launch record (REUSE U5).
//!
//! `Heirloom.app` reads the host time zone once at launch from `libcoral::time::zone()` (`coral-tz`) and delivers
//! a single `"eka-tz"` `Control::Input` record: `v1\t<iana>\t<offset_s>\t<0|1>\n`. This adapter holds no zone rules
//! or database of its own — it simply applies the host-supplied UTC offset in `RTz::ConvertToLocalTime` and
//! `RTz::ConvertToUniversalTime` (defaulting to UTC+0 when a host does not supply `"eka-tz"`).
use alloc::format;
use alloc::string::String;
use crate::loader::Process;
use crate::natives::NativeOutcome;

/// Named `Control::Input` carrying CoralOS's launch time zone record (REUSE-AUDIT U5).
pub const EKA_TZ_INPUT: &str = "eka-tz";

/// Parsed host launch time-zone record (`v1\t<iana>\t<offset_s>\t<0|1>\n`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostTz {
    pub iana: String,
    pub offset_secs: i32,
    pub is_dst: bool,
}

impl Default for HostTz {
    fn default() -> Self {
        Self {
            iana: String::from("UTC"),
            offset_secs: 0,
            is_dst: false,
        }
    }
}

/// Parse an `"eka-tz"` launch record (`v1\t<iana>\t<offset_s>\t<0|1>\n`).
/// Rejects malformed records with an error prefixed `"eka-tz:"` (also relied on by engine-rt's capability probe).
pub fn parse_launch_record(bytes: &[u8]) -> Result<HostTz, String> {
    let text = core::str::from_utf8(bytes).map_err(|_| String::from("eka-tz: launch record is not UTF-8"))?;
    let line = text.lines().next().unwrap_or("").trim_end_matches('\r');
    let mut parts = line.split('\t');
    let ver = parts.next().unwrap_or("");
    if ver != "v1" {
        return Err(format!("eka-tz: unsupported record version {ver:?}"));
    }
    let iana = parts.next().unwrap_or("");
    if iana.is_empty() {
        return Err(String::from("eka-tz: missing IANA zone name"));
    }
    let offset_str = parts.next().ok_or_else(|| String::from("eka-tz: missing UTC offset"))?;
    let offset_secs: i32 = offset_str
        .parse()
        .map_err(|_| format!("eka-tz: invalid UTC offset {offset_str:?}"))?;
    // Sanity bound: all real-world IANA zones fall within [-14h, +14h].
    if !(-50_400..=50_400).contains(&offset_secs) {
        return Err(format!("eka-tz: UTC offset {offset_secs} out of range"));
    }
    let dst_str = parts.next().ok_or_else(|| String::from("eka-tz: missing DST flag"))?;
    let is_dst = match dst_str {
        "0" => false,
        "1" => true,
        other => return Err(format!("eka-tz: invalid DST flag {other:?}")),
    };
    if parts.next().is_some() {
        return Err(String::from("eka-tz: unexpected trailing fields"));
    }
    Ok(HostTz {
        iana: String::from(iana),
        offset_secs,
        is_dst,
    })
}

/// A connected session to this engine's time zone server: non-zero.
const TZ_SESSION_HANDLE: u32 = 1;
const KERR_NONE: u32 = 0;

fn shift_ttime(p: &mut Process, ttime_addr: u32, delta_us: i64) -> Result<(), String> {
    let lo = p.mem.vm.read_u32(ttime_addr).map_err(|_| format!("TTime at {ttime_addr:#x} is unreadable"))?;
    let hi = p.mem.vm.read_u32(ttime_addr.wrapping_add(4)).map_err(|_| format!("TTime high word at {ttime_addr:#x} is unreadable"))?;
    let val = ((i64::from(hi as i32)) << 32) | i64::from(lo);
    let shifted = val.saturating_add(delta_us);
    p.mem.vm.write_u32(ttime_addr, shifted as u32).map_err(|_| format!("TTime at {ttime_addr:#x} is unwritable"))?;
    p.mem.vm.write_u32(ttime_addr.wrapping_add(4), (shifted >> 32) as u32).map_err(|_| format!("TTime high word at {ttime_addr:#x} is unwritable"))?;
    Ok(())
}

/// The natives of this module; `None` = not ours.
pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let this = p.cpu.regs[0];
    let set = |p: &mut Process, v: u32| -> NativeOutcome {
        match p.mem.vm.write_u32(this, v) {
            Ok(()) => NativeOutcome::Return,
            Err(_) => NativeOutcome::Refused(format!("NEXT-NEEDED tzclient {name}: the session at {this:#x} is unwritable")),
        }
    };
    Some(match name {
        "_ZN3RTzC1Ev" => set(p, 0),
        "_ZN3RTz7ConnectEv" => {
            let o = set(p, TZ_SESSION_HANDLE);
            p.cpu.regs[0] = KERR_NONE;
            o
        }
        "_ZN3RTz5CloseEv" | "_ZN3RTzD1Ev" => set(p, 0),
        // TInt RTz::ConvertToLocalTime(TTime&): UTC -> host local time using CoralOS launch tz offset.
        "_ZNK3RTz18ConvertToLocalTimeER5TTime" => {
            if p.mem.vm.read_u32(this).map_or(true, |h| h == 0) {
                return Some(NativeOutcome::Refused(String::from("NEXT-NEEDED tzclient ConvertToLocalTime: the session is not connected")));
            }
            let delta_us = i64::from(p.host_tz.offset_secs) * 1_000_000;
            if let Err(m) = shift_ttime(p, p.cpu.regs[1], delta_us) {
                return Some(NativeOutcome::Refused(format!("NEXT-NEEDED tzclient ConvertToLocalTime: {m}")));
            }
            p.cpu.regs[0] = KERR_NONE;
            NativeOutcome::Return
        }
        // TInt RTz::ConvertToUniversalTime(TTime&): host local time -> UTC using CoralOS launch tz offset.
        "_ZNK3RTz22ConvertToUniversalTimeER5TTime" => {
            if p.mem.vm.read_u32(this).map_or(true, |h| h == 0) {
                return Some(NativeOutcome::Refused(String::from("NEXT-NEEDED tzclient ConvertToUniversalTime: the session is not connected")));
            }
            let delta_us = -i64::from(p.host_tz.offset_secs) * 1_000_000;
            if let Err(m) = shift_ttime(p, p.cpu.regs[1], delta_us) {
                return Some(NativeOutcome::Refused(format!("NEXT-NEEDED tzclient ConvertToUniversalTime: {m}")));
            }
            p.cpu.regs[0] = KERR_NONE;
            NativeOutcome::Return
        }
        _ => return None,
    })
}
