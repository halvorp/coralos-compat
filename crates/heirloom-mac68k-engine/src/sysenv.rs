//! `_SysEnvirons` (Inside Macintosh V, "Compatibility Guidelines"; Technical Note 129): what machine and system software
//! the program runs on. `D0.W` = the SysEnvRec version the program understands, `A0` = the record; the result code is in D0.
//!
//! The answer is the machine this engine IS: a 68020 Macintosh II class machine (the CPU core is a 68020), System 7.0.0,
//! a standard ADB keyboard, no FPU (the 68881 is not modelled), and `hasColorQD` by machine profile (TRUE on the
//! Macintosh II: 8-bit Color QuickDraw, [`crate::cqd`]; FALSE on the Plus and the SE). Nothing is claimed that the engine does not do.
//!
//! SysEnvRec (16 bytes): environsVersion 0, machineType 2, systemVersion 4, processor 6, hasFPU 8 (Boolean), hasColorQD 9,
//! keyBoardType 10, atDrvrVersNum 12, sysVRefNum 14 (the WORKING DIRECTORY reference number of the System Folder, not the volume's).
use crate::machine::Mac;
use crate::osconv::Regs;
use crate::Refusal;
use alloc::format;
use alloc::string::String;
use heirloom_ui_abi::GuestUi;
use heirloom_cpu_68k::Cpu;

/// The newest SysEnvRec version this engine fills in (SysEnvirons version 2).
pub const VERSION: i16 = 2;
pub const ENV_BAD_VERS: i16 = -5501;
pub const ENV_VERS_TOO_BIG: i16 = -5502;
const ENV_STAND_ADB_KBD: u16 = 3;
/// System 7.0.0, as BCD in `systemVersion`.
pub const SYSTEM_VERSION: u16 = 0x0700;

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> { m.register_os(0xA090, sys_environs::<U>) }

fn sys_environs<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> Result<String, Refusal> {
    let want = r.d0 as u16 as i16;
    if want < 1 { r.err(ENV_BAD_VERS); return Ok(format!("[toolbox] SysEnvirons version {want}: envBadVers")); }
    let p = r.a0;
    let e = Refusal::mem("SysEnvRec");
    let sys_vref = m.fm.system_folder_wd().map_err(|e| Refusal(format!("SysEnvirons: the System Folder has no working directory (OSErr {e})")))? as u16;
    m.mem.write_u16(p, VERSION.min(want) as u16).map_err(&e)?;
    m.mem.write_u16(p + 2, m.profile.env_machine).map_err(&e)?;
    m.mem.write_u16(p + 4, SYSTEM_VERSION).map_err(&e)?;
    m.mem.write_u16(p + 6, m.profile.env_processor).map_err(&e)?;
    // hasFPU = FALSE (byte 8), hasColorQD (byte 9) by profile
    m.mem.write_u16(p + 8, u16::from(m.profile.color_qd)).map_err(&e)?;
    m.mem.write_u16(p + 10, ENV_STAND_ADB_KBD).map_err(&e)?;
    m.mem.write_u16(p + 12, 0).map_err(&e)?;
    m.mem.write_u16(p + 14, sys_vref).map_err(&e)?;
    r.err(if want > VERSION { ENV_VERS_TOO_BIG } else { 0 });
    Ok(format!("[toolbox] SysEnvirons {want} -> machine {}, System {SYSTEM_VERSION:#06x}, processor {}, no FPU, {} colour QD", m.profile.env_machine, m.profile.env_processor, if m.profile.color_qd { "has" } else { "no" }))
}
