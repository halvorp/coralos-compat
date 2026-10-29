//! os-ami.library's table (feature `os-library`): the AmigaOS 68k machine behind heirloom-os-abi's OsVTable, so ami68k.engine (a
//! CoralOS process) runs it as a dynamically loaded library over cpu-68k.library (docs/heirloom/plan.tsv rows os-ami + ami68k,
//! doctrine ONE-NAME-ONE-CONCERN-LOADED). The session logic is heirloom-os-abi's (`serve`); this is the Amiga's part of it: its
//! family number, its CPU binding, its machine, and the two controls only an Amiga takes - AmigaDOS assigns from the bundle's
//! manifest and MUI's host profile (doc 23 Phase 3; text is sized by the guest font: the host's measurer stays in the host).
use crate::machine::Amiga;
use crate::mui::layout::Metrics;
use alloc::format;
use alloc::string::String;
use heirloom_common::Refusal;
use heirloom_os_abi::serve::{vtable, LibUi, OsFamily};
use heirloom_os_abi::{CpuVTable, OsVTable, BIND_ALREADY, BIND_OK, BIND_REFUSED, FAMILY_AMI68K};
use heirloom_ui_abi::wire::HostProfile;

pub struct AmiFamily;

/// MUI's layout metrics for a host profile. `text_w` is None: the host's text measurer cannot cross the process boundary, so
/// MUI sizes every text by the screen font the guest opened (the fallback doc 23 Phase 3 names for a refused MEASURE).
pub fn metrics_of(p: &HostProfile) -> Metrics {
    Metrics {
        toggle: p.toggle,
        control_h: p.control_h,
        pad_w: p.pad_w,
        label_right: p.label_right,
        text_w: None,
        group_head: p.group_head,
        group_inset: p.group_inset,
        label_gap: p.label_gap,
        spacing: p.spacing,
        win_pad: p.win_pad,
    }
}

impl OsFamily for AmiFamily {
    const FAMILY: u32 = FAMILY_AMI68K;
    type Machine = Amiga<LibUi>;
    fn bind_cpu(cpu: &'static CpuVTable) -> i32 {
        match crate::cpu::bind(cpu) {
            Ok(()) => BIND_OK,
            Err(crate::cpu::BindRefusal::AlreadyBound) => BIND_ALREADY,
            Err(_) => BIND_REFUSED,
        }
    }
    fn cpu_bound() -> bool {
        crate::cpu::is_bound()
    }
    /// The Amiga has one machine: "" (or "amiga") is it; any other profile is refused by name.
    fn create(profile: &str, ui: LibUi) -> Result<Amiga<LibUi>, Refusal> {
        match profile {
            "" | "amiga" => Amiga::new(ui),
            other => Err(Refusal(format!("ami68k: no machine profile {other:?} (the Amiga machine takes \"\" or \"amiga\")"))),
        }
    }
    fn assign(m: &mut Amiga<LibUi>, name: &str, target: &str, add: bool) -> Result<(), String> {
        if name.is_empty() || name.contains(':') || name.contains('/') {
            return Err(format!("assign {name:?}: not an AmigaDOS assign name"));
        }
        if add { m.fs.vol.assign_add(name, target) } else { m.fs.vol.assign(name, target) }
        Ok(())
    }
    fn host_profile(m: &mut Amiga<LibUi>, p: &HostProfile) -> Result<(), String> {
        m.objects.mui_metrics = metrics_of(p);
        Ok(())
    }
}

static TABLE: OsVTable = vtable::<AmiFamily>();

/// The table os-ami.library's `coral_lib_open` returns.
pub fn os_vtable() -> &'static OsVTable {
    &TABLE
}
