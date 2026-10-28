//! The Script Manager's `_ScriptUtil` ($A8B5), for a Mac with the Roman script only (Inside Macintosh: Text, "Script Manager").
//!
//! The selector is a LONG pushed last. Its encoding (read off MPW's inline glue as systemless documents it, and the SimpleText call
//! `move.l #$84020008,-(sp)`): byte 3 = $80 | the result size in bytes, byte 2 = the size of the Pascal arguments in bytes, the low
//! word = the routine's number, which counts 0, 2, 4 ... in the order `FontScript, IntlScript, KeyScript, FontToScript,
//! GetScriptManagerVariable, SetScriptManagerVariable, GetScriptVariable, SetScriptVariable, CharByte, CharType, Pixel2Char,
//! Char2Pixel, Transliterate, FindWord, HiliteText, DrawJust, MeasureJust, ParseTable` (Executor `trapinfo` lists that order; the
//! values $800E001C HiliteText and $82040022 ParseTable are confirmed by systemless' generated route table). So the frame of EVERY
//! selector is known from the selector itself: the callee pops selector + arguments and leaves the result above them.
//!
//! Implemented: `FontScript`, `IntlScript`, `FontToScript` (all answer smRoman, 0), `KeyScript` (accepted: there is one keyboard
//! script), `GetScriptManagerVariable` and `GetScriptVariable` for the variables that have an honest Roman answer (below), `CharByte` (every byte is a single
//! byte), `VisibleLength` ($84080028: trailing blanks and tabs excluded). Every other selector pops its frame and is refused BY NAME,
//! the result slot untouched. NAMED LIMIT: Roman only; `smEnabled` is 1.
use crate::machine::{Mac, Regs};
use crate::Refusal;
use alloc::format;
use alloc::string::String;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

pub const VISIBLE_LENGTH: u32 = 0x8408_0028;
const NAMES: [&str; 18] = ["FontScript", "IntlScript", "KeyScript", "FontToScript", "GetScriptManagerVariable", "SetScriptManagerVariable", "GetScriptVariable",
    "SetScriptVariable", "CharByte", "CharType", "Pixel2Char", "Char2Pixel", "Transliterate", "FindWord", "HiliteText", "DrawJust", "MeasureJust", "ParseTable"];

/// The Roman answer for `GetScriptManagerVariable(selector)`, or None when there is no honest one.
pub fn manager_variable(sel: i16) -> Option<(u32, &'static str)> {
    Some(match sel {
        0 => (0x0007, "smVersion"),            // Script Manager 7
        4 => (1, "smEnabled"),                 // one script system: Roman
        6 => (0, "smBidirect"),                // no right-to-left script is installed
        8 | 10 | 12 | 14 | 16 | 18 | 20 | 22 => (0, "a script code (smRoman)"), // smFontForce .. smKeyScript: all Roman
        // the rest of the Roman-only table (spec: macos9-specs/InterfaceLib/GetScriptManagerVariable): smMunged, smSysRef .. smKeyDisableState are 0
        2 | 24 | 26 | 28 | 30 | 32 | 34 | 36 | 38 | 42 => (0, "a Roman-only zero"),
        // smRegionCode: 2 = verBritain -- CoralOS's default region (Peter: European first, British default), ruled by the coordinator 2026-10-24; the 9.2.1 System file itself holds 0 (verUS)
        40 => (2, "smRegionCode (verBritain)"),
        _ => return None,
    })
}

/// The Roman answer for `GetScriptVariable(script, selector)`, or None when there is no honest one. Only the two selectors the Stage-A
/// specs ground are answered; the Script Manager's selector numbering is NOT taken from memory (Executor's header and systemless
/// disagree with each other), so every other selector is refused by name with its number.
/// * 6: the CoreFoundation script-flag test `GetScriptVariable(script, 6) == 0` means "not a double-byte script"
///   (spec: macos9-specs/CoreFoundationLib/CFStringGetBytes.md, step 1): 0 for Roman.
/// * 90 and 92: `smScriptValidStyles` and `smScriptAliasStyle`, the last two bytes of the script's `itlb` bundle. The Mac OS 9.2.1
///   System file's `itlb` 0 "Roman" (50 bytes, from the owner's install CD, docs/consults/2026-10-24-macos921-oracle.md) ends `7F 02`:
///   every QuickDraw style but none beyond (bold, italic, underline, outline, shadow, condense, extend = 0x7F) and italic (2) as the
///   alias style. The selector numbers are the Script.h ones (valid styles follows the three font-and-size records at 84, 86, 88);
///   SimpleText asks 90 to enable its Style menu.
/// * 0x54: the script's system font record, `fondID << 16 | size`, which `UseThemeFont` reads (spec:
///   macos9-specs/AppearanceLib/ThemeBrushes.md, UseThemeFont): Chicago (family 0) at 12 points, the classic Roman system font.
pub fn script_variable(sel: i16) -> Option<(u32, &'static str)> {
    Some(match sel {
        6 => (0, "not a double-byte script"),
        90 => (0x7F, "smScriptValidStyles: itlb 0 itlbValidStyles"),
        92 => (2, "smScriptAliasStyle: itlb 0 itlbAliasStyle, italic"),
        0x54 => (12, "the system font record: Chicago (family 0), 12 points"),
        _ => return None,
    })
}

