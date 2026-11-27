//! AmigaOS V39 `gadgets/colorwheel.gadget` library fallback (`ConvertHSBToRGB` LVO -30,
//! `ConvertRGBToHSB` LVO -36).
//!
//! Facts: AROS `workbench/classes/gadgets/colorwheel/{colorwheel.conf,converthsbtorgb.c,convertrgbtohsb.c}`
//! (FIXED_MATH 16.16 fixed-point variant; `#![no_std]`, zero floating-point dependency) and
//! `gadgets/colorwheel.h` (`struct ColorWheelHSB { ULONG cw_Hue, cw_Saturation, cw_Brightness; }`,
//! `struct ColorWheelRGB { ULONG cw_Red, cw_Green, cw_Blue; }`).
//!
//! When a real `SYS:Classes/Gadgets/colorwheel.gadget` exists on disk, `disklib::try_open` loads
//! it first. When a minimal `SYS:` volume omits `Classes/Gadgets/`, opening this built-in V39
//! library lets Zune's `Coloradjust.mui` (`coloradjust.c:370`) convert colours cleanly instead of
//! jumping through a NULL `ColorWheelBase`.
use alloc::format;
use alloc::string::String;
use crate::cpu::Cpu;
use crate::libtable::LibraryBuilder;
use crate::{machine::Amiga, Refusal};
use heirloom_ui_abi::GuestUi;

/// Convert 32-bit left-justified RGB (`0..=0xFFFF_FFFF` per gun) to 32-bit HSB, matching AROS
/// `convertrgbtohsb.c` (`FIXED_MATH` 16-bit components replicated into high & low 16 bits).
pub fn rgb_to_hsb(r32: u32, g32: u32, b32: u32) -> (u32, u32, u32) {
    let (r, g, b) = ((r32 >> 16) as i64, (g32 >> 16) as i64, (b32 >> 16) as i64);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let i = max as u32;
    let s = if max != 0 {
        (((max - min) << 16) / max).min(0xFFFF) as u32
    } else {
        0
    };
    let h = if s == 0 {
        0u32
    } else {
        let delta = max - min;
        let mut hue = if r == max {
            ((g - b) << 16) / delta
        } else if g == max {
            (2 << 16) + (((b - r) << 16) / delta)
        } else {
            (4 << 16) + (((r - g) << 16) / delta)
        };
        hue *= 60;
        if hue < 0 {
            hue += 360 << 16;
        }
        (hue / 360).clamp(0, 0xFFFF) as u32
    };
    (h | (h << 16), s | (s << 16), i | (i << 16))
}

/// Convert 32-bit left-justified HSB to 32-bit RGB, matching AROS `converthsbtorgb.c` (`FIXED_MATH`).
pub fn hsb_to_rgb(h32: u32, s32: u32, b32: u32) -> (u32, u32, u32) {
    let mut h = h32 >> 16;
    let mut s = s32 >> 16;
    let i = b32 >> 16;
    if s == 0 {
        return (i | (i << 16), i | (i << 16), i | (i << 16));
    }
    if h == 0xFFFF {
        h = 0;
    }
    let h6 = h * 6;
    let sector = (h6 >> 16) % 6;
    let f = h6 & 0xFFFF;
    if s > 0xFFFF {
        s = 0xFFFF;
    }
    let w = (i * (0xFFFF - s)) >> 16;
    let q = (i * (0xFFFF - ((s * f) >> 16))) >> 16;
    let t = (i * (0xFFFF - ((s * (0xFFFF - f)) >> 16))) >> 16;
    let (r, g, b) = match sector {
        0 => (i, t, w),
        1 => (q, i, w),
        2 => (w, i, t),
        3 => (w, q, i),
        4 => (t, w, i),
        _ => (i, w, q),
    };
    (r | (r << 16), g | (g << 16), b | (b << 16))
}

