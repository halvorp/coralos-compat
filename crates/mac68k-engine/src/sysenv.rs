//! `_SysEnvirons` (Inside Macintosh V, "Compatibility Guidelines"; Technical Note 129): what machine and system software
//! the program runs on. `D0.W` = the SysEnvRec version the program understands, `A0` = the record; the result code is in D0.
//!
//! The answer is the machine this engine IS: a 68020 Macintosh II class machine (the CPU core is a 68020), System 7.0.0,
//! a standard ADB keyboard, no FPU (the 68881 is not modelled), and black-and-white QuickDraw (`hasColorQD` FALSE: the
//! engine's QuickDraw is 1 bit deep). Nothing is claimed that the engine does not do.
//!
//! SysEnvRec (16 bytes): environsVersion 0, machineType 2, systemVersion 4, processor 6, hasFPU 8 (Boolean), hasColorQD 9,
//! keyBoardType 10, atDrvrVersNum 12, sysVRefNum 14.
use crate::machine::Mac;
use crate::osconv::Regs;
use crate::Refusal;
use alloc::format;
use alloc::string::String;
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

/// The newest SysEnvRec version this engine fills in (SysEnvirons version 2).
pub const VERSION: i16 = 2;
pub const ENV_BAD_VERS: i16 = -5501;
pub const ENV_VERS_TOO_BIG: i16 = -5502;
const ENV_MAC_II: u16 = 4;
const ENV_68020: u16 = 3;
const ENV_STAND_ADB_KBD: u16 = 3;
/// System 7.0.0, as BCD in `systemVersion`.
pub const SYSTEM_VERSION: u16 = 0x0700;

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> { m.register_os(0xA090, sys_environs::<U>) }

fn sys_environs<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> Result<String, Refusal> {
    let want = r.d0 as u16 as i16;
    if want < 1 { r.err(ENV_BAD_VERS); return Ok(format!("[toolbox] SysEnvirons version {want}: envBadVers")); }
    let p = r.a0;
    let e = Refusal::mem("SysEnvRec");
    let sys_vref = m.fm.vref_of(m.fm.def_vol) as u16;
    m.mem.write_u16(p, VERSION.min(want) as u16).map_err(&e)?;
    m.mem.write_u16(p + 2, ENV_MAC_II).map_err(&e)?;
    m.mem.write_u16(p + 4, SYSTEM_VERSION).map_err(&e)?;
    m.mem.write_u16(p + 6, ENV_68020).map_err(&e)?;
    m.mem.write_u16(p + 8, 0).map_err(&e)?; // hasFPU = FALSE, hasColorQD = FALSE
    m.mem.write_u16(p + 10, ENV_STAND_ADB_KBD).map_err(&e)?;
    m.mem.write_u16(p + 12, 0).map_err(&e)?;
    m.mem.write_u16(p + 14, sys_vref).map_err(&e)?;
    r.err(if want > VERSION { ENV_VERS_TOO_BIG } else { 0 });
    Ok(format!("[toolbox] SysEnvirons {want} -> machine {ENV_MAC_II}, System {SYSTEM_VERSION:#06x}, 68020, no FPU, no colour QD"))
}
