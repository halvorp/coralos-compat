//! os-mac.library's table (feature `os-library`): the Mac 68k machine behind heirloom-os-abi's OsVTable, so mac68k.engine (a
//! CoralOS process) runs it as a dynamically loaded library over cpu-68k.library (docs/heirloom/plan.tsv rows os-mac + mac68k,
//! doctrine ONE-NAME-ONE-CONCERN-LOADED). The session logic is heirloom-os-abi's (`serve`), shared by every family; this is the
//! Mac's part of it: its family number, its CPU binding and its machine.
use crate::machine::Mac;
use heirloom_common::Refusal;
use heirloom_os_abi::serve::{vtable, LibUi, OsFamily};
use heirloom_os_abi::{CpuVTable, OsVTable, BIND_ALREADY, BIND_OK, BIND_REFUSED, FAMILY_MAC68K};

pub struct MacFamily;

impl OsFamily for MacFamily {
    const FAMILY: u32 = FAMILY_MAC68K;
    type Machine = Mac<LibUi>;
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
    /// "" = the default machine (mac-ii); otherwise a profile.rs name, an unknown one refused by name.
    fn create(profile: &str, ui: LibUi) -> Result<Mac<LibUi>, Refusal> {
        if profile.is_empty() { Mac::new(ui) } else { Mac::with_profile(ui, profile) }
    }
    fn input(m: &mut Mac<LibUi>, name: &str, bytes: alloc::vec::Vec<u8>) -> Result<(), alloc::string::String> {
        if name == crate::machine::DISPLAY_SIZE_INPUT {
            let (w, h) = crate::machine::parse_display_size(&bytes)?;
            return m.set_display_size(w, h).map_err(|r| r.0);
        }
        if name == heirloom_bind::wire::CDEF_BINDING_INPUT {
            let b = heirloom_bind::wire::decode_cdef(&bytes)?;
            m.add_control_def_binding(b);
            return Ok(());
        }
        if name == heirloom_bind::wire::PLANE_CHROME_BINDING_INPUT {
            let b = heirloom_bind::wire::decode_plane_chrome(&bytes)?;
            m.add_plane_chrome_binding(b);
            return Ok(());
        }
        Err(alloc::format!(
            "input {name:?}: the 68k Mac takes {:?} and {:?}",
            heirloom_bind::wire::CDEF_BINDING_INPUT,
            heirloom_bind::wire::PLANE_CHROME_BINDING_INPUT
        ))
    }
    fn attach_datatype(m: &mut Mac<LibUi>, host: heirloom_os_abi::datatype::DatatypeHost) -> Result<(), alloc::string::String> {
        m.datatype_host = Some(alloc::boxed::Box::new(host));
        Ok(())
    }
    fn attach_clipboard(m: &mut Mac<LibUi>, host: heirloom_os_abi::clip::ClipHost) -> Result<(), alloc::string::String> {
        m.clip_host = Some(alloc::boxed::Box::new(host));
        Ok(())
    }
    fn attach_audio(m: &mut Mac<LibUi>, host: heirloom_os_abi::audio::AudioHost) -> Result<(), alloc::string::String> {
        m.audio_host = Some(alloc::boxed::Box::new(host));
        Ok(())
    }
}

static TABLE: OsVTable = vtable::<MacFamily>();

/// The table os-mac.library's `coral_lib_open` returns.
pub fn os_vtable() -> &'static OsVTable {
    &TABLE
}
