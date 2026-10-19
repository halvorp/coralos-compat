//! aros.library: ArosInquireA (AROS rom/aros/arosinquirea.c, APL, read for facts only; tag numbers from aros/inquire.h).
//!
//! The engine is NOT AROS, but it presents aros.library 41 so programs built for AROS' ABI open it. Answers are the facts of that
//! ABI, nothing invented: AI_ArosVersion = 41 (the library version), AI_ArosReleaseMajor/Minor 1/12 and AI_ArosReleaseDate 7874 (the
//! AROS 1.12 the ABI is from), AI_ArosABIMajor -1 ("under development", as AROS says), AI_ArosVariant NULL (no distribution), and
//! AI_KickstartSize 0 (a hosted system has no ROM, as AROS' non-native build answers). Tags the engine cannot answer truthfully are LEFT
//! UNTOUCHED, as ArosInquireA does for a tag it does not understand, and named in the refusal list: AI_ArosBuildDate (the engine has no
//! build date), AI_ArosArchitecture (it is no machine architecture) and AI_KickstartBase / Version / Revision (no ROM). The result is
//! the 1-based index of the first tag not handled, or 0.
use alloc::format;
use alloc::string::String;
use crate::libtable::LibraryBuilder;
use crate::machine::Amiga;
use crate::{tags, Refusal};
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

const AI_BASE: u32 = tags::TAG_USER;
const AI_BASE_A: u32 = AI_BASE + 0x10000;

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> { b.named("ArosInquireA", aros_inquire_a::<U>).version(41) }

/// ArosInquireA(a0 = taglist): each tag's ti_Data points at the longword that receives the answer.
fn aros_inquire_a<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let list = cpu.areg(0);
    let t = tags::walk(&a.mem, list)?;
    let (mut first_unhandled, mut log) = (0u32, String::new());
    for (i, &(tag, ptr)) in t.iter().enumerate() {
        let answer: Option<(u32, &str)> = match tag {
            x if x == AI_BASE + 1 => Some((41, "AI_ArosVersion")),
            x if x == AI_BASE + 2 => Some((1, "AI_ArosReleaseMajor")),
            x if x == AI_BASE + 3 => Some((12, "AI_ArosReleaseMinor")),
            x if x == AI_BASE + 4 => Some((7874, "AI_ArosReleaseDate")),
            x if x == AI_BASE + 6 => Some((0, "AI_ArosVariant")),
            x if x == AI_BASE + 8 => Some((u32::MAX, "AI_ArosABIMajor")),
            x if x == AI_BASE_A + 2 => Some((0, "AI_KickstartSize")),
            _ => None,
        };
        match answer {
            Some((v, name)) => { a.mem.write_u32(ptr, v).map_err(Refusal::mem("ArosInquireA result"))?; log.push_str(&format!(" {name}={v:#x}")); }
            None => {
                if first_unhandled == 0 { first_unhandled = i as u32 + 1; }
                let name = match tag { x if x == AI_BASE + 5 => "AI_ArosBuildDate", x if x == AI_BASE + 7 => "AI_ArosArchitecture", x if x == AI_BASE_A + 1 => "AI_KickstartBase", x if x == AI_BASE_A + 3 => "AI_KickstartVersion", x if x == AI_BASE_A + 4 => "AI_KickstartRevision", _ => "an unknown tag" };
                a.refused.push(format!("aros.library/ArosInquireA: {name} ({tag:#x}) is not answered: the engine has no such fact; the result location is left untouched"));
            }
        }
    }
    cpu.set_d0(first_unhandled);
    Ok(format!("[aros] ArosInquireA ->{log}{}", if first_unhandled != 0 { format!(" (first unhandled tag #{first_unhandled})") } else { String::new() }))
}
