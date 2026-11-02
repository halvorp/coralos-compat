//! os-mac.library's WHOLE table (feature `os-library`): OS ABI v2's multi-family machine face over the one Mac Toolbox - the 68k
//! Mac (heirloom-mac68k-engine's family, the header's, on cpu-68k.library) and the PowerPC Mac (this crate's machine, on
//! cpu-ppc.library through CPU ABI v3). mac68k.engine and macppc.engine both open this one library (docs/heirloom/plan.tsv row
//! os-mac; the 2026-10-30 ruling /build/patches/h81/macppc-engine-split.consult.md, Option A: the PowerPC personality reaches into
//! the Toolbox's state, so it lives in the same library rather than behind a trap face). The session logic is heirloom-os-abi's.
use crate::machine::MacPpc;
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_common::Refusal;
use heirloom_mac68k_engine::os_face::MacFamily;
use heirloom_os_abi::ppc::PpcVTable;
use heirloom_os_abi::serve::{vtable2, LibUi, OsFamily};
use heirloom_os_abi::ppc::{PpcCodeMem, Tier};
use heirloom_os_abi::{GlHost, OsVTable, BIND_ALREADY, BIND_OK, BIND_REFUSED, CPU_CONTRACT_PPC, FAMILY_MACPPC};

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
    /// macppc.engine's GL worker (OS ABI v3): the guest's AGL / OpenGL batches go to the engine's `gl.req` session from now on.
    fn gl_attach(m: &mut MacPpc<LibUi>, gl: GlHost) -> Result<(), String> {
        m.gl_attach(Box::new(HostGlWorker(gl)));
        Ok(())
    }
    /// The stage clock on the CPU's time-stamp counter (the clock Heirloom.app's in-process machine used), its lines formatted at the
    /// engine's measured rate.
    fn perf(m: &mut MacPpc<LibUi>, tsc_per_us: u64) -> Result<Option<String>, String> {
        if m.perf.is_none() {
            m.perf_enable(tsc);
        }
        Ok(m.perf_take_line(tsc_per_us).map(|l| {
            // which tier ran: the one asked for, and whether the library had to run tier 1 instead (no or a refused provider)
            let e = m.exec();
            format!("{l} | cpu: tier={} fell_back={}", tier_name(e.tier), e.lockstep_refused())
        }))
    }
    /// macppc.engine's executable memory (OS ABI v3): tier 2 runs in it from now on, unless the bundle's `cpu-tier` input said
    /// otherwise.
    fn set_code_mem(m: &mut MacPpc<LibUi>, mem: PpcCodeMem) -> Result<(), String> {
        // the engine gives the memory right after it makes the machine; a `cpu-tier` input (start_file's batch) comes later and wins
        m.exec().set_code_mem(mem);
        m.exec().tier = Tier::Jit;
        Ok(())
    }
    /// The named inputs: `home` (the manifest's `(home ...)`: the program's own folder) and `cpu-tier` (the manifest's `(cpu-tier
    /// "interp" | "blocks" | "jit")`: forces the PowerPC tier - tier 1 on a box where the JIT misbehaves, without a rebuild).
    fn input(m: &mut MacPpc<LibUi>, name: &str, bytes: Vec<u8>) -> Result<(), String> {
        if name == HOME {
            // the program's own folder on the staged volumes, volume first, colon-separated ("Warcraft III"): GuestMachine::set_home,
            // as an in-process host calls it before start_file (the volume chunks precede every input in that batch)
            let path = core::str::from_utf8(&bytes).map_err(|_| format!("{HOME}: the folder is not UTF-8"))?;
            let parts: Vec<&str> = path.split(':').collect();
            return heirloom_common::GuestMachine::set_home(m, &parts).map_err(|r| r.0);
        }
        if name != CPU_TIER {
            return Err(format!("input {name:?}: the PowerPC Mac takes {CPU_TIER:?} and {HOME:?}"));
        }
        let t = core::str::from_utf8(&bytes).ok().and_then(Tier::parse).ok_or_else(|| format!("{CPU_TIER}: {:?} is not a tier (interp, blocks or jit)", String::from_utf8_lossy(&bytes)))?;
        m.exec().tier = t;
        Ok(())
    }
}

/// The named input that forces the PowerPC tier (heirloom-ui-abi Control::Input).
pub const CPU_TIER: &str = "cpu-tier";
/// The named input that sets the program's own folder (a host's `(home ...)`; colon-separated, volume first).
pub const HOME: &str = "home";

fn tier_name(t: Tier) -> &'static str {
    match t {
        Tier::Interp => "interp",
        Tier::Blocks => "blocks",
        Tier::Lockstep => "lockstep",
        Tier::Jit => "jit",
        Tier::JitLockstep => "jitlockstep",
    }
}

/// The CPU's time-stamp counter (an unprivileged instruction on CoralOS, as in Heirloom.app); 0 where there is none.
fn tsc() -> u64 {
    #[cfg(target_arch = "x86_64")]
    // SAFETY: RDTSC has no memory effects; CoralOS leaves it enabled at ring 3.
    return unsafe { core::arch::x86_64::_rdtsc() };
    #[cfg(not(target_arch = "x86_64"))]
    0
}

/// The engine's [`GlHost`] as the machine's GL worker: each batch through the host, the reply copied out (empty = the worker is
/// gone, and the GL call is refused by the machine as for any dead worker).
struct HostGlWorker(GlHost);

impl crate::gl_c::GlWorker for HostGlWorker {
    fn transact(&mut self, request: &[u8]) -> Vec<u8> {
        self.0.transact(request).to_vec()
    }
}

static TABLE: OsVTable = vtable2::<MacFamily, MacPpcFamily>();

/// The table os-mac.library's `coral_lib_open` returns: both Mac families.
pub fn os_mac_vtable() -> &'static OsVTable {
    &TABLE
}
