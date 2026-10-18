//! exec.library: OpenLibrary / CloseLibrary by name, over the fake ExecBase.
use alloc::string::String;
use alloc::format;
use crate::{machine::Amiga, Refusal};
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

pub const LVO_OPEN_LIBRARY: i32 = -552;
pub const LVO_CLOSE_LIBRARY: i32 = -414;
pub const LVO_OLD_OPEN_LIBRARY: i32 = -408;

/// a1 = library name, d0 = minimum version. Returns the base, or 0 for an unknown library
/// (a legitimate NULL, as on a real machine, so not a refusal).
pub fn open_library<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let name = a.mem.read_cstr(cpu.areg(1)).map_err(Refusal::mem("OpenLibrary.libName"))?;
    let ver = cpu.dreg(0);
    let base = match a.libs.iter_mut().find(|l| l.name.eq_ignore_ascii_case(&name)) {
        Some(l) => { l.open_count += 1; l.base }
        // A library the name table knows: open it with every vector a named refusal, so a real program runs until the
        // first function it needs that is not bridged, and says which. Unknown names stay a legitimate NULL.
        None => {
            // A library on disk (LIBS:, PROGDIR:Libs): loaded and initialised in the guest, which sets D0 itself.
            if let Some(line) = crate::disklib::try_open(a, cpu, &name, ver)? { return Ok(line); }
            match crate::libtable::known_library(&name) {
                Some(known) => a.open_known_library(known)?,
                None => 0,
            }
        }
    };
    cpu.set_d0(base);
    Ok(format!("[exec] OpenLibrary name=\"{name}\" ver={ver} -> {base:#x}"))
}

/// a1 = library base.
pub fn close_library<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let base = cpu.areg(1);
    if let Some(i) = crate::disklib::index_of_base(a, base) { return crate::disklib::close(a, cpu, i); }
    let l = a.libs.iter_mut().find(|l| l.base == base)
        .ok_or_else(|| Refusal(format!("CloseLibrary: {base:#x} is not an open library base")))?;
    if l.open_count == 0 { return Err(Refusal(format!("CloseLibrary: {} closed more times than opened", l.name))); }
    l.open_count -= 1;
    Ok(format!("[exec] CloseLibrary {} base={base:#x}", l.name))
}

/// OldOpenLibrary(a1 = name): OpenLibrary with no version requirement (exec 1.x; C startups still call it).
pub fn old_open_library<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    cpu.set_d0(0);
    open_library(a, cpu)
}
