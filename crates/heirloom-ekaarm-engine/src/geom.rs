//! Geometry value types (`TPoint`, `TSize`, `TRect`: pairs and quads of words, e32std.h). `TRect` is (iTl.iX, iTl.iY, iBr.iX, iBr.iY).
use alloc::format;
use crate::loader::Process;
use crate::natives::NativeOutcome;

pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let this = p.cpu.regs[0];
    let rd = |p: &Process, a: u32| p.mem.vm.read_u32(a).map(|v| v as i32);
    // TInt TRect::Width() const / Height() const (e32std.h): iBr - iTl along the axis.
    if name == "_ZNK5TRect5WidthEv" || name == "_ZNK5TRect6HeightEv" {
        let axis = if name.ends_with("HeightEv") { 1 } else { 0 };
        let (Ok(tl), Ok(br)) = (rd(p, this + axis * 4), rd(p, this + 8 + axis * 4)) else { return Some(NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: the TRect at {this:#x} is unreadable"))) };
        p.cpu.regs[0] = br.wrapping_sub(tl) as u32;
        return Some(NativeOutcome::Return);
    }
    // TSize TRect::Size() const: a class return, so r0 is the result's address and r1 the TRect; (iBr.iX - iTl.iX, iBr.iY - iTl.iY).
    if name == "_ZNK5TRect4SizeEv" {
        let r1 = p.cpu.regs[1];
        let (Ok(a), Ok(b), Ok(c), Ok(d)) = (rd(p, r1), rd(p, r1 + 4), rd(p, r1 + 8), rd(p, r1 + 12)) else { return Some(NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: the TRect at {r1:#x} is unreadable"))) };
        let ok = p.mem.vm.write_u32(this, c.wrapping_sub(a) as u32).is_ok() && p.mem.vm.write_u32(this + 4, d.wrapping_sub(b) as u32).is_ok();
        return Some(if ok { NativeOutcome::Return } else { NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: the TSize at {this:#x} is unwritable")) });
    }
    let words: [i32; 4] = match name {
        // TRect::TRect(): zeroed.
        "_ZN5TRectC1Ev" => [0; 4],
        // TRect::TRect(const TSize& aSize): (0, 0) to the size (e32std.h).
        "_ZN5TRectC1ERK5TSize" => {
            let r1 = p.cpu.regs[1];
            let (Ok(w), Ok(h)) = (rd(p, r1), rd(p, r1 + 4)) else { return Some(NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: the TSize at {r1:#x} is unreadable"))) };
            [0, 0, w, h]
        }
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
