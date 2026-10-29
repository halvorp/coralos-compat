//! os-eka.library's table (feature `os-library`): the Symbian EKA2 machine behind heirloom-os-abi's OsVTable, so ekaarm.engine (a
//! CoralOS process) runs it as a dynamically loaded library over cpu-arm.library (docs/heirloom/plan.tsv rows os-eka + ekaarm,
//! doctrine ONE-NAME-ONE-CONCERN-LOADED). The session logic is heirloom-os-abi's (`serve`), shared by every family; this is the
//! Symbian part of it: its family number, its CPU binding, its machine and the three named inputs it takes before `StartFile`
//! (the ordinal table, the layout data and the device ROM - files the in-process host handed over by method call).
use crate::machine::EkaArm;
use crate::romtree::TreeRom;
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_common::Refusal;
use heirloom_os_abi::serve::{vtable, LibUi, OsFamily};
use heirloom_os_abi::{CpuVTable, OsVTable, BIND_ALREADY, BIND_OK, BIND_REFUSED, FAMILY_EKAARM};
use heirloom_ui_abi::wire::session::inputs::{EKA_LAYOUT, EKA_ORDINALS, EKA_ROM};

pub struct EkaFamily;

impl OsFamily for EkaFamily {
    const FAMILY: u32 = FAMILY_EKAARM;
    type Machine = EkaArm<LibUi>;
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
    /// "" = the one machine this engine models (an S60 5th edition device); any other profile is refused by name.
    fn create(profile: &str, ui: LibUi) -> Result<EkaArm<LibUi>, Refusal> {
        if !profile.is_empty() {
            return Err(Refusal(format!("ekaarm: no machine profile {profile:?} (the S60 device is the only one)")));
        }
        Ok(EkaArm::without_ordinals(ui))
    }
    fn input(m: &mut EkaArm<LibUi>, name: &str, bytes: Vec<u8>) -> Result<(), String> {
        match name {
            EKA_ORDINALS => {
                let text = core::str::from_utf8(&bytes).map_err(|_| String::from("input eka-ordinals: the ordinal table is not UTF-8"))?;
                let table = heirloom_fmt_e32::OrdinalTable::parse(text).map_err(|(line, why)| format!("input eka-ordinals: line {line}: {why}"))?;
                m.set_ordinals(table);
                Ok(())
            }
            EKA_LAYOUT if bytes.is_empty() => Err(String::from("input s60-layout: the layout data is empty")),
            EKA_LAYOUT => {
                m.set_cdl_data(bytes);
                Ok(())
            }
            EKA_ROM => {
                m.set_rom(Box::new(TreeRom::from_snapshot(&bytes).map_err(|e| format!("input rom: {e}"))?));
                Ok(())
            }
            other => Err(format!("input {other:?}: the Symbian machine takes {EKA_ORDINALS:?}, {EKA_LAYOUT:?} and {EKA_ROM:?} only")),
        }
    }
}

static TABLE: OsVTable = vtable::<EkaFamily>();

/// The table os-eka.library's `coral_lib_open` returns.
pub fn os_vtable() -> &'static OsVTable {
    &TABLE
}
