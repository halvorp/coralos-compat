//! os-mac.library's WHOLE table (feature `os-library`): OS ABI v2's multi-family machine face over the one Mac Toolbox - the 68k
//! Mac (heirloom-mac68k-engine's family, the header's, on cpu-68k.library) and the PowerPC Mac (this crate's machine, on
//! cpu-ppc.library through CPU ABI v3). mac68k.engine and macppc.engine both open this one library (docs/heirloom/plan.tsv row
//! os-mac; the 2026-10-30 ruling /build/patches/h81/macppc-engine-split.consult.md, Option A: the PowerPC personality reaches into
//! the Toolbox's state, so it lives in the same library rather than behind a trap face). The session logic is heirloom-os-abi's.
use crate::machine::MacPpc;
use alloc::format;
use heirloom_common::Refusal;
use heirloom_mac68k_engine::os_face::MacFamily;
use heirloom_os_abi::ppc::PpcVTable;
use heirloom_os_abi::serve::{vtable2, LibUi, OsFamily};
use heirloom_os_abi::{OsVTable, BIND_ALREADY, BIND_OK, BIND_REFUSED, CPU_CONTRACT_PPC, FAMILY_MACPPC};

pub struct MacPpcFamily;

impl OsFamily for MacPpcFamily {
    const FAMILY: u32 = FAMILY_MACPPC;
    const CPU_CONTRACT: u32 = CPU_CONTRACT_PPC;
    type Machine = MacPpc<LibUi>;
    fn bind_cpu_ppc(cpu: &'static PpcVTable) -> i32 {
        match crate::cpu::bind(cpu) {
            Ok(()) => BIND_OK,
            Err(crate::cpu::BindRefusal::AlreadyBound) => BIND_ALREADY,
            Err(_) => BIND_REFUSED,
        }
    }
    fn cpu_bound() -> bool {
        crate::cpu::is_bound()
    }
    /// "" = the PowerPC Mac (its RAM is sized from the program's SIZE partition at start_file); any other profile is refused by
    /// name: the 68k profiles (mac-ii, ...) name 68k machines.
    fn create(profile: &str, ui: LibUi) -> Result<MacPpc<LibUi>, Refusal> {
        if !profile.is_empty() {
            return Err(Refusal(format!("macppc: no machine profile {profile:?} (the PowerPC Mac is the only one)")));
        }
        MacPpc::new(ui).map_err(|e| Refusal(format!("macppc: {e}")))
    }
}

static TABLE: OsVTable = vtable2::<MacFamily, MacPpcFamily>();

/// The table os-mac.library's `coral_lib_open` returns: both Mac families.
pub fn os_mac_vtable() -> &'static OsVTable {
    &TABLE
}
