//! Time value types. `TDateTime` is seven words, in this order (the class is not in the SDK header tree; this is the layout of the
//! public class): `iYear`, `iMonth`, `iDay`, `iHour`, `iMinute`, `iSecond`, `iMicroSecond`.
use alloc::format;
use crate::loader::Process;
use crate::natives::NativeOutcome;

pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    match name {
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
        // TTime& TTime::operator=(const TDateTime&): microseconds since 00:00:00 1 Jan 0 AD (year 0 counted as a leap year, which puts
        // 1970-01-01 at 62167219200 s). TDateTime's month and day are zero-based. TTime is one 64-bit word pair.
        "_ZN5TTimeaSERK9TDateTime" => {
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
        _ => None,
    }
}
