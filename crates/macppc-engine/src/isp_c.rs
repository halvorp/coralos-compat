//! InputSprocket (`InputSprocketLib`) as a native library over the host's keyboard and mouse ONLY (no hardware enumeration beyond them), from the public API contract and the structure
//! facts confirmed by systemless (GPL, facts only): `ISpElementInfo` is 80 bytes (label @0, kind @4, a Str63 name @8, two reserved longs), `ISpElementEvent` is 20 bytes (an
//! `AbsoluteTime` `when` @0, `element` @8, `refCon` @12, `data` @16), `ISpGetVersion` returns a `NumVersion` through the hidden result pointer in r3 (1.7).
//! * Two devices: the keyboard (class `'keyd'`; one button element for "any key") and the mouse (class `'mous'`; X and Y delta elements and one button). `ISpDevices_ExtractByClass`
//!   counts the devices of a class (0 = all) and copies up to the buffer; `ISpDevices_ActivateClass`, `ISpSuspend`, `ISpResume` succeed (nothing to switch).
//! * Element lists hold element references (`ISpElementList_New/AddElements/Extract/Dispose`, `ISpDevice_GetElementList` makes a fresh list of the device's elements).
//! * `ISpElementList_GetNextEvent` polls the mouse the way the Event Manager's polling calls do (`Mouse` and `MBState` in low memory): the first call on a list only records the state;
//!   after that a change of the button or a non-zero movement is one event (button data 1 = pressed / 0 = released, delta data = the signed movement), oldest first, one per call.
//!   NAMED LIMIT: the keyboard element never produces events or state here (keys reach the game as ordinary key events through the Event Manager); `ISpElement_GetSimpleState` of the
//!   keyboard is 0 and of a delta element is 0.
//! * `ISpElement_GetConfigurationInfo`: a button has one UInt32 (its id; the mouse button 1, the keyboard 0), a delta two reserved UInt32s.
//! Errors are `paramErr` (-50) for a reference or buffer that is not one of these. Not provided, so unbound and stopping by name: virtual elements (`ISpElement_NewVirtual*`), the
//! configuration dialog, device definitions.
use crate::clib::arg;
use crate::machine::MacPpc;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use guestui_abi::GuestUi;
use retro_ppc::Cpu;

pub const NAME: &str = "InputSprocketLib";
const KEYBOARD_DEVICE: u32 = 0x4000_2000;
const MOUSE_DEVICE: u32 = 0x4000_2010;
const KEYBOARD_ELEMENT: u32 = 0x4000_2020;
const MOUSE_X: u32 = 0x4000_2030;
const MOUSE_Y: u32 = 0x4000_2040;
const MOUSE_BUTTON: u32 = 0x4000_2050;
const LIST_BASE: u32 = 0x4001_0000;
const CLASS_KEYBOARD: u32 = u32::from_be_bytes(*b"keyd");
const CLASS_MOUSE: u32 = u32::from_be_bytes(*b"mous");
const KIND_BUTTON: u32 = u32::from_be_bytes(*b"butn");
const KIND_DELTA: u32 = u32::from_be_bytes(*b"delt");
const PARAM_ERR: i32 = -50;
const VERSION_1_7: u32 = 0x0170_8000;
const EVENT_SIZE: u32 = 20;
const INFO_SIZE: u32 = 80;

#[derive(Default)]
struct List { elements: Vec<u32>, refcon: u32, last: Option<(i16, i16, bool)>, pending: Vec<(u32, u32)> }
#[derive(Default)]
pub struct IspState { started: bool, lists: BTreeMap<u32, List>, next: u32 }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IspFn {
    Startup, Shutdown, GetVersion, Suspend, Resume, DevicesExtractByClass, DevicesActivateClass, DeviceGetElementList,
    ElementListNew, ElementListAddElements, ElementListExtract, ElementListDispose, ElementListGetNextEvent, ElementGetInfo, ElementGetSimpleState, ElementGetConfigurationInfo,
}

impl IspFn {
    pub fn by_name(name: &str) -> Option<IspFn> {
        use IspFn::*;
        Some(match name {
            "ISpStartup" => Startup, "ISpShutdown" => Shutdown, "ISpGetVersion" => GetVersion, "ISpSuspend" => Suspend, "ISpResume" => Resume,
            "ISpDevices_ExtractByClass" => DevicesExtractByClass, "ISpDevices_ActivateClass" => DevicesActivateClass, "ISpDevice_GetElementList" => DeviceGetElementList,
            "ISpElementList_New" => ElementListNew, "ISpElementList_AddElements" => ElementListAddElements, "ISpElementList_Extract" => ElementListExtract,
            "ISpElementList_Dispose" => ElementListDispose, "ISpElementList_GetNextEvent" => ElementListGetNextEvent, "ISpElement_GetInfo" => ElementGetInfo,
            "ISpElement_GetSimpleState" => ElementGetSimpleState, "ISpElement_GetConfigurationInfo" => ElementGetConfigurationInfo,
            _ => return None,
        })
    }
}

