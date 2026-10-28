//! dos.library DoPkt: synchronous packets to a device's handler port.
//!
//! The engine has no handler processes; its devices' MsgPorts (GetDeviceProc, DosList nodes) are private and nothing serves
//! them. DoPkt(d1 = port, d2 = action, d3..d7 = arg1..arg5) therefore performs the action HOST-side for the packets a real
//! program sends to a file system, and returns the packet's res1 in D0 with res2 in IoErr(). Supported: ACTION_DISK_INFO (25: arg1 =
//! BPTR to an InfoData, filled as Info() does). Every other action, and any port that is not a device's, is a named refusal
//! (DOpus: Library/devices.c:231, launcher.c:1745 ask for disk info; the 64-bit size packets ACTION_GET_FILE_SIZE64 come when a
//! lister reads a directory with Examine64-style calls).
use alloc::format;
use alloc::string::String;
use crate::dos_err::*;
use crate::libtable::LibraryBuilder;
use crate::machine::Amiga;
use crate::{doslist, Refusal};
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> { b.named("DoPkt", do_pkt::<U>) }

const ACTION_DISK_INFO: u32 = 25;
const DOSTRUE: u32 = 0xFFFF_FFFF;

fn do_pkt<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (port, action, arg1) = (cpu.dreg(1), cpu.dreg(2), cpu.dreg(3));
    let Some(dev) = doslist::device_of_port(a, port) else { return Err(Refusal(format!("DoPkt: {port:#x} is not a device's handler port (no process serves it)"))) };
    match action {
        ACTION_DISK_INFO => {
            let node = a.fs.vol.resolve(0, &format!("{dev}:")).map_err(|e| Refusal(format!("DoPkt ACTION_DISK_INFO: device {dev}: {}", e.0)))?;
            let id = arg1 << 2;
            crate::dos_misc::fill_info_data(a, a.fs.vol.device_of(node), id)?;
            set_ioerr(&mut a.mem, DosErr(0));
            cpu.set_d0(DOSTRUE);
            Ok(format!("[dos] DoPkt {dev}: ACTION_DISK_INFO -> InfoData {id:#x}"))
        }
        other => Err(Refusal(format!("DoPkt: action {other} to device {dev}: is not modelled (only ACTION_DISK_INFO, 25)"))),
    }
}
