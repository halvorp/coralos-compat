//! Special-purpose registers, condition-register moves and the time base (PEM 4.2.? "Processor Control").
//! User mode reaches XER (1), LR (8), CTR (9) and the time base (268/269 via mftb or mfspr); every other SPR is a
//! named trap (SPR numbers with bit 0x10 set are supervisor-only on the 6xx).
use crate::cpu::{Cpu, XER_CA, XER_MASK, XER_OV, XER_SO};
use crate::exec::R;
use crate::fields::W;

const SPR_XER: u32 = 1;
const SPR_LR: u32 = 8;
const SPR_CTR: u32 = 9;
const TBL: u32 = 268;
const TBU: u32 = 269;

fn spr_name(spr: u32) -> &'static str {
    match spr {
        0 => "SPR 0 (601 MQ)", 4 | 5 | 6 => "SPR 4-6 (601 RTC)", 22 => "SPR 22 (DEC)",
        256 => "SPR 256 (VRSAVE)", 936..=939 | 952..=959 => "SPR (performance monitor)",
        _ => "SPR outside the user subset",
    }
}

pub(crate) fn x31(cpu: &mut Cpu, w: W, xo: u32) -> Option<R> {
    let t = w.rt();
    match xo {
        19 => cpu.gpr[t] = cpu.cr, // mfcr
        144 => { // mtcrf FXM (bits 12-19): field 0 = bit 0x80
            let fxm = w.0 >> 12 & 0xFF;
            let mut m = 0u32;
            for i in 0..8 { if fxm & (0x80 >> i) != 0 { m |= 0xF << (28 - 4 * i); } }
            cpu.cr = (cpu.cr & !m) | (cpu.gpr[t] & m);
        }
        512 => { // mcrxr: XER[0:3] -> CR field, cleared
            let v = cpu.xer >> 28;
            cpu.set_cr_field(w.bf(), v);
            cpu.xer &= !(XER_SO | XER_OV | XER_CA);
        }
        339 => { // mfspr
            let spr = w.spr();
            cpu.gpr[t] = match spr {
                SPR_XER => cpu.xer, SPR_LR => cpu.lr, SPR_CTR => cpu.ctr,
                TBL => cpu.tb as u32, TBU => (cpu.tb >> 32) as u32,
                s if s & 0x10 != 0 => return Some(Err(cpu.privileged(w, "mfspr of a supervisor SPR"))),
                s => return Some(Err(cpu.unimpl(w, spr_name(s)))),
            };
        }
        371 => { // mftb: TBR 268 = TBL, 269 = TBU
            cpu.gpr[t] = match w.spr() {
                TBL => cpu.tb as u32, TBU => (cpu.tb >> 32) as u32,
                _ => return Some(Err(cpu.invalid(w, "mftb with a TBR other than 268/269"))),
            };
        }
        467 => { // mtspr
            let v = cpu.gpr[t];
            match w.spr() {
                SPR_XER => cpu.xer = v & XER_MASK, SPR_LR => cpu.lr = v, SPR_CTR => cpu.ctr = v,
                s if s & 0x10 != 0 => return Some(Err(cpu.privileged(w, "mtspr of a supervisor SPR"))),
                s => return Some(Err(cpu.unimpl(w, spr_name(s)))),
            }
        }
        83 => return Some(Err(cpu.privileged(w, "mfmsr"))),
        146 => return Some(Err(cpu.privileged(w, "mtmsr"))),
        _ => return None,
    }
    Some(Ok(()))
}