impl<U: GuestUi> MacPpc<U> {
    fn isp_put32(&mut self, at: u32, v: u32) -> bool { at != 0 && self.mac.mem.write_u32(at, v).is_ok() }
    fn isp_mouse(&self) -> (i16, i16, bool) {
        let m = &self.mac.mem;
        (m.read_i16(mac68k_engine::lowmem::MOUSE).unwrap_or(0), m.read_i16(mac68k_engine::lowmem::MOUSE + 2).unwrap_or(0), m.read_u8(mac68k_engine::lowmem::MB_STATE).is_ok_and(|b| b & 0x80 == 0))
    }
    fn isp_new_list(&mut self, elements: Vec<u32>) -> u32 {
        self.isp.next += 1;
        let id = LIST_BASE + 0x10 * self.isp.next;
        self.isp.lists.insert(id, List { elements, ..Default::default() });
        id
    }

    pub(crate) fn isp_call(&mut self, f: IspFn, cpu: &mut Cpu) -> Result<(), String> {
        use IspFn::*;
        let (a0, a1, a2, a3) = (arg(self, cpu, 0)?, arg(self, cpu, 1).unwrap_or(0), arg(self, cpu, 2).unwrap_or(0), arg(self, cpu, 3).unwrap_or(0));
        let n = format!("InputSprocket {f:?}");
        let r: i32 = match f {
            Startup => { self.isp.started = true; 0 }
            Shutdown => { self.isp.lists.clear(); self.isp.started = false; 0 }
            // ISpGetVersion returns a NumVersion: a struct result, written through the hidden pointer in r3 (1.7)
            GetVersion => { self.isp_put32(a0, VERSION_1_7); a0 as i32 }
            Suspend | Resume | DevicesActivateClass => 0,
            // ISpDevices_ExtractByClass(class, bufferCount, UInt32 *outCount, ISpDeviceReference *buffer)
            DevicesExtractByClass => {
                let devs: Vec<u32> = [(CLASS_KEYBOARD, KEYBOARD_DEVICE), (CLASS_MOUSE, MOUSE_DEVICE)].iter().filter(|(c, _)| a0 == 0 || a0 == *c).map(|&(_, d)| d).collect();
                for (i, d) in devs.iter().take(a1 as usize).enumerate() { self.isp_put32(a3 + 4 * i as u32, *d); }
                self.isp_put32(a2, devs.len() as u32);
                0
            }
            // ISpDevice_GetElementList(device, ISpElementListReference *)
            DeviceGetElementList => match a0 {
                KEYBOARD_DEVICE => { let l = self.isp_new_list(alloc::vec![KEYBOARD_ELEMENT]); self.isp_put32(a1, l); 0 }
                MOUSE_DEVICE => { let l = self.isp_new_list(alloc::vec![MOUSE_X, MOUSE_Y, MOUSE_BUTTON]); self.isp_put32(a1, l); 0 }
                _ => PARAM_ERR,
            },
            // ISpElementList_New(count, ISpElementReference *elements, ISpElementListReference *out, flags)
            ElementListNew => {
                let mut els = Vec::new();
                for i in 0..a0 { els.push(self.mac.mem.read_u32(a1 + 4 * i).map_err(|_| format!("{n}: the element array leaves guest RAM"))?); }
                if els.iter().any(|e| !matches!(*e, KEYBOARD_ELEMENT | MOUSE_X | MOUSE_Y | MOUSE_BUTTON)) { PARAM_ERR } else { let l = self.isp_new_list(els); self.isp_put32(a2, l); 0 }
            }
            // ISpElementList_AddElements(list, refCon, count, elements)
            ElementListAddElements => {
                let mut els = Vec::new();
                for i in 0..a2 { els.push(self.mac.mem.read_u32(a3 + 4 * i).map_err(|_| format!("{n}: the element array leaves guest RAM"))?); }
                match self.isp.lists.get_mut(&a0) {
                    Some(l) => { l.refcon = a1; for e in els { if !l.elements.contains(&e) { l.elements.push(e); } } 0 }
                    None => PARAM_ERR,
                }
            }
            // ISpElementList_Extract(list, bufferCount, UInt32 *outCount, ISpElementReference *buffer)
            ElementListExtract => match self.isp.lists.get(&a0).map(|l| l.elements.clone()) {
                Some(els) => { for (i, e) in els.iter().take(a1 as usize).enumerate() { self.isp_put32(a3 + 4 * i as u32, *e); } self.isp_put32(a2, els.len() as u32); 0 }
                None => PARAM_ERR,
            },
            ElementListDispose => if self.isp.lists.remove(&a0).is_some() { 0 } else { PARAM_ERR },
            // ISpElementList_GetNextEvent(list, bufferSize, ISpElementEvent *, Boolean *gotEvent)
            ElementListGetNextEvent => {
                if a1 < EVENT_SIZE || a2 == 0 { PARAM_ERR } else {
                    let now = self.isp_mouse();
                    let ticks = self.mac.ev.ticks_total;
                    match self.isp.lists.get_mut(&a0) {
                        None => PARAM_ERR,
                        Some(l) => {
                            if let Some((pv, ph, pb)) = l.last {
                                let has = |e| l.elements.contains(&e);
                                if has(MOUSE_X) && now.1 != ph { l.pending.push((MOUSE_X, (i32::from(now.1) - i32::from(ph)) as u32)); }
                                if has(MOUSE_Y) && now.0 != pv { l.pending.push((MOUSE_Y, (i32::from(now.0) - i32::from(pv)) as u32)); }
                                if has(MOUSE_BUTTON) && now.2 != pb { l.pending.push((MOUSE_BUTTON, u32::from(now.2))); }
                            }
                            l.last = Some(now);
                            let refcon = l.refcon;
                            let ev = if l.pending.is_empty() { None } else { Some(l.pending.remove(0)) };
                            match ev {
                                None => { self.mac.mem.write_u8(a3, 0).ok(); }
                                Some((el, data)) => {
                                    let ns = ticks * 1_000_000_000 / 60;
                                    for (o, v) in [(0u32, (ns >> 32) as u32), (4, ns as u32), (8, el), (12, refcon), (16, data)] { self.isp_put32(a2 + o, v); }
                                    self.mac.mem.write_u8(a3, 1).ok();
                                }
                            }
                            0
                        }
                    }
                }
            }
            ElementGetInfo => {
                let (label, kind, name): (&[u8; 4], u32, &[u8]) = match a0 {
                    KEYBOARD_ELEMENT => (b"none", KIND_BUTTON, b"Keyboard Key"),
                    MOUSE_X => (b"curx", KIND_DELTA, b"Mouse X Delta"),
                    MOUSE_Y => (b"cury", KIND_DELTA, b"Mouse Y Delta"),
                    MOUSE_BUTTON => (b"mou1", KIND_BUTTON, b"Mouse Button"),
                    _ => { cpu.gpr[3] = PARAM_ERR as u32; return Ok(()); }
                };
                if a1 == 0 { PARAM_ERR } else {
                    let mut b = alloc::vec![0u8; INFO_SIZE as usize];
                    b[0..4].copy_from_slice(label); b[4..8].copy_from_slice(&kind.to_be_bytes()); b[8] = name.len() as u8; b[9..9 + name.len()].copy_from_slice(name);
                    self.put_bytes(a1, &b, &n)?;
                    0
                }
            }
            ElementGetSimpleState => match a0 {
                MOUSE_BUTTON => { let down = self.isp_mouse().2; self.isp_put32(a1, u32::from(down)); 0 }
                KEYBOARD_ELEMENT | MOUSE_X | MOUSE_Y => { self.isp_put32(a1, 0); 0 }
                _ => PARAM_ERR,
            },
            // ISpElement_GetConfigurationInfo(element, bufferSize, void *): a button has one UInt32 (its id), a delta two reserved UInt32s
            ElementGetConfigurationInfo => {
                let (size, words): (u32, &[u32]) = match a0 { KEYBOARD_ELEMENT => (4, &[0]), MOUSE_BUTTON => (4, &[1]), MOUSE_X | MOUSE_Y => (8, &[0, 0]), _ => (0, &[]) };
                if size == 0 || a2 == 0 || a1 < size { PARAM_ERR } else { for (i, w) in words.iter().enumerate() { self.isp_put32(a2 + 4 * i as u32, *w); } 0 }
            }
        };
        cpu.gpr[3] = r as u32;
        Ok(())
    }
}
