//! The Help Manager's trap (`_Pack14`, $A830; Inside Macintosh: Macintosh Toolbox Essentials, "Help Manager"): a Toolbox trap whose
//! routine is named by D0, the routine's Pascal arguments on the stack as for any other. Balloon Help is OFF here, always: the host
//! has no balloon part, and a Mac with the Help menu off answers the same way.
//!
//! Implemented: `HMGetBalloons` (D0 = 3, `FUNCTION HMGetBalloons: BOOLEAN`, no arguments) answers FALSE. SimpleText 1.3 and 1.4 ask it
//! before they build their Help menu. Every other selector is refused BY NAME and its frame left as the program made it (the selector
//! values for the rest are not in a source we hold, and a guessed frame would corrupt the stack): `HMGetHelpMenuHandle`,
//! `HMSetBalloons`, `HMShowBalloon`, `HMRemoveBalloon`, `HMShowMenuBalloon`, `HMIsBalloon`, ...
use crate::machine::{Mac, Regs};
use crate::Refusal;
use alloc::format;
use alloc::string::String;
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

fn pack14<U: GuestUi>(m: &mut Mac<U>, cpu: &mut Cpu, r: &mut Regs) -> Result<String, Refusal> {
    let sel = (r.d0 & 0xFFFF) as u16;
    match sel {
        3 => {
            // the BOOLEAN result slot is the word at SP; its first (high) byte is the value
            let sp = cpu.areg(7);
            m.mem.write_u16(sp, 0).map_err(Refusal::mem("HMGetBalloons result"))?;
            Ok(String::from("[help] HMGetBalloons -> FALSE (Balloon Help is off)"))
        }
        other => Err(Refusal(format!("_Pack14 (Help Manager) selector {other:#06x} is not implemented (D0 = {:#010x}; the frame is unknown and was left as the program made it)", r.d0))),
    }
}

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> { m.register_os(0xA830, pack14::<U>) }
