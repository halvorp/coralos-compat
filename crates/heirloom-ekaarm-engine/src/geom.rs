//! Geometry value types (`TPoint`, `TSize`, `TRect`: pairs and quads of words, e32std.h). `TRect` is (iTl.iX, iTl.iY, iBr.iX, iBr.iY).
use alloc::format;
use crate::loader::Process;
use crate::natives::NativeOutcome;

pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let this = p.cpu.regs[0];
    if name == "_ZN6TPoint5SetXYEii" || name == "_ZN5TSize7SetSizeEii" {
        let (a, b) = (p.cpu.regs[1], p.cpu.regs[2]);
        let ok = p.mem.vm.write_u32(this, a).is_ok() && p.mem.vm.write_u32(this.wrapping_add(4), b).is_ok();
        return Some(if ok { NativeOutcome::Return } else { NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: {this:#x} is unwritable")) });
    }
    let rd = |p: &Process, a: u32| p.mem.vm.read_u32(a).map(|v| v as i32);
    if name == "_ZN6TPointpLERKS_" || name == "_ZN6TPointmIERKS_" {
        let r1 = p.cpu.regs[1];
        let (Ok(ax), Ok(ay), Ok(bx), Ok(by)) = (rd(p, this), rd(p, this + 4), rd(p, r1), rd(p, r1 + 4)) else {
            return Some(NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: TPoint unreadable")));
        };
        let (nx, ny) = if name.contains("mIERKS_") { (ax.wrapping_sub(bx), ay.wrapping_sub(by)) } else { (ax.wrapping_add(bx), ay.wrapping_add(by)) };
        let ok = p.mem.vm.write_u32(this, nx as u32).is_ok() && p.mem.vm.write_u32(this + 4, ny as u32).is_ok();
        return Some(if ok { p.cpu.regs[0] = this; NativeOutcome::Return } else { NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: TPoint unwritable")) });
    }
    if name == "_ZNK6TPoint6AsSizeEv" || name == "_ZNK5TSize7AsPointEv" {
        let r1 = p.cpu.regs[1];
        let (Ok(x), Ok(y)) = (rd(p, r1), rd(p, r1 + 4)) else {
            return Some(NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: arg unreadable")));
        };
        let ok = p.mem.vm.write_u32(this, x as u32).is_ok() && p.mem.vm.write_u32(this + 4, y as u32).is_ok();
        return Some(if ok { NativeOutcome::Return } else { NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: sret unwritable")) });
    }
    if name == "_ZNK6TPointplERKS_" || name == "_ZNK6TPointmiERKS_" {
        // sret TPoint in r0, this in r1, rhs in r2
        let (r1, r2) = (p.cpu.regs[1], p.cpu.regs[2]);
        let (Ok(ax), Ok(ay), Ok(bx), Ok(by)) = (rd(p, r1), rd(p, r1 + 4), rd(p, r2), rd(p, r2 + 4)) else {
            return Some(NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: TPoint unreadable")));
        };
        let (nx, ny) = if name.contains("miERKS_") { (ax.wrapping_sub(bx), ay.wrapping_sub(by)) } else { (ax.wrapping_add(bx), ay.wrapping_add(by)) };
        let ok = p.mem.vm.write_u32(this, nx as u32).is_ok() && p.mem.vm.write_u32(this + 4, ny as u32).is_ok();
        return Some(if ok { NativeOutcome::Return } else { NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: sret unwritable")) });
    }
    if name == "_ZN5TRect4MoveERK6TPoint" {
        let r1 = p.cpu.regs[1];
        let (Ok(x0), Ok(y0), Ok(x1), Ok(y1), Ok(dx), Ok(dy)) = (rd(p, this), rd(p, this + 4), rd(p, this + 8), rd(p, this + 12), rd(p, r1), rd(p, r1 + 4)) else {
            return Some(NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: TRect/TPoint unreadable")));
        };
        for (i, v) in [x0.wrapping_add(dx), y0.wrapping_add(dy), x1.wrapping_add(dx), y1.wrapping_add(dy)].iter().enumerate() {
            let _ = p.mem.vm.write_u32(this + i as u32 * 4, *v as u32);
        }
        return Some(NativeOutcome::Return);
    }
    if name == "_ZNK5TSizeneERKS_" || name == "_ZNK5TSizeeqERKS_" {
        let r1 = p.cpu.regs[1];
        let (Ok(w0), Ok(h0), Ok(w1), Ok(h1)) = (rd(p, this), rd(p, this + 4), rd(p, r1), rd(p, r1 + 4)) else {
            return Some(NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: TSize unreadable")));
        };
        let eq = w0 == w1 && h0 == h1;
        p.cpu.regs[0] = u32::from(if name.contains("neERKS_") { !eq } else { eq });
        return Some(NativeOutcome::Return);
    }
    if name == "_ZNK6TPointeqERKS_" || name == "_ZNK6TPointneERKS_" {
        let r1 = p.cpu.regs[1];
        let (Ok(x0), Ok(y0), Ok(x1), Ok(y1)) = (rd(p, this), rd(p, this + 4), rd(p, r1), rd(p, r1 + 4)) else {
            return Some(NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: TPoint unreadable")));
        };
        let eq = x0 == x1 && y0 == y1;
        p.cpu.regs[0] = u32::from(if name.contains("neERKS_") { !eq } else { eq });
        return Some(NativeOutcome::Return);
    }
    if name == "_ZNK5TRectneERKS_" || name == "_ZNK5TRecteqERKS_" {
        let r1 = p.cpu.regs[1];
        let (Ok(a0), Ok(a1), Ok(a2), Ok(a3)) = (rd(p, this), rd(p, this + 4), rd(p, this + 8), rd(p, this + 12)) else {
            return Some(NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: TRect unreadable")));
        };
        let (Ok(b0), Ok(b1), Ok(b2), Ok(b3)) = (rd(p, r1), rd(p, r1 + 4), rd(p, r1 + 8), rd(p, r1 + 12)) else {
            return Some(NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: TRect arg unreadable")));
        };
        let eq = (a0, a1, a2, a3) == (b0, b1, b2, b3);
        p.cpu.regs[0] = u32::from(if name.contains("neERKS_") { !eq } else { eq });
        return Some(NativeOutcome::Return);
    }
    if name == "_ZN5TRect7SetSizeERK5TSize" {
        let r1 = p.cpu.regs[1];
        let (Ok(x), Ok(y), Ok(w), Ok(h)) = (rd(p, this), rd(p, this + 4), rd(p, r1), rd(p, r1 + 4)) else {
            return Some(NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: TRect/TSize unreadable")));
        };
        let ok = p.mem.vm.write_u32(this + 8, x.wrapping_add(w) as u32).is_ok()
            && p.mem.vm.write_u32(this + 12, y.wrapping_add(h) as u32).is_ok();
        return Some(if ok { NativeOutcome::Return } else { NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: TRect unwritable")) });
    }
    if name == "_ZNK5TRect7IsEmptyEv" {
        let (Ok(x0), Ok(y0), Ok(x1), Ok(y1)) = (rd(p, this), rd(p, this + 4), rd(p, this + 8), rd(p, this + 12)) else {
            return Some(NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: TRect unreadable")));
        };
        p.cpu.regs[0] = u32::from(x0 >= x1 || y0 >= y1);
        return Some(NativeOutcome::Return);
    }
    if name == "_ZN5TRect6ShrinkEii" || name == "_ZN5TRect4GrowEii" || name == "_ZN5TRect4MoveEii" {
        let (dx, dy) = (p.cpu.regs[1] as i32, p.cpu.regs[2] as i32);
        let (Ok(x0), Ok(y0), Ok(x1), Ok(y1)) = (rd(p, this), rd(p, this + 4), rd(p, this + 8), rd(p, this + 12)) else {
            return Some(NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: TRect unreadable")));
        };
        let out = if name.contains("Shrink") {
            [x0 + dx, y0 + dy, x1 - dx, y1 - dy]
        } else if name.contains("Grow") {
            [x0 - dx, y0 - dy, x1 + dx, y1 + dy]
        } else {
            [x0 + dx, y0 + dy, x1 + dx, y1 + dy]
        };
        for (i, v) in out.iter().enumerate() {
            let _ = p.mem.vm.write_u32(this + i as u32 * 4, *v as u32);
        }
        return Some(NativeOutcome::Return);
    }
    if name == "_ZN5TRect12IntersectionERKS_" {
        let r1 = p.cpu.regs[1];
        let (Ok(ax0), Ok(ay0), Ok(ax1), Ok(ay1)) = (rd(p, this), rd(p, this + 4), rd(p, this + 8), rd(p, this + 12)) else {
            return Some(NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: TRect unreadable")));
        };
        let (Ok(bx0), Ok(by0), Ok(bx1), Ok(by1)) = (rd(p, r1), rd(p, r1 + 4), rd(p, r1 + 8), rd(p, r1 + 12)) else {
            return Some(NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: TRect arg unreadable")));
        };
        let (ix0, iy0, ix1, iy1) = (ax0.max(bx0), ay0.max(by0), ax1.min(bx1), ay1.min(by1));
        let out = if ix0 < ix1 && iy0 < iy1 { [ix0, iy0, ix1, iy1] } else { [0, 0, 0, 0] };
        for (i, v) in out.iter().enumerate() {
            let _ = p.mem.vm.write_u32(this + i as u32 * 4, *v as u32);
        }
        return Some(NativeOutcome::Return);
    }
    if name == "_ZN5TRect12BoundingRectERKS_" {
        let r1 = p.cpu.regs[1];
        let (Ok(ax0), Ok(ay0), Ok(ax1), Ok(ay1)) = (rd(p, this), rd(p, this + 4), rd(p, this + 8), rd(p, this + 12)) else {
            return Some(NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: TRect unreadable")));
        };
        let (Ok(bx0), Ok(by0), Ok(bx1), Ok(by1)) = (rd(p, r1), rd(p, r1 + 4), rd(p, r1 + 8), rd(p, r1 + 12)) else {
            return Some(NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: TRect arg unreadable")));
        };
        let out = if ax0 >= ax1 || ay0 >= ay1 {
            [bx0, by0, bx1, by1]
        } else if bx0 >= bx1 || by0 >= by1 {
            [ax0, ay0, ax1, ay1]
        } else {
            [ax0.min(bx0), ay0.min(by0), ax1.max(bx1), ay1.max(by1)]
        };
        for (i, v) in out.iter().enumerate() {
            let _ = p.mem.vm.write_u32(this + i as u32 * 4, *v as u32);
        }
        return Some(NativeOutcome::Return);
    }
    // TInt TRect::Width() const / Height() const (e32std.h): iBr - iTl along the axis.
    if name == "_ZNK5TRect5WidthEv" || name == "_ZNK5TRect6HeightEv" {
        let axis = if name.ends_with("HeightEv") { 1 } else { 0 };
        let (Ok(tl), Ok(br)) = (rd(p, this + axis * 4), rd(p, this + 8 + axis * 4)) else { return Some(NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: the TRect at {this:#x} is unreadable"))) };
        p.cpu.regs[0] = br.wrapping_sub(tl) as u32;
        return Some(NativeOutcome::Return);
    }
    // TRect TRegion::BoundingRect() const: sret TRect in r0, this in r1. Return the full screen rect.
    if name == "_ZNK7TRegion12BoundingRectEv" {
        let (sw, sh) = p.screen_size();
        for (i, v) in [0i32, 0, sw, sh].iter().enumerate() {
            if p.mem.vm.write_u32(this.wrapping_add(i as u32 * 4), *v as u32).is_err() {
                return Some(NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: TRect at {this:#x} unwritable")));
            }
        }
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
        // TRect::TRect(TInt aAx, TInt aAy, TInt aBx, TInt aBy) / TRect::SetRect(TInt, TInt, TInt, TInt): r1, r2, r3, [sp]
        "_ZN5TRectC1Eiiii" | "_ZN5TRectC2Eiiii" | "_ZN5TRect7SetRectEiiii" => {
            let sp = p.cpu.regs[13];
            let Ok(d) = rd(p, sp) else { return Some(NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: stack arg unreadable"))) };
            [p.cpu.regs[1] as i32, p.cpu.regs[2] as i32, p.cpu.regs[3] as i32, d]
        }
        // TRect::TRect(const TSize& aSize): (0, 0) to the size (e32std.h).
        "_ZN5TRectC1ERK5TSize" => {
            let r1 = p.cpu.regs[1];
            let (Ok(w), Ok(h)) = (rd(p, r1), rd(p, r1 + 4)) else { return Some(NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: the TSize at {r1:#x} is unreadable"))) };
            [0, 0, w, h]
        }
        // TRect::TRect(const TPoint& aTl, const TPoint& aBr) / (const TPoint&, const TSize&): corners, or corner and extent.
        "_ZN5TRectC1ERK6TPointS2_" | "_ZN5TRectC1ERK6TPointRK5TSize" | "_ZN5TRect7SetRectERK6TPointRK5TSize" | "_ZN5TRect7SetRectERK6TPointS2_" => {
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
