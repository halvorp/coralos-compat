//! AmigaOS `gameport.device`: joystick/controller port queries and async event read (`GPD_READEVENT`).
//! With no hardware joystick attached, `GPD_ASKCTYPE` reports `GPCT_NOCONTROLLER`, setup commands succeed,
//! and `GPD_READEVENT` remains pending until aborted on shutdown (`AbortIO`).
use crate::{machine::Amiga, Refusal};
use heirloom_ui_abi::GuestUi;

pub const NAME: &str = "gameport.device";

const IO_COMMAND: u32 = 28;
const IO_ACTUAL: u32 = 32;
const IO_DATA: u32 = 40;

const CMD_CLEAR: u16 = 5;
const CMD_FLUSH: u16 = 8;
const GPD_READEVENT: u16 = 9;
const GPD_ASKCTYPE: u16 = 10;
const GPD_SETCTYPE: u16 = 11;
const GPD_ASKTRIGGER: u16 = 12;
const GPD_SETTRIGGER: u16 = 13;

/// Start a `gameport.device` IORequest. Returns `Ok(true)` if completed synchronously, `Ok(false)` if left pending.
pub fn start<U: GuestUi>(a: &mut Amiga<U>, io: u32) -> Result<bool, Refusal> {
    let cmd = a.mem.read_u16(io + IO_COMMAND).map_err(Refusal::mem("gameport io_Command"))?;
    match cmd {
        GPD_READEVENT => {
            // Leave pending until AbortIO (no joystick events queued).
            a.gameport_pending.push(io);
            Ok(false)
        }
        GPD_ASKCTYPE => {
            let data = a.mem.read_u32(io + IO_DATA).map_err(Refusal::mem("gameport io_Data"))?;
            if data != 0 {
                a.mem.write_u8(data, 0).map_err(Refusal::mem("GPCT_NOCONTROLLER"))?;
            }
            a.mem.write_u32(io + IO_ACTUAL, 1).map_err(Refusal::mem("gameport io_Actual"))?;
            crate::timer::complete_io(a, io, 0)?;
            Ok(true)
        }
        GPD_SETCTYPE | GPD_ASKTRIGGER | GPD_SETTRIGGER | CMD_CLEAR | CMD_FLUSH => {
            crate::timer::complete_io(a, io, 0)?;
            Ok(true)
        }
        _ => {
            crate::timer::complete_io(a, io, -3)?; // IOERR_NOCMD
            Ok(true)
        }
    }
}
