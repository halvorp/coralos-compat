//! The Cursor Device Manager (`_CursorDeviceDispatch` $AADB, selector in D0, Pascal arguments on the stack; Universal Interfaces
//! CursorDevices.h: `MOVEQ #sel,D0; _CursorDeviceDispatch`). Mac OS 9 has it, so a program may call it without testing for it
//! (Warcraft III's own CursorDeviceLib fragment does, and QUITS when `CursorDeviceNewDevice` fails). The list of devices
//! is real: `CursorDeviceNewDevice` allocates a zeroed `CursorDevice` record (`CURSOR_DEVICE_SIZE` bytes, the program fills its own
//! fields) and registers it, `CursorDeviceNextDevice` walks the registered ones, `CursorDeviceDisposeDevice` unregisters and frees.
//! NAMED LIMIT: the pointer on screen is the HOST's - a `Move`/`MoveTo` on a registered device is accepted (noErr) and NOT carried
//! out, said once as a refusal; the button / acceleration / timing settings are accepted and have no effect here. A device that is not
//! registered is paramErr.
use crate::machine::Mac;
use crate::Refusal;
use crate::osconv::Regs;
use crate::pascal::{self, Arg};
use alloc::format;
use alloc::string::String;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

const PARAM_ERR: i16 = -50;
const MEM_FULL_ERR: i16 = -108;
/// The `CursorDevice` record's size: CursorDevices.h's struct (next, cursor, refCon, unused, devID, resolution, devClass, cntButtons,
/// filler, buttons, buttonOp[8], unused2) is 56 bytes; rounded up so a later field the program writes still lands inside.
pub const CURSOR_DEVICE_SIZE: u32 = 64;

/// (selector, name, Pascal arguments in declaration order). The result is an OSErr word.
const CALLS: &[(u32, &str, &[Arg])] = &[
    (0x00, "CursorDeviceMove", &[Arg::L, Arg::L, Arg::L]),
    (0x01, "CursorDeviceMoveTo", &[Arg::L, Arg::L, Arg::L]),
    (0x02, "CursorDeviceFlush", &[Arg::L]),
    (0x03, "CursorDeviceButtons", &[Arg::L, Arg::W]),
    (0x04, "CursorDeviceButtonDown", &[Arg::L]),
    (0x05, "CursorDeviceButtonUp", &[Arg::L]),
    (0x06, "CursorDeviceButtonOp", &[Arg::L, Arg::W, Arg::W, Arg::L]),
    (0x07, "CursorDeviceSetButtons", &[Arg::L, Arg::W]),
    (0x08, "CursorDeviceSetAcceleration", &[Arg::L, Arg::L]),
    (0x09, "CursorDeviceDoubleTime", &[Arg::L, Arg::L]),
    (0x0A, "CursorDeviceUnitsPerInch", &[Arg::L, Arg::L]),
    (0x0B, "CursorDeviceNextDevice", &[Arg::L]),
    (0x0C, "CursorDeviceNewDevice", &[Arg::L]),
    (0x0D, "CursorDeviceDisposeDevice", &[Arg::L]),
];

/// The call `selector` names: (name, its Pascal argument layout).
pub fn layout(selector: u32) -> Option<(&'static str, &'static [Arg])> { CALLS.iter().find(|c| c.0 == selector).map(|c| (c.1, c.2)) }

