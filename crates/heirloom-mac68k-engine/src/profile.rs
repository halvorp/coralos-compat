//! Machine profiles: WHICH classic Mac the engine is, chosen by name from the bundle's data
//! (`Info.sexp (guest (machine "mac-plus"))`), never from what an application is called or how old it looks.
//!
//! A profile fixes what a program can observe about the processor: the CPU model (and with it the instruction set and the
//! exception stack frame: a 68000 pushes SR + PC, 6 bytes, where a 68020 pushes a fourth word, 8 bytes, so a handler that
//! pops the frame by size only works on the model it was written for), the address mode the machine boots in, and the
//! answers `_Gestalt` / `_SysEnvirons` give. The default profile is [`DEFAULT`] (the Macintosh II class machine the engine
//! always was: 68020, 24-bit addressing).
//!
//! Codes: `'mach'` (Gestalt machine type, Inside Macintosh VI 3-15), `'cput'` (0 = 68000, 1 = 68010, 2 = 68020), the
//! SysEnvRec `machineType` (envMacPlus 2, envSE 3, envMacII 4) and `processor` (env68000 1, env68020 3).
use crate::addrmode::AddrMode;
use crate::cpu::CpuModel;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MachineProfile {
    pub name: &'static str,
    pub cpu: CpuModel,
    pub addr: AddrMode,
    pub gestalt_machine: u32,
    pub gestalt_cpu: u32,
    pub env_machine: u16,
    pub env_processor: u16,
    /// Color QuickDraw (8-bit, the Macintosh II's) is in the ROM: SysEnvirons `hasColorQD`, Gestalt `'qd  '` / `'qdrw'`, and the
    /// `ROM85` global agree on it. The Plus and the SE have the original 1-bit QuickDraw only.
    pub color_qd: bool,
    /// The low-memory word `ROM85` ($28E): $7FFF on a Plus, $3FFF on the color-era ROMs (executor docs/globals.cpp:98-102 and
    /// src/init.cpp:466; Inside Macintosh V-328).
    pub rom85: u16,
}

const MAC_II: MachineProfile = MachineProfile { name: "mac-ii", cpu: CpuModel::M68020, addr: AddrMode::Bit24, gestalt_machine: 6, gestalt_cpu: 2, env_machine: 4, env_processor: 3, color_qd: true, rom85: 0x3FFF };
const MAC_PLUS: MachineProfile = MachineProfile { name: "mac-plus", cpu: CpuModel::M68000, addr: AddrMode::Bit24, gestalt_machine: 4, gestalt_cpu: 0, env_machine: 2, env_processor: 1, color_qd: false, rom85: 0x7FFF };
const MAC_SE: MachineProfile = MachineProfile { name: "mac-se", cpu: CpuModel::M68000, addr: AddrMode::Bit24, gestalt_machine: 5, gestalt_cpu: 0, env_machine: 3, env_processor: 1, color_qd: false, rom85: 0x3FFF };

/// Every profile, by the name a bundle's `machine` entry uses.
pub const PROFILES: &[MachineProfile] = &[MAC_II, MAC_PLUS, MAC_SE];
/// The profile of a machine nobody configured.
pub const DEFAULT: MachineProfile = MAC_II;

/// The profile called `name` (ASCII case-insensitive), or `None`: the caller refuses by name, it does not fall back.
pub fn lookup(name: &str) -> Option<MachineProfile> { PROFILES.iter().copied().find(|p| p.name.eq_ignore_ascii_case(name)) }