fn convert_hsb_to_rgb<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (hsb_p, rgb_p) = (cpu.areg(0), cpu.areg(1));
    if hsb_p == 0 || rgb_p == 0 {
        return Err(Refusal(format!("ConvertHSBToRGB: NULL pointer (hsb={hsb_p:#x}, rgb={rgb_p:#x})")));
    }
    let h = a.mem.read_u32(hsb_p).map_err(Refusal::mem("ColorWheelHSB.cw_Hue"))?;
    let s = a.mem.read_u32(hsb_p + 4).map_err(Refusal::mem("ColorWheelHSB.cw_Saturation"))?;
    let b = a.mem.read_u32(hsb_p + 8).map_err(Refusal::mem("ColorWheelHSB.cw_Brightness"))?;
    let (r, g, bl) = hsb_to_rgb(h, s, b);
    a.mem.write_u32(rgb_p, r).map_err(Refusal::mem("ColorWheelRGB.cw_Red"))?;
    a.mem.write_u32(rgb_p + 4, g).map_err(Refusal::mem("ColorWheelRGB.cw_Green"))?;
    a.mem.write_u32(rgb_p + 8, bl).map_err(Refusal::mem("ColorWheelRGB.cw_Blue"))?;
    Ok(format!("[colorwheel] ConvertHSBToRGB ({h:#x},{s:#x},{b:#x}) -> ({r:#x},{g:#x},{bl:#x})"))
}

fn convert_rgb_to_hsb<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (rgb_p, hsb_p) = (cpu.areg(0), cpu.areg(1));
    if rgb_p == 0 || hsb_p == 0 {
        return Err(Refusal(format!("ConvertRGBToHSB: NULL pointer (rgb={rgb_p:#x}, hsb={hsb_p:#x})")));
    }
    let r = a.mem.read_u32(rgb_p).map_err(Refusal::mem("ColorWheelRGB.cw_Red"))?;
    let g = a.mem.read_u32(rgb_p + 4).map_err(Refusal::mem("ColorWheelRGB.cw_Green"))?;
    let b = a.mem.read_u32(rgb_p + 8).map_err(Refusal::mem("ColorWheelRGB.cw_Blue"))?;
    let (h, s, br) = rgb_to_hsb(r, g, b);
    a.mem.write_u32(hsb_p, h).map_err(Refusal::mem("ColorWheelHSB.cw_Hue"))?;
    a.mem.write_u32(hsb_p + 4, s).map_err(Refusal::mem("ColorWheelHSB.cw_Saturation"))?;
    a.mem.write_u32(hsb_p + 8, br).map_err(Refusal::mem("ColorWheelHSB.cw_Brightness"))?;
    Ok(format!("[colorwheel] ConvertRGBToHSB ({r:#x},{g:#x},{b:#x}) -> ({h:#x},{s:#x},{br:#x})"))
}

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.version(44)
        .named("ConvertHSBToRGB", convert_hsb_to_rgb::<U>)
        .named("ConvertRGBToHSB", convert_rgb_to_hsb::<U>)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_hsb_round_trips_primaries_and_greys() {
        for (r, g, b) in [
            (0xFFFF_FFFF, 0, 0),
            (0, 0xFFFF_FFFF, 0),
            (0, 0, 0xFFFF_FFFF),
            (0xFFFF_FFFF, 0xFFFF_FFFF, 0xFFFF_FFFF),
            (0, 0, 0),
            (0x8080_8080, 0x8080_8080, 0x8080_8080),
            (0xCCCC_CCCC, 0x6666_6666, 0x3333_3333),
        ] {
            let (h, s, v) = rgb_to_hsb(r, g, b);
            let (r2, g2, b2) = hsb_to_rgb(h, s, v);
            assert!((r >> 24).abs_diff(r2 >> 24) <= 1, "R {r:#x} -> {r2:#x}");
            assert!((g >> 24).abs_diff(g2 >> 24) <= 1, "G {g:#x} -> {g2:#x}");
            assert!((b >> 24).abs_diff(b2 >> 24) <= 1, "B {b:#x} -> {b2:#x}");
        }
    }
}
