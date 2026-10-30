//! The Apple Event Manager for PowerPC programs: the HANDLER REGISTRY and the one event the host sends (`'aevt'/'oapp'`, "open application"), the same scope as the 68k engine's
//! (`heirloom_mac68k_engine::appleevents`) but with PowerPC handlers: `AEProcessAppleEvent` runs `OSErr handler(const AppleEvent *event, AppleEvent *reply, long refcon)` as the program's own
//! code through the callback machine and returns its OSErr. The event has NO parameters and no reply is wanted, so reading one (`AEGetParamDesc`, `AEGetParamPtr`, `AEGetAttributePtr`,
//! `AEGetNthPtr`, `AECoerceDesc`) answers `errAEDescNotFound` (-1701) and `AECountItems` of it `errAENotAList` (-1704) -- real, honest answers for an event that carries nothing, and each
//! logged. Building and sending events (`AECreateAppleEvent`, `AESend`) is NOT provided and stays unbound by name.
//! An `AEDesc` is 8 bytes: the type then a Handle; an event is `'aevt'` with an empty handle, the reply `'null'` with none.
use crate::callback::{Continuation, Frame, Saved};
use crate::clib::arg;
use crate::machine::MacPpc;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use heirloom_ui_abi::GuestUi;
use heirloom_mac68k_engine::events::MacEvent;
use crate::cpu::Cpu;

pub const ERR_AE_DESC_NOT_FOUND: i32 = -1701;
pub const ERR_AE_NOT_A_LIST: i32 = -1704;
pub const ERR_AE_EVENT_NOT_HANDLED: i32 = -1708;
pub const ERR_AE_HANDLER_NOT_FOUND: i32 = -1717;
const fn fc(b: &[u8; 4]) -> u32 { u32::from_be_bytes(*b) }
const HIGH_LEVEL_EVENT: u16 = 23;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AeFn { InstallEventHandler, RemoveEventHandler, GetEventHandler, ProcessAppleEvent, DisposeDesc, GetParam, CountItems, SetInteractionAllowed, GetInteractionAllowed }
impl AeFn {
    pub fn by_name(name: &str) -> Option<AeFn> {
        use AeFn::*;
        Some(match name {
            "AEInstallEventHandler" => InstallEventHandler, "AERemoveEventHandler" => RemoveEventHandler, "AEGetEventHandler" => GetEventHandler, "AEProcessAppleEvent" => ProcessAppleEvent,
            "AEDisposeDesc" => DisposeDesc, "AEGetParamDesc" | "AEGetParamPtr" | "AEGetAttributePtr" | "AEGetNthPtr" | "AEGetNthDesc" | "AECoerceDesc" => GetParam, "AECountItems" => CountItems,
            "AESetInteractionAllowed" => SetInteractionAllowed, "AEGetInteractionAllowed" => GetInteractionAllowed,
            _ => return None,
        })
    }
}

/// (class, id, isSysHandler) -> (handler TVector, refcon).
#[derive(Default)]
pub(crate) struct AeState { handlers: BTreeMap<(u32, u32, bool), (u32, u32)>, oapp_sent: bool, interaction: u32 }

