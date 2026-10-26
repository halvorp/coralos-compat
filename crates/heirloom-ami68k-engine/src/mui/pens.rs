//! `MUI_ObtainPen` / `MUI_ReleasePen`: a `struct MUI_PenSpec` (the 32-byte string a Poppen object holds) becomes a pen of the screen.
//! Facts: Zune's mui_obtainpen.c, mui_releasepen.c and penspec.c (APL; read for behaviour, nothing copied): the spec is `m<n>` (a MUI pen,
//! `mri_Pens[n]`), `p<n>` (a colormap entry, negative counts from the end), `s<n>` (a DrawInfo pen) or `r<red>,<green>,<blue>` (32-bit hex
//! components; also accepted without the `r`). An RGB spec is `ObtainBestPenA` on the screen's palette (the Gute Form palette through the existing
//! pen allocator, [`crate::graphics::palette`]) with OBP_FailIfBad FALSE, else `FindColor`; the result of an allocated pen has bit 16 set so
//! `MUI_ReleasePen` knows to give it back. Anything unparsable or out of range is -1 (Zune returns -1 too); the log says which.
use alloc::format;
use alloc::string::String;
use crate::graphics::layout::{DRI_NUM_PENS, DRI_PENS};
use crate::graphics::palette;
use crate::{machine::Amiga, Refusal};
use heirloom_ui_abi::GuestUi;
use heirloom_cpu_68k::Cpu;

const MRI_DRAW_INFO: u32 = 8;
const MRI_PENS: u32 = 12;
const MRI_COLORMAP: u32 = 28;
const CM_COUNT: u32 = 2;
const MPEN_COUNT: i32 = 8;
const PEN_ALLOCATED: u32 = 0x10000;
const PEN_MASK: u32 = 0xFFFF;

enum Spec { Mui(i32), Cmap(i32), Sys(i32), Rgb(u32, u32, u32) }

/// dos `StrToLong`: optional sign, decimal digits; None when there are none.
fn str_to_long(s: &str) -> Option<i32> {
    let t = s.trim_start_matches([' ', '\t']);
    let (neg, d) = match t.strip_prefix('-') { Some(r) => (true, r), None => (false, t.strip_prefix('+').unwrap_or(t)) };
    let n = d.bytes().take_while(u8::is_ascii_digit).count();
    if n == 0 { return None; }
    let v = d[..n].parse::<i64>().ok()?;
    Some(if neg { -v } else { v } as i32)
}

/// C `strtoul(s, &end, 16)`: (value, rest); None when no digit was read. A leading `0x` is skipped like strtoul does.
fn strtoul16(s: &str) -> Option<(u32, &str)> {
    let t = s.trim_start();
    let t = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).unwrap_or(t);
    let n = t.bytes().take_while(u8::is_ascii_hexdigit).count();
    if n == 0 { return None; }
    Some((u32::from_str_radix(&t[..n], 16).unwrap_or(u32::MAX), &t[n..]))
}

fn parse(spec: &str) -> Option<Spec> {
    match spec.as_bytes().first()? {
        b'm' => str_to_long(&spec[1..]).map(Spec::Mui),
        b'p' => str_to_long(&spec[1..]).map(Spec::Cmap),
        b's' => str_to_long(&spec[1..]).map(Spec::Sys),
        _ => {
            let s = spec.strip_prefix('r').unwrap_or(spec);
            let (r, rest) = strtoul16(s)?;
            let (g, rest) = strtoul16(rest.get(1..)?)?;
            let (b, _) = strtoul16(rest.get(1..)?)?;
            Some(Spec::Rgb(r, g, b))
        }
    }
}

/// The pen `spec` names on the render info `mri`: (pen, allocated), or why not.
fn setup<U: GuestUi>(a: &mut Amiga<U>, mri: u32, spec: Spec) -> Result<Option<(u32, bool)>, Refusal> {
    let m = Refusal::mem("MUI_RenderInfo");
    let cm = a.mem.read_u32(mri + MRI_COLORMAP).map_err(m)?;
    if cm == 0 { return Ok(None); }
    Ok(match spec {
        Spec::Mui(n) if (0..MPEN_COUNT).contains(&n) => {
            let pens = a.mem.read_u32(mri + MRI_PENS).map_err(Refusal::mem("mri_Pens"))?;
            Some((a.mem.read_u16(pens + 2 * n as u32).map_err(Refusal::mem("mri_Pens[]"))? as u32, false))
        }
        Spec::Cmap(n) => {
            let count = a.mem.read_u16(cm + CM_COUNT).map_err(Refusal::mem("ColorMap.Count"))? as i32;
            let pen = if n >= 0 { n } else { count + n };
            (0..count).contains(&pen).then_some((pen as u32, false))
        }
        Spec::Sys(n) => {
            let dri = a.mem.read_u32(mri + MRI_DRAW_INFO).map_err(Refusal::mem("mri_DrawInfo"))?;
            let num = a.mem.read_u16(dri + DRI_NUM_PENS).map_err(Refusal::mem("DrawInfo.NumPens"))? as i32;
            if !(0..num).contains(&n) { return Ok(None); }
            let pens = a.mem.read_u32(dri + DRI_PENS).map_err(Refusal::mem("dri_Pens"))?;
            Some((a.mem.read_u16(pens + 2 * n as u32).map_err(Refusal::mem("dri_Pens[]"))? as u32, false))
        }
        Spec::Rgb(r, g, b) => {
            let (r, g, b) = (r >> 24, g >> 24, b >> 24);
            match palette::obtain_best(a, r, g, b, false)? {
                -1 => Some((palette::closest(a, r, g, b, -1) as u32, false)),
                pen => Some((pen as u32, true)),
            }
        }
        Spec::Mui(_) => None,
    })
}

/// MUI_ObtainPen(a0 = mri, a1 = struct MUI_PenSpec, d0 = flags) -> the pen (bit 16 set when it must be released), or -1.
pub fn obtain_pen<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (mri, at) = (cpu.areg(0), cpu.areg(1));
    let text = if at == 0 || mri == 0 { None } else { Some(a.mem.read_cstr(at).map_err(Refusal::mem("MUI_PenSpec"))?) };
    let res = match text.as_deref().and_then(parse) {
        Some(spec) => setup(a, mri, spec)?.map(|(p, alloc)| p | if alloc { PEN_ALLOCATED } else { 0 }),
        None => None,
    };
    cpu.set_d0(res.unwrap_or(u32::MAX));
    Ok(format!("[muimaster] MUI_ObtainPen mri={mri:#x} spec={at:#x} {:?} -> {}", text.unwrap_or_default(), res.map_or(-1, |p| p as i64)))
}

/// MUI_ReleasePen(a0 = mri, d0 = pen from MUI_ObtainPen): only a pen with bit 16 set was allocated.
pub fn release_pen<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (mri, pen) = (cpu.areg(0), cpu.dreg(0));
    let cm = if mri == 0 { 0 } else { a.mem.read_u32(mri + MRI_COLORMAP).map_err(Refusal::mem("mri_Colormap"))? };
    if pen != u32::MAX && cm != 0 && pen & !PEN_MASK == PEN_ALLOCATED { palette::release(a, (pen & PEN_MASK) as usize); }
    Ok(format!("[muimaster] MUI_ReleasePen {pen:#x}"))
}

