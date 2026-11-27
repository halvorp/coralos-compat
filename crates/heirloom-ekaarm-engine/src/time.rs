//! Time value types. `TDateTime` is seven words, in this order (the class is not in the SDK header tree; this is the layout of the
//! public class): `iYear`, `iMonth`, `iDay`, `iHour`, `iMinute`, `iSecond`, `iMicroSecond`.
use alloc::format;
use crate::loader::Process;
use crate::natives::NativeOutcome;

pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    match name {
        // TTime::TTime(const TDesC16& aString) (e32std.h): parses "YYYYMMDD:HHMMSS.MMMMMM" (month & day 0-based in Symbian!).
        "_ZN5TTimeC1ERK7TDesC16" | "_ZN5TTimeC2ERK7TDesC16" => {
            let (this, des) = (p.cpu.regs[0], p.cpu.regs[1]);
            let s = match crate::desc::read16(p, des) {
                Ok(u) => crate::desc::to_utf8(&u),
                Err(m) => return Some(NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: {m}"))),
            };
            let (date_s, time_s) = s.split_once(':').unwrap_or((s.as_str(), ""));
            let y: i64 = date_s.get(0..4).and_then(|x| x.parse().ok()).unwrap_or(1970);
            let mo: i64 = date_s.get(4..6).and_then(|x| x.parse().ok()).unwrap_or(0);
            let d: i64 = date_s.get(6..8).and_then(|x| x.parse().ok()).unwrap_or(0);
            let (hms, frac) = time_s.split_once('.').unwrap_or((time_s, ""));
            let h: i64 = hms.get(0..2).and_then(|x| x.parse().ok()).unwrap_or(0);
            let mi: i64 = hms.get(2..4).and_then(|x| x.parse().ok()).unwrap_or(0);
            let sec: i64 = hms.get(4..6).and_then(|x| x.parse().ok()).unwrap_or(0);
            let us: i64 = frac.parse().unwrap_or(0);
            let leap = |yr: i64| (yr % 4 == 0 && yr % 100 != 0) || yr % 400 == 0;
            let before_year = 365 * y + (y + 3) / 4 - (y + 99) / 100 + (y + 399) / 400;
            const DAYS: [i64; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
            let before_month: i64 = (0..(mo.clamp(0, 11) as usize)).map(|m| DAYS[m] + (m == 1 && leap(y)) as i64).sum();
            let days = before_year + before_month + d;
            let micro = (((days * 24 + h) * 60 + mi) * 60 + sec) * 1_000_000 + us;
            let ok = p.mem.vm.write_u32(this, micro as u64 as u32).is_ok()
                && p.mem.vm.write_u32(this.wrapping_add(4), ((micro as u64) >> 32) as u32).is_ok();
            if ok { p.cpu.regs[0] = this; Some(NativeOutcome::Return) } else { Some(NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: TTime unwritable"))) }
        }
        // TTimeIntervalMicroSeconds TTime::MicroSecondsFrom(TTime aTime) const (e32std.h):
        // ARM AAPCS: TTimeIntervalMicroSeconds is an 8-byte struct (TInt64), returned in r0:r1 (or via sret? No, 8-byte C++ POD / non-POD:
        // on Symbian RVCT/GCCE TTimeIntervalMicroSeconds has a constructor so it's returned via sret in r0, this in r1, aTime in r2:r3).
        "_ZNK5TTime16MicroSecondsFromES_" => {
            let (sret, this) = (p.cpu.regs[0], p.cpu.regs[1]);
            let a_time = (u64::from(p.cpu.regs[2]) | (u64::from(p.cpu.regs[3]) << 32)) as i64;
            let (Ok(lo), Ok(hi)) = (p.rd(this), p.rd(this.wrapping_add(4))) else {
                return Some(NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: TTime unreadable")));
            };
            let cur = (u64::from(lo) | (u64::from(hi) << 32)) as i64;
            let diff = cur.wrapping_sub(a_time) as u64;
            let ok = p.mem.vm.write_u32(sret, diff as u32).is_ok()
                && p.mem.vm.write_u32(sret.wrapping_add(4), (diff >> 32) as u32).is_ok();
            p.cpu.regs[0] = sret;
            if ok { Some(NativeOutcome::Return) } else { Some(NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: sret {sret:#x} unwritable"))) }
        }
        // void TTime::HomeTime(): microseconds since 00:00:00 1 Jan 0 AD (2026-11-22 base + virtual clock).
        "_ZN5TTime8HomeTimeEv" | "_ZN5TTime13UniversalTimeEv" => {
            let this = p.cpu.regs[0];
            let us: u64 = 63_936_000_000_000_000u64
                .wrapping_add(p.nat.clock_us)
                .wrapping_add(p.insns / 10);
            let ok = p.mem.vm.write_u32(this, us as u32).is_ok()
                && p.mem.vm.write_u32(this.wrapping_add(4), (us >> 32) as u32).is_ok();
            if ok { Some(NativeOutcome::Return) } else { Some(NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: TTime at {this:#x} unwritable"))) }
        }
        // TDateTime::TDateTime(aYear, aMonth, aDay, aHour, aMinute, aSecond, aMicroSecond): r1-r3 then the stack (AAPCS).
        "_ZN9TDateTimeC1Ei6TMonthiiiii" => {
            let this = p.cpu.regs[0];
            let sp = p.cpu.regs[13];
            let mut v = [p.cpu.regs[1], p.cpu.regs[2], p.cpu.regs[3], 0, 0, 0, 0];
            for (i, slot) in v.iter_mut().enumerate().skip(3) {
                match p.mem.vm.read_u32(sp + (i as u32 - 3) * 4) { Ok(w) => *slot = w, Err(_) => return Some(NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: stack arguments unreadable"))) }
            }
            for (i, w) in v.iter().enumerate() {
                if p.mem.vm.write_u32(this + i as u32 * 4, *w).is_err() {
                    return Some(NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: the TDateTime at {this:#x} is unwritable")));
                }
            }
            Some(NativeOutcome::Return)
        }
        // TTime::TTime(const TDateTime&) / TTime& TTime::operator=(const TDateTime&): microseconds since 00:00:00 1 Jan 0 AD (year 0 counted as a leap year, which puts
        // 1970-01-01 at 62167219200 s). TDateTime's month and day are zero-based. TTime is one 64-bit word pair.
        "_ZN5TTimeaSERK9TDateTime" | "_ZN5TTimeC1ERK9TDateTime" | "_ZN5TTimeC2ERK9TDateTime" => {
            let (this, src) = (p.cpu.regs[0], p.cpu.regs[1]);
            let mut f = [0i64; 7];
            for (i, slot) in f.iter_mut().enumerate() {
                match p.mem.vm.read_u32(src + i as u32 * 4) { Ok(w) => *slot = w as i32 as i64, Err(_) => return Some(NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: the TDateTime at {src:#x} is unreadable"))) }
            }
            let [y, mo, d, h, mi, s, us] = f;
            if y < 0 || !(0..12).contains(&mo) || !(0..=30).contains(&d) || h > 23 || mi > 59 || s > 59 {
                return Some(NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: TDateTime {y}-{mo}-{d} {h}:{mi}:{s} is out of range (a KErrGeneral from the checked setters)")));
            }
            let leap = |y: i64| (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
            let before_year = 365 * y + (y + 3) / 4 - (y + 99) / 100 + (y + 399) / 400;
            const DAYS: [i64; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
            let before_month: i64 = (0..mo as usize).map(|m| DAYS[m] + (m == 1 && leap(y)) as i64).sum();
            let days = before_year + before_month + d;
            let micro = (((days * 24 + h) * 60 + mi) * 60 + s) * 1_000_000 + us;
            let ok = p.mem.vm.write_u32(this, micro as u64 as u32).is_ok() && p.mem.vm.write_u32(this + 4, ((micro as u64) >> 32) as u32).is_ok();
            if ok { p.cpu.regs[0] = this; Some(NativeOutcome::Return) } else { Some(NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: the TTime at {this:#x} is unwritable"))) }
        }
        // TInt TTime::SecondsFrom(TTime aTime, TTimeIntervalSeconds& aInterval) const (e32std.h):
        // ARM AAPCS: r0 = this, r2:r3 = aTime (8-byte aligned TInt64), [sp] = &aInterval (or r1 if unaligned; read [sp] first,
        // falling back to r1 if [sp] is not a valid stack/heap pointer).
        "_ZNK5TTime11SecondsFromES_R20TTimeIntervalSeconds" => {
            let this = p.cpu.regs[0];
            let a_time = (u64::from(p.cpu.regs[2]) | (u64::from(p.cpu.regs[3]) << 32)) as i64;
            let sp = p.cpu.regs[13];
            let out_ptr = p.rd(sp).ok().filter(|&a| a >= 0x0400_0000).unwrap_or(p.cpu.regs[1]);
            let (Ok(lo), Ok(hi)) = (p.rd(this), p.rd(this.wrapping_add(4))) else {
                return Some(NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: TTime unreadable")));
            };
            let cur = (u64::from(lo) | (u64::from(hi) << 32)) as i64;
            let secs = cur.saturating_sub(a_time) / 1_000_000;
            if secs < i64::from(i32::MIN) || secs > i64::from(i32::MAX) {
                p.cpu.regs[0] = (-9i32) as u32; // KErrOverflow
            } else {
                let _ = p.wr(out_ptr, secs as i32 as u32);
                p.cpu.regs[0] = 0;
            }
            Some(NativeOutcome::Return)
        }
        // TDateTime TTime::DateTime() const (hidden return pointer in r0, this in r1).
        "_ZNK5TTime8DateTimeEv" => {
            let (out_ptr, this) = (p.cpu.regs[0], p.cpu.regs[1]);
            let (Ok(lo), Ok(hi)) = (p.rd(this), p.rd(this.wrapping_add(4))) else {
                return Some(NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: TTime unreadable")));
            };
            let us = (u64::from(lo) | (u64::from(hi) << 32)) as i64;
            let micro = us.rem_euclid(1_000_000) as i32;
            let total_secs = us.div_euclid(1_000_000);
            let sec = total_secs.rem_euclid(60) as i32;
            let total_mins = total_secs.div_euclid(60);
            let min = total_mins.rem_euclid(60) as i32;
            let total_hrs = total_mins.div_euclid(60);
            let hr = total_hrs.rem_euclid(24) as i32;
            let mut days = total_hrs.div_euclid(24);
            let leap = |y: i64| (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
            let mut year = 0i64;
            while days >= if leap(year) { 366 } else { 365 } {
                days -= if leap(year) { 366 } else { 365 };
                year += 1;
            }
            const DAYS: [i64; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
            let mut month = 0i32;
            while (month as usize) < 11 {
                let md = DAYS[month as usize] + if month == 1 && leap(year) { 1 } else { 0 };
                if days < md { break; }
                days -= md;
                month += 1;
            }
            for (i, v) in [year as i32, month, days as i32, hr, min, sec, micro].iter().enumerate() {
                let _ = p.wr(out_ptr + (i as u32) * 4, *v as u32);
            }
            p.cpu.regs[0] = out_ptr;
            Some(NativeOutcome::Return)
        }
        // void TTime::FormatL(TDes16& aDes, const TDesC16& aFormat) const
        "_ZNK5TTime7FormatLER6TDes16RK7TDesC16" => {
            let (this, dst, fmt_addr) = (p.cpu.regs[0], p.cpu.regs[1], p.cpu.regs[2]);
            let res = (|| -> Result<(), alloc::string::String> {
                let lo = p.rd(this)? as u64;
                let hi = p.rd(this.wrapping_add(4))? as u64;
                let us = lo | (hi << 32);
                let total_secs = (us / 1_000_000) as i64;
                let sec = total_secs.rem_euclid(60);
                let total_mins = total_secs.div_euclid(60);
                let min = total_mins.rem_euclid(60);
                let total_hrs = total_mins.div_euclid(60);
                let hr = total_hrs.rem_euclid(24);
                let mut days = total_hrs.div_euclid(24);
                let leap = |y: i64| (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
                let mut year = 0i64;
                while days >= if leap(year) { 366 } else { 365 } {
                    days -= if leap(year) { 366 } else { 365 };
                    year += 1;
                }
                const DAYS: [i64; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
                let mut month = 0usize;
                while month < 11 {
                    let md = DAYS[month] + if month == 1 && leap(year) { 1 } else { 0 };
                    if days < md { break; }
                    days -= md;
                    month += 1;
                }
                let fmt_u = crate::desc::read16(p, fmt_addr)?;
                let fmt_s = crate::desc::to_utf8(&fmt_u);
                let mut out = alloc::string::String::new();
                let mut chars = fmt_s.chars().peekable();
                while let Some(c) = chars.next() {
                    if c == '%' {
                        match chars.next() {
                            Some('F') | Some('*') | Some('-') | Some('+') => {}
                            Some('D') => out.push_str(&format!("{:02}", days + 1)),
                            Some('M') => out.push_str(&format!("{:02}", month + 1)),
                            Some('Y') => out.push_str(&format!("{:04}", year)),
                            Some('H') | Some('J') => out.push_str(&format!("{:02}", hr)),
                            Some('T') => out.push_str(&format!("{:02}", min)),
                            Some('S') => out.push_str(&format!("{:02}", sec)),
                            Some('%') => out.push('%'),
                            Some(other) => out.push(other),
                            None => break,
                        }
                    } else {
                        out.push(c);
                    }
                }
                let units: alloc::vec::Vec<u16> = out.encode_utf16().collect();
                crate::desc::write16(p, dst, &units)
            })();
            match res {
                Ok(()) => Some(NativeOutcome::Return),
                Err(m) => Some(NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: {m}"))),
            }
        }
        _ => None,
    }
}