impl<U: GuestUi> MacPpc<U> {
    pub(crate) fn ae_call(&mut self, f: AeFn, cpu: &mut Cpu) -> Result<(), String> {
        use AeFn::*;
        let a = |m: &Self, i: usize| arg(m, cpu, i);
        let (a0, a1, a2, a3) = (a(self, 0)?, a(self, 1).unwrap_or(0), a(self, 2).unwrap_or(0), a(self, 3).unwrap_or(0));
        let ret = cpu.lr & !3;
        let r: i32 = match f {
            InstallEventHandler => { // (class, id, handler UPP, refcon, isSysHandler)
                let sys = a(self, 4)? & 0xFF != 0;
                let tv = self.proc_tvector(a2)?;
                self.rt.ae.handlers.insert((a0, a1, sys), (tv, a3));
                self.mac.log.push(format!("[ae] AEInstallEventHandler '{}'/'{}' -> {tv:#x}", String::from_utf8_lossy(&a0.to_be_bytes()), String::from_utf8_lossy(&a1.to_be_bytes())));
                // the Finder's first act on launching a program is to send it "open application"
                if (a0, a1) == (fc(b"aevt"), fc(b"oapp")) && !self.rt.ae.oapp_sent {
                    self.rt.ae.oapp_sent = true;
                    self.mac.ev.queue.push_back(MacEvent { what: HIGH_LEVEL_EVENT, message: a0, when: self.mac.ev.ticks, v: 0, h: 0, modifiers: 0 });
                    if let Some(e) = self.mac.ev.queue.back_mut() { e.v = (a1 >> 16) as i16; e.h = a1 as i16; } // where = the event id
                }
                0
            }
            RemoveEventHandler => if self.rt.ae.handlers.remove(&(a0, a1, a(self, 2)? & 0xFF != 0)).is_some() { 0 } else { ERR_AE_HANDLER_NOT_FOUND },
            GetEventHandler => { // (class, id, &handler, &refcon, isSysHandler)
                match self.rt.ae.handlers.get(&(a0, a1, a(self, 4)? & 0xFF != 0)).copied() {
                    Some((tv, refcon)) => { self.put32(a2, tv, "AEGetEventHandler")?; self.put32(a3, refcon, "AEGetEventHandler")?; 0 }
                    None => ERR_AE_HANDLER_NOT_FOUND,
                }
            }
            ProcessAppleEvent => { // (const EventRecord*): message = class, where = id
                let class = self.mac.mem.read_u32(a0 + 2).map_err(|_| String::from("AEProcessAppleEvent: the EventRecord leaves guest RAM"))?;
                let (v, h) = (self.mac.mem.read_u16(a0 + 10).unwrap_or(0), self.mac.mem.read_u16(a0 + 12).unwrap_or(0));
                let id = (u32::from(v) << 16) | u32::from(h);
                let found = self.rt.ae.handlers.get(&(class, id, false)).or_else(|| self.rt.ae.handlers.get(&(class, id, true))).copied();
                let Some((tv, refcon)) = found else {
                    self.mac.log.push(format!("[ae] AEProcessAppleEvent '{}'/'{}': no handler", String::from_utf8_lossy(&class.to_be_bytes()), String::from_utf8_lossy(&id.to_be_bytes())));
                    cpu.gpr[3] = ERR_AE_EVENT_NOT_HANDLED as u32;
                    return Ok(());
                };
                // the event (an empty 'aevt' descriptor) and the reply (the null descriptor), in guest memory for the handler
                let blk = self.mac.mm.new_ptr(&mut self.mac.mem, &mut self.mac.heap, 16, true).map_err(|e| format!("AEProcessAppleEvent: no memory (memory error {e})"))?;
                let h = self.mac.mm.new_handle(&mut self.mac.mem, &mut self.mac.heap, 0, true).map_err(|e| format!("AEProcessAppleEvent: no memory (memory error {e})"))?;
                self.mac.mem.write_u32(blk, fc(b"aevt")).and_then(|_| self.mac.mem.write_u32(blk + 4, h)).and_then(|_| self.mac.mem.write_u32(blk + 8, fc(b"null"))).map_err(|_| String::from("AEProcessAppleEvent: guest RAM"))?;
                let frame = Frame { saved: Saved::of(cpu, ret), cont: Continuation::AeHandler };
                let sp = cpu.gpr[1];
                self.enter_tvector(cpu, tv, &[blk, blk + 8, refcon], sp).map_err(|e| format!("AEProcessAppleEvent: the handler: {e}"))?;
                self.rt.callbacks.push(frame);
                return Ok(());
            }
            DisposeDesc => { // (AEDesc*): free the handle, make it the null descriptor
                let h = self.mac.mem.read_u32(a0 + 4).unwrap_or(0);
                if h != 0 { let _ = self.mac.dispose_handle(h); }
                self.mac.mem.write_u32(a0, fc(b"null")).and_then(|_| self.mac.mem.write_u32(a0 + 4, 0)).map_err(|_| String::from("AEDisposeDesc: the descriptor leaves guest RAM"))?;
                0
            }
            GetParam => { self.mac.log.push(String::from("[ae] a parameter/attribute/item was asked of an event that carries none: errAEDescNotFound")); ERR_AE_DESC_NOT_FOUND }
            CountItems => { self.mac.log.push(String::from("[ae] AECountItems of a descriptor that is not a list: errAENotAList")); ERR_AE_NOT_A_LIST }
            SetInteractionAllowed => { self.rt.ae.interaction = a0; 0 }
            GetInteractionAllowed => { let v = self.rt.ae.interaction; self.put32(a0, v, "AEGetInteractionAllowed")?; 0 }
        };
        cpu.gpr[3] = r as u32;
        Ok(())
    }

    /// The handler returned: its OSErr is `AEProcessAppleEvent`'s result.
    pub(crate) fn ae_handler_returned(&mut self, cpu: &mut Cpu, frame: Frame) -> Result<(), String> {
        let r = cpu.gpr[3];
        frame.saved.restore(cpu);
        cpu.gpr[3] = r as u16 as i16 as i32 as u32; // an OSErr is 16 bits
        Ok(())
    }
}
