//! exec.library devices: OpenDevice / CloseDevice. A device the name table knows (timer.device, input.device,
//! console.device) opens with every vector a named refusal, exactly like an on-demand library, so a real program runs
//! until the first device function or command it needs; any other device fails to open, by name (IOERR_OPENFAIL).
//!
//! struct IORequest (exec/io.h): io_Message 0 (a struct Message, 20 bytes), io_Device 20, io_Unit 24, io_Command 28,
//! io_Flags 30, io_Error 31.
use alloc::string::String;
use alloc::format;
use crate::{machine::Amiga, Refusal};
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

pub const LVO_OPEN_DEVICE: i32 = -444;
pub const LVO_CLOSE_DEVICE: i32 = -450;
pub const LVO_OPEN_RESOURCE: i32 = -498;

const IO_DEVICE: u32 = 20;
const IO_UNIT: u32 = 24;
const IO_ERROR: u32 = 31;
/// exec/errors.h IOERR_OPENFAIL.
const IOERR_OPENFAIL: i8 = -1;

/// OpenDevice(a0 = name, d0 = unit, a1 = IORequest, d1 = flags) -> d0 = 0 or an IOERR_ code (also in io_Error).
pub fn open_device<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let name = a.mem.read_cstr(cpu.areg(0)).map_err(Refusal::mem("OpenDevice.devName"))?;
    let (unit, io) = (cpu.dreg(0), cpu.areg(1));
    let known = crate::libtable::known_library(&name).filter(|n| n.ends_with(".device"));
    let base = match (known, a.libs.iter_mut().find(|l| l.name.eq_ignore_ascii_case(&name))) {
        (_, Some(l)) => { l.open_count += 1; Some(l.base) }
        (Some(k), None) => Some(a.open_known_library(k)?),
        (None, None) => None,
    };
    let err = if base.is_some() { 0 } else { IOERR_OPENFAIL };
    a.mem.write_u8(io + IO_ERROR, err as u8).map_err(Refusal::mem("io_Error"))?;
    if let Some(b) = base {
        a.mem.write_u32(io + IO_DEVICE, b).map_err(Refusal::mem("io_Device"))?;
        a.mem.write_u32(io + IO_UNIT, unit).map_err(Refusal::mem("io_Unit"))?;
    }
    cpu.set_d0(err as i32 as u32);
    Ok(match base {
        Some(b) => format!("[exec] OpenDevice \"{name}\" unit={unit} -> {b:#x}"),
        None => format!("[exec] OpenDevice \"{name}\" unit={unit} -> IOERR_OPENFAIL (device not modelled)"),
    })
}

/// CloseDevice(a1 = IORequest).
pub fn close_device<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let io = cpu.areg(1);
    let dev = a.mem.read_u32(io + IO_DEVICE).map_err(Refusal::mem("io_Device"))?;
    let l = a.libs.iter_mut().find(|l| l.base == dev).ok_or_else(|| Refusal(format!("CloseDevice: io_Device {dev:#x} is not an open device")))?;
    l.open_count = l.open_count.checked_sub(1).ok_or_else(|| Refusal(format!("CloseDevice: {} closed more than opened", l.name)))?;
    Ok(format!("[exec] CloseDevice {}", l.name))
}

/// OpenResource(a1 = name) -> d0 = the resource, or NULL. The machine models one resource, AROS's `task.resource`
/// (task_resource.rs), created on first use; every other name fails, which is what real exec does for a resource nobody
/// added (AmigaOS programs probe e.g. "FileSystem.resource" and carry on).
pub fn open_resource<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let name = a.mem.read_cstr(cpu.areg(1)).map_err(Refusal::mem("OpenResource.resName"))?;
    if !name.eq_ignore_ascii_case(crate::task_resource::NAME) {
        cpu.set_d0(0);
        return Ok(format!("[exec] OpenResource \"{name}\" -> NULL (no such resource)"));
    }
    let base = match a.task_res.base {
        Some(b) => b,
        None => {
            let b = a.alloc_lib_base(crate::task_resource::NAME)?;
            a.install_lib(crate::task_resource::library(b))?;
            a.task_res.base = Some(b);
            b
        }
    };
    cpu.set_d0(base);
    Ok(format!("[exec] OpenResource \"{name}\" -> {base:#x}"))
}
