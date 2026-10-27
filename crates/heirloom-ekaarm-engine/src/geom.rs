//! Geometry value types (`TPoint`, `TSize`, `TRect`: pairs and quads of words, e32std.h). `TRect` is (iTl.iX, iTl.iY, iBr.iX, iBr.iY).
use alloc::format;
use crate::loader::Process;
use crate::natives::NativeOutcome;

pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let this = p.cpu.regs[0];
    let rd = |p: &Process, a: u32| p.mem.vm.read_u32(a).map(|v| v as i32);
    let words: [i32; 4] = match name {
        // TRect::TRect(): zeroed.
        "_ZN5TRectC1Ev" => [0; 4],
        // TRect::TRect(const TPoint& aTl, const TPoint& aBr) / (const TPoint&, const TSize&): corners, or corner and extent.
        "_ZN5TRectC1ERK6TPointS2_" | "_ZN5TRectC1ERK6TPointRK5TSize" => {
            let (r1, r2) = (p.cpu.regs[1], p.cpu.regs[2]);
            let (Ok(x), Ok(y), Ok(a), Ok(b)) = (rd(p, r1), rd(p, r1 + 4), rd(p, r2), rd(p, r2 + 4)) else { return Some(NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: arguments unreadable"))) };
            if name.ends_with("TSize") { [x, y, x + a, y + b] } else { [x, y, a, b] }
        }
        _ => return None,
    };
    for (i, w) in words.iter().enumerate() {
        if p.mem.vm.write_u32(this + i as u32 * 4, *w as u32).is_err() {
            return Some(NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: the TRect at {this:#x} is unwritable")));
        }
    }
    Some(NativeOutcome::Return)
}