impl<U: GuestUi> Mac<U> {
    /// A Cursor Device Manager call BY VALUE (the PowerPC entry, Mixed Mode's `CallUniversalProc` on the trap): `args` in declaration
    /// order. Runs the same body the 68k trap runs. The OSErr, and the log line.
    pub fn cursor_device_call(&mut self, selector: u32, args: &[u32]) -> Result<i16, Refusal> {
        let (name, _) = layout(selector).ok_or_else(|| Refusal(format!("CursorDeviceDispatch selector {selector:#x} is not a Cursor Device Manager call")))?;
        let arg = |i: usize| args.get(i).copied().unwrap_or(0);
        let known = |m: &Mac<U>, d: u32| d != 0 && m.cursor_devices.contains(&d);
        let (e, why): (i16, String) = match selector {
            // ourDevice: in, the device to start after (NIL = the first); out, the next registered one (NIL at the end)
            0x0B => {
                let p = arg(0);
                if p == 0 { (PARAM_ERR, String::from("ourDevice is NIL")) } else {
                    let after = self.mem.read_u32(p).map_err(Refusal::mem("CursorDeviceNextDevice ourDevice"))?;
                    let next = if after == 0 { self.cursor_devices.first().copied() } else {
                        self.cursor_devices.iter().position(|&d| d == after).and_then(|i| self.cursor_devices.get(i + 1).copied())
                    };
                    self.mem.write_u32(p, next.unwrap_or(0)).map_err(Refusal::mem("CursorDeviceNextDevice ourDevice"))?;
                    (0, format!("{:#x}", next.unwrap_or(0)))
                }
            }
            0x0C => {
                let p = arg(0);
                if p == 0 { (PARAM_ERR, String::from("ourDevice is NIL")) } else {
                    match self.mm.new_ptr(&mut self.mem, &mut self.heap, CURSOR_DEVICE_SIZE, true) {
                        Err(x) => (MEM_FULL_ERR, format!("no memory for the record ({x})")),
                        Ok(d) => {
                            self.mem.write_u32(p, d).map_err(Refusal::mem("CursorDeviceNewDevice ourDevice"))?;
                            self.cursor_devices.push(d);
                            (0, format!("registered {d:#x}"))
                        }
                    }
                }
            }
            0x0D => {
                let d = arg(0);
                if !known(self, d) { (PARAM_ERR, format!("{d:#x} is not a registered cursor device")) } else {
                    self.cursor_devices.retain(|&x| x != d);
                    let _ = self.mm.dispose_ptr(&mut self.heap, d);
                    (0, format!("unregistered {d:#x}"))
                }
            }
            0x00 | 0x01 if known(self, arg(0)) => {
                if !self.refused.iter().any(|r| r.starts_with("CursorDeviceMove")) {
                    self.note_refusal(name, String::from("NAMED LIMIT: the pointer is the host's; a guest warp is accepted and not carried out"));
                }
                (0, format!("({}, {}) not carried out: the host owns the pointer", arg(1) as i32, arg(2) as i32))
            }
            _ if known(self, arg(0)) => (0, String::from("accepted; no effect on the host's pointer")),
            _ => (PARAM_ERR, format!("device {:#x} is not a registered cursor device", arg(0))),
        };
        self.log.push(format!("[cursor] {name} -> {e} ({why})"));
        Ok(e)
    }
}

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> { m.register_os(0xAADB, dispatch::<U>) }

fn dispatch<U: GuestUi>(m: &mut Mac<U>, c: &mut Cpu, r: &mut Regs) -> Result<String, Refusal> {
    let selector = c.dreg(0);
    let (name, lay) = layout(selector).ok_or_else(|| Refusal(format!("CursorDeviceDispatch selector {selector:#x} is not a Cursor Device Manager call")))?;
    let sp = c.areg(7);
    let a = pascal::read_args(&m.mem, sp, lay)?;
    let values: alloc::vec::Vec<u32> = (0..lay.len()).map(|i| a.get(i)).collect();
    let e = m.cursor_device_call(selector, &values)?;
    let new_sp = pascal::complete(&mut m.mem, sp, lay, Some((Arg::W, e as u16 as u32)))?;
    c.set_areg(7, new_sp);
    r.d0 = 0;
    Ok(format!("[cursor] CursorDeviceDispatch {name} -> {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_selector_has_a_layout_and_the_list_is_in_selector_order() {
        for (i, c) in CALLS.iter().enumerate() { assert_eq!(c.0, i as u32, "{}", c.1); }
        assert!(layout(0x0E).is_none());
    }
}
