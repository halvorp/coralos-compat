//! Font family names and numbers: `GetFNum(name, VAR familyID)` and `GetFontName(familyID, VAR name)` (Inside Macintosh I,
//! "The Font Manager"). A family is found first among the `FOND` resources of the open files (the resource's ID is the family
//! number, its name the family name; names compare ignoring case), then in the System file's standard families, listed here
//! from the Font Manager chapter: Chicago 0 (the system font), New York 2, Geneva 3, Monaco 4, Venice 5, London 6, Athens 7, San
//! Francisco 8, Toronto 9, Cairo 11, Los Angeles 12, Times 20, Helvetica 21, Courier 22, Symbol 23, Mobile 24. `applFont` (1) is
//! the application font, whose name is Geneva on a US system. An unknown name answers family 0 (the book: "the system font
//! number"); an unknown number answers the empty string.
use crate::machine::{Call, Done, Mac};
use crate::pascal::{read_pstring_bytes, write_pstring, Arg};
use crate::Refusal;
use alloc::format;
use alloc::vec::Vec;
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

/// The standard families (family number, name).
pub const SYSTEM_FAMILIES: [(i16, &str); 16] = [
    (0, "Chicago"), (1, "Geneva"), (2, "New York"), (3, "Geneva"), (4, "Monaco"), (5, "Venice"), (6, "London"), (7, "Athens"),
    (8, "San Francisco"), (9, "Toronto"), (11, "Cairo"), (12, "Los Angeles"), (20, "Times"), (21, "Helvetica"), (22, "Courier"), (23, "Symbol"),
];
/// The families a Font menu lists on a stock System 7 (Chicago, the system font, included).
pub const MENU_FAMILIES: [&str; 15] = ["Athens", "Cairo", "Chicago", "Courier", "Geneva", "Helvetica", "London", "Los Angeles", "Monaco", "New York", "San Francisco", "Symbol", "Times", "Toronto", "Venice"];
const MOBILE: (i16, &str) = (24, "Mobile");

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    m.register(0xA900, &[Arg::P, Arg::P], None, get_fnum::<U>)?;
    m.register(0xA8FF, &[Arg::W, Arg::P], None, get_font_name::<U>)?;
    m.register(0xA834, &[Arg::B], None, set_fscale_disable::<U>)?;
    m.register(0xA814, &[Arg::B], None, set_fract_enable::<U>)
}

/// `SetFScaleDisable(fscaleDisable)`: whether the Font Manager may scale a font that lacks the requested size. Recorded in
/// `FScaleDisable` ($A63); this engine's text is drawn in one bitmap face at any size, so the flag changes nothing it draws.
fn set_fscale_disable<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let on = c.args.get(0) as u8 != 0;
    m.mem.write_u8(0xA63, on as u8).map_err(Refusal::mem("FScaleDisable"))?;
    Ok(Done { result: 0, log: format!("[font] SetFScaleDisable {on} (recorded; text is not scaled from outlines here)") })
}

/// `SetFractEnable(fractEnable)`: fractional glyph widths; recorded in `FractEnable` ($BF4), not used by the bitmap text.
fn set_fract_enable<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let on = c.args.get(0) as u8 != 0;
    m.mem.write_u8(0xBF4, on as u8).map_err(Refusal::mem("FractEnable"))?;
    Ok(Done { result: 0, log: format!("[font] SetFractEnable {on} (recorded; glyph widths are whole pixels here)") })
}

/// The family number for `name` in the standard table (case-insensitive); `applFont` is a number, not a name, so 1 is skipped.
pub fn system_family(name: &[u8]) -> Option<i16> {
    SYSTEM_FAMILIES.iter().chain(core::iter::once(&MOBILE)).find(|(n, f)| *n != 1 && f.as_bytes().eq_ignore_ascii_case(name)).map(|(n, _)| *n)
}

fn get_fnum<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (name, out) = (read_pstring_bytes(&m.mem, c.args.get(0))?, c.args.get(1));
    let own = m.get_named_resource(*b"FOND", &name, false).ok().and_then(|h| m.res_info(h)).map(|(_, id, _)| id);
    let (id, from) = match (own, system_family(&name)) { (Some(i), _) => (i, "FOND"), (None, Some(i)) => (i, "system"), (None, None) => (0, "not found") };
    m.mem.write_u16(out, id as u16).map_err(Refusal::mem("GetFNum result"))?;
    Ok(Done { result: 0, log: format!("[font] GetFNum \"{}\" -> {id} ({from})", name.iter().map(|&b| char::from(b)).collect::<alloc::string::String>()) })
}

fn get_font_name<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (id, out) = (c.args.get(0) as u16 as i16, c.args.get(1));
    let own = m.get_resource(*b"FOND", id, false).ok().and_then(|h| m.res_info(h)).and_then(|(_, _, n)| n);
    let name: Vec<u8> = own.or_else(|| SYSTEM_FAMILIES.iter().chain(core::iter::once(&MOBILE)).find(|(n, _)| *n == id).map(|(_, s)| s.as_bytes().to_vec())).unwrap_or_default();
    write_pstring(&mut m.mem, out, &name)?;
    Ok(Done { result: 0, log: format!("[font] GetFontName {id} -> \"{}\"", name.iter().map(|&b| char::from(b)).collect::<alloc::string::String>()) })
}
