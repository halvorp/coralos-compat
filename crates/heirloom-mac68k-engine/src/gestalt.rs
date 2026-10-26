//! `_Gestalt` ($A1AD; Inside Macintosh VI, "Gestalt Manager"): ask what the machine and system software are. `D0` = the
//! selector (a four-character code), the answer comes back in `A0` and the result code in `D0`. Programs of the System 7
//! era call it first to decide whether they may run at all.
//!
//! The answers describe the machine the engine IS (the same facts as `_SysEnvirons`, [`crate::sysenv`]; the machine
//! type and processor come from the [`crate::profile`], by default a 68020 Macintosh
//! II class machine), System 7.0.0, 16 MiB of RAM, no FPU, original QuickDraw or 8-bit Color QuickDraw by profile, an ADB keyboard. A selector the engine
//! has no honest answer for is `gestaltUndefSelectorErr` (-5551) with the selector named in the log, never an invented value.
use crate::addrmode::AddrMode;
use crate::machine::Mac;
use crate::osconv::Regs;
use crate::Refusal;
use alloc::format;
use alloc::string::String;
use heirloom_ui_abi::GuestUi;
use heirloom_mem_flat32::DEFAULT_SIZE;
use heirloom_cpu_68k::Cpu;

pub const UNDEF_SELECTOR_ERR: i16 = -5551;


fn answer<U: GuestUi>(m: &Mac<U>, sel: [u8; 4]) -> Option<u32> {
    Some(match &sel {
        b"sysv" => u32::from(crate::sysenv::SYSTEM_VERSION),
        b"vers" => 1, // the Gestalt Manager itself: version 1
        b"mach" => m.profile.gestalt_machine,
        b"cput" => m.profile.gestalt_cpu,
        b"proc" => u32::from(m.profile.env_processor), // the 68k processor family member (1 = 68000, 3 = 68020), as SysEnvirons' processor
        b"fpu " => 0, // no FPU
        // gestaltQuickdrawVersion: gestaltOriginalQD $000, gestalt8BitQD $100 (Gestalt.h; Inside Macintosh: Imaging With QuickDraw A-?)
        b"qd  " => if m.profile.color_qd { 0x100 } else { 0 },
        // gestaltQuickdrawFeatures: bit 0 gestaltHasColor; no deep GWorlds (bit 1), no direct PixMaps (bit 2), no grayish-text-or (bit 3)
        b"qdrw" => u32::from(m.profile.color_qd),
        b"kbd " => 3, // gestaltStandADBKbd
        b"ram " | b"lram" => DEFAULT_SIZE as u32, // physical / logical RAM: the guest address space
        // gestalt32BitAddressing (bit 0) = the machine is IN 32-bit mode now; gestalt32BitCapable (bit 2) = it can be.
        b"addr" => u32::from(m.address_mode() == AddrMode::Bit32) | (1 << 2),
        // gestaltFSAttr: bit 1 gestaltHasFSSpecCalls (the FSp... calls are `_HighLevelFSDispatch`, fm/fsspec.rs); nothing else is claimed
        b"fs  " => 1 << 1,
        b"os  " => 0, // gestaltClassicOS is not asked of a 68k System 7: the sysv selector is the answer
        _ => return None,
    })
}

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> { m.register_os(0xA1AD, gestalt::<U>) }

fn gestalt<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> Result<String, Refusal> {
    let sel = r.d0.to_be_bytes();
    let name: String = sel.iter().map(|&b| char::from(b)).collect();
    match answer(m, sel) {
        Some(v) => { r.a0 = v; r.err(0); Ok(format!("[toolbox] Gestalt '{name}' -> {v:#x}")) }
        None => { r.err(UNDEF_SELECTOR_ERR); Ok(format!("[toolbox] Gestalt '{name}': gestaltUndefSelectorErr (no honest answer)")) }
    }
}