fn script_util<U: GuestUi>(m: &mut Mac<U>, cpu: &mut Cpu, _: &mut Regs) -> Result<String, Refusal> {
    let sp = cpu.areg(7);
    let sel = m.mem.read_u32(sp).map_err(Refusal::mem("ScriptUtil selector"))?;
    if sel & 0x8000_0000 == 0 { return Err(Refusal(format!("_ScriptUtil selector {sel:#010x}: not a stack-long selector (bit 31 clear); its frame is unknown and the stack was left as the program made it"))); }
    let (result, params, routine) = ((sel >> 24) & 0x0F, (sel >> 16) & 0xFF, (sel & 0xFFFF) as u16 as i16);
    let at = sp + 4; // the arguments start above the selector, the LAST one first
    let out = sp + 4 + params;
    let done = |cpu: &mut Cpu| cpu.set_areg(7, out);
    let put = |m: &mut Mac<U>, v: u32| -> Result<(), Refusal> {
        match result { 2 => m.mem.write_u16(out, v as u16), 4 => m.mem.write_u32(out, v), _ => Ok(()) }.map_err(Refusal::mem("ScriptUtil result"))
    };
    let name = if routine >= 0 && routine % 2 == 0 && (routine as usize / 2) < NAMES.len() { NAMES[routine as usize / 2] } else { "an unnamed routine" };
    let log = match (sel, routine) {
        (VISIBLE_LENGTH, _) => {
            let (len, ptr) = (m.mem.read_u32(at).map_err(Refusal::mem("VisibleLength"))? as i32, m.mem.read_u32(at + 4).map_err(Refusal::mem("VisibleLength"))?);
            let n = len.max(0) as u32;
            let bytes = if n > 0 { m.mem.bytes(m.mem.span(ptr, n).map_err(Refusal::mem("VisibleLength text"))?).map_err(Refusal::mem("VisibleLength text"))?.to_vec() } else { alloc::vec::Vec::new() };
            let visible = bytes.iter().rposition(|&b| b != b' ' && b != b'\t').map_or(0, |i| i + 1) as u32;
            put(m, visible)?;
            format!("VisibleLength {n} -> {visible}")
        }
        (_, 0) | (_, 2) | (_, 6) if result == 2 => { put(m, 0)?; format!("{name} -> smRoman") }
        (_, 4) => format!("KeyScript accepted (one keyboard script: Roman)"),
        (_, 8) if result == 4 && params == 2 => {
            let v = m.mem.read_u16(at).map_err(Refusal::mem("GetScriptManagerVariable"))? as i16;
            match manager_variable(v) {
                Some((val, what)) => { put(m, val)?; format!("GetScriptManagerVariable {v} ({what}) -> {val}") }
                None => { done(cpu); return Err(Refusal(format!("GetScriptManagerVariable({v}) has no Roman-only answer here (the frame was popped, the result is untouched)"))); }
            }
        }
        (_, 12) if result == 4 && params == 4 => {
            let (selector, script) = (m.mem.read_u16(at).map_err(Refusal::mem("GetScriptVariable"))? as i16, m.mem.read_u16(at + 2).map_err(Refusal::mem("GetScriptVariable"))? as i16);
            // smSystemScript (-1), smCurrentScript (-2) and smRoman (0) all name the one installed script; smAllScripts (-3) names no script
            if !matches!(script, -2..=0) { done(cpu); return Err(Refusal(format!("GetScriptVariable(script {script}, selector {selector}): only the Roman script (0, -1, -2) is installed here (the frame was popped, the result is untouched)"))); }
            match script_variable(selector) {
                Some((val, what)) => { put(m, val)?; format!("GetScriptVariable {script} {selector} ({what}) -> {val}") }
                None => { done(cpu); return Err(Refusal(format!("GetScriptVariable(script {script}, selector {selector}) has no Roman-only answer here (the frame was popped, the result is untouched)"))); }
            }
        }
        (_, 16) if result == 2 => { put(m, 0)?; String::from("CharByte -> smSingleByte") }
        _ => { done(cpu); return Err(Refusal(format!("_ScriptUtil {name} (selector {sel:#010x}) is not implemented (its {params}-byte frame was popped; the result slot is untouched)"))); }
    };
    done(cpu);
    Ok(format!("[script] {log}"))
}

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> { m.register_os(0xA8B5, script_util::<U>) }
