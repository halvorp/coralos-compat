//! The Apple Event Manager's handler tables (`_Pack8`, $A816; Inside Macintosh: Interapplication Communication, chapter 3).
//!
//! Every Apple Event routine is reached through `_Pack8` with the selector in D0.W as `paramWords << 8 | selector`, so the frame
//! size of ANY selector is known from D0 alone (the high byte counts the 16-bit words of arguments; the OSErr result slot lies
//! above them). That is what lets an unmodelled selector be refused by name WITHOUT corrupting the stack: its frame is popped
//! and the call answers `paramErr`. A program that installs handlers and is never sent an event does not need more than the
//! registry, so only the registry is modelled:
//!
//! * `AEInstallEventHandler` (0x091F), `AERemoveEventHandler` (0x0720), `AEGetEventHandler` (0x0921): the (class, id, isSysHandler)
//!   -> (handler, refcon) table. Installing over an existing entry replaces it; removing or getting a missing one is
//!   `errAEHandlerNotFound` (-1717). Facts: the selectors and frames from the Universal Interfaces (`AppleEvents.h`), the codes
//!   from `AEDataModel.h`.
//!
//! **The one event the host sends: `'aevt'/'oapp'`.** A program that installs an `oapp` handler is the program the Finder
//! just launched, and the Finder's first act is to send it "open application"; so the first `AEInstallEventHandler` for
//! `'aevt'/'oapp'` queues a high-level event (`what` 23, `message` = the class, `where` = the event id), which a
//! `GetNextEvent` / `WaitNextEvent` mask with `highLevelEventMask` (0x0400) returns. The program hands it to
//! `AEProcessAppleEvent` (0x021B), which looks the handler up and RUNS IT IN THE GUEST as the Pascal function
//! `handler(theAppleEvent, reply, refcon): OSErr`: the stack is rebuilt for that call, the trampoline catches the return, and
//! its OSErr becomes `AEProcessAppleEvent`'s. The event is an empty one (no parameters: `AEGetParamPtr` finds none), the reply
//! is the null descriptor (no reply wanted). An event with no handler is `errAEEventNotHandled` (-1708).
//!
//! `AESetInteractionAllowed` (0x011E) / `AEGetInteractionAllowed` (0x021D) keep the process's interaction level (default
//! `kAEInteractWithLocal`); no event is ever sent, so nothing consults it yet.
//!
//! `AEDisposeDesc` (0x0204) is modelled (it frees the descriptor's handle and nulls it); building, sending and reading events
//! (`AECreateAppleEvent`, `AESend`, `AEGetParamPtr`, lists) answer `paramErr` with a named refusal and pop their frame.
use crate::machine::Mac;
use crate::osconv::Regs;
use crate::Refusal;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

pub const ERR_AE_HANDLER_NOT_FOUND: i16 = -1717;
const PARAM_ERR: i16 = -50;

mod sel {
    pub const INSTALL_EVENT_HANDLER: u16 = 0x091F;
    pub const REMOVE_EVENT_HANDLER: u16 = 0x0720;
    pub const GET_EVENT_HANDLER: u16 = 0x0921;
    pub const PROCESS_APPLE_EVENT: u16 = 0x021B;
    pub const DISPOSE_DESC: u16 = 0x0204;
    pub const SET_INTERACTION_ALLOWED: u16 = 0x011E;
    pub const GET_INTERACTION_ALLOWED: u16 = 0x021D;
}

pub const ERR_AE_EVENT_NOT_HANDLED: i16 = -1708;
/// `kHighLevelEvent`: the `what` of an Apple Event in the Event Manager.
pub const HIGH_LEVEL_EVENT: u16 = 23;
const FOUR_AEVT: u32 = u32::from_be_bytes(*b"aevt");
const FOUR_OAPP: u32 = u32::from_be_bytes(*b"oapp");
const FOUR_NULL: u32 = u32::from_be_bytes(*b"null");

/// (event class, event id, isSysHandler) -> (handler address, refcon).
#[derive(Default)]
pub struct AppleEvents { pub handlers: BTreeMap<(u32, u32, bool), (u32, u32)>, oapp_sent: bool, interaction: Option<u16> }

/// `kAEInteractWithLocal`: the level a process starts with (Inside Macintosh: Interapplication Communication, "Apple Event Manager").
const INTERACT_WITH_LOCAL: u16 = 1;

/// What the trampoline needs when a handler returns: the program's stack and PC to resume at, the descriptors to free.
#[derive(Debug, Clone)]
pub(crate) struct AeDone { orig_sp: u32, final_pc: u32, descs: u32, event_data: u32 }

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> { m.register_os(0xA816, pack8::<U>) }

fn tag(v: u32) -> String { v.to_be_bytes().iter().map(|&b| if (0x20..0x7F).contains(&b) { char::from(b) } else { '?' }).collect() }

fn pack8<U: GuestUi>(m: &mut Mac<U>, c: &mut Cpu, r: &mut Regs) -> Result<String, Refusal> {
    let selector = r.d0 as u16;
    let words = u32::from(selector >> 8);
    let sp = c.areg(7);
    let e = Refusal::mem("Apple Event frame");
    // the frame: `words` of arguments (the LAST nearest sp), then the OSErr result slot
    let arg_long = |m: &Mac<U>, from_top: u32| m.mem.read_u32(sp + from_top);
    let result_at = sp + words * 2;
    let (err, log) = match selector {
        sel::INSTALL_EVENT_HANDLER => {
            // (class, id, handler, refcon, isSys): sp+0 isSys (2), +2 refcon, +6 handler, +10 id, +14 class
            let (sys, refcon, handler, id, class) = (m.mem.read_u8(sp).map_err(&e)? != 0, arg_long(m, 2).map_err(&e)?, arg_long(m, 6).map_err(&e)?, arg_long(m, 10).map_err(&e)?, arg_long(m, 14).map_err(&e)?);
            m.ae.handlers.insert((class, id, sys), (handler, refcon));
            if (class, id) == (FOUR_AEVT, FOUR_OAPP) && !m.ae.oapp_sent {
                m.ae.oapp_sent = true;
                m.post_with(HIGH_LEVEL_EVENT, FOUR_AEVT, (FOUR_OAPP >> 16) as i16, FOUR_OAPP as i16, 0);
            }
            (0, format!("[ae] AEInstallEventHandler '{}' '{}'{} -> {handler:#x}", tag(class), tag(id), if sys { " (system)" } else { "" }))
        }
        sel::REMOVE_EVENT_HANDLER => {
            // (class, id, handler, isSys): +0 isSys, +2 handler, +6 id, +10 class
            let (sys, id, class) = (m.mem.read_u8(sp).map_err(&e)? != 0, arg_long(m, 6).map_err(&e)?, arg_long(m, 10).map_err(&e)?);
            let had = m.ae.handlers.remove(&(class, id, sys)).is_some();
            (if had { 0 } else { ERR_AE_HANDLER_NOT_FOUND }, format!("[ae] AERemoveEventHandler '{}' '{}'", tag(class), tag(id)))
        }
        sel::GET_EVENT_HANDLER => {
            // (class, id, VAR handler, VAR refcon, isSys): +0 isSys, +2 refcon ptr, +6 handler ptr, +10 id, +14 class
            let (sys, rp, hp, id, class) = (m.mem.read_u8(sp).map_err(&e)? != 0, arg_long(m, 2).map_err(&e)?, arg_long(m, 6).map_err(&e)?, arg_long(m, 10).map_err(&e)?, arg_long(m, 14).map_err(&e)?);
            match m.ae.handlers.get(&(class, id, sys)).copied() {
                Some((h, rc)) => {
                    if hp != 0 { m.mem.write_u32(hp, h).map_err(&e)?; }
                    if rp != 0 { m.mem.write_u32(rp, rc).map_err(&e)?; }
                    (0, format!("[ae] AEGetEventHandler '{}' '{}' -> {h:#x}", tag(class), tag(id)))
                }
                None => (ERR_AE_HANDLER_NOT_FOUND, format!("[ae] AEGetEventHandler '{}' '{}': none installed", tag(class), tag(id))),
            }
        }
        sel::DISPOSE_DESC => {
            let d = m.mem.read_u32(sp).map_err(&e)?;
            let handle = m.mem.read_u32(d + 4).map_err(&e)?;
            if handle != 0 { let _ = m.dispose_handle(handle); }
            m.mem.write_u32(d, FOUR_NULL).and_then(|_| m.mem.write_u32(d + 4, 0)).map_err(&e)?;
            (0, format!("[ae] AEDisposeDesc {d:#x}"))
        }
        sel::SET_INTERACTION_ALLOWED => {
            // AESetInteractionAllowed(level): the level is the process's policy for AEInteractWithUser; kAEInteractWithSelf 0, Local 1, All 2
            let level = u16::from(m.mem.read_u8(sp).map_err(&e)?); // AEInteractAllowed is a byte: in a 16-bit slot it is the first (low-address) byte
            if level > 2 { (PARAM_ERR, format!("[ae] AESetInteractionAllowed {level}: not an AEInteractAllowed level")) }
            else { m.ae.interaction = Some(level); (0, format!("[ae] AESetInteractionAllowed {level}")) }
        }
        sel::GET_INTERACTION_ALLOWED => {
            let (out, level) = (m.mem.read_u32(sp).map_err(&e)?, m.ae.interaction.unwrap_or(INTERACT_WITH_LOCAL));
            m.mem.write_u8(out, level as u8).map_err(&e)?;
            (0, format!("[ae] AEGetInteractionAllowed -> {level}"))
        }
        sel::PROCESS_APPLE_EVENT => return process_apple_event(m, c, r, sp),
        other => {
            m.note_refusal("Pack8", format!("Apple Event selector {other:#06x} is not modelled (no event is ever delivered to a program here); the {words}-word frame is popped and the call answers paramErr"));
            (PARAM_ERR, format!("[ae] selector {other:#06x} REFUSED"))
        }
    };
    m.mem.write_u16(result_at, err as u16).map_err(&e)?;
    c.set_areg(7, result_at);
    r.d0 = 0;
    Ok(format!("{log} -> {err}"))
}

/// `AEProcessAppleEvent(theEventRecord)`: find the handler and run it in the guest (see the module comment for the stack).
fn process_apple_event<U: GuestUi>(m: &mut Mac<U>, c: &mut Cpu, r: &mut Regs, sp: u32) -> Result<String, Refusal> {
    let e = Refusal::mem("AEProcessAppleEvent");
    let ev = m.mem.read_u32(sp).map_err(&e)?;
    let (what, class) = (m.mem.read_u16(ev).map_err(&e)?, m.mem.read_u32(ev + 2).map_err(&e)?);
    let id = (u32::from(m.mem.read_u16(ev + 10).map_err(&e)?) << 16) | u32::from(m.mem.read_u16(ev + 12).map_err(&e)?);
    let result_at = sp + 4;
    let found = if what == HIGH_LEVEL_EVENT { m.ae.handlers.get(&(class, id, false)).or_else(|| m.ae.handlers.get(&(class, id, true))).copied() } else { None };
    let Some((handler, refcon)) = found else {
        m.mem.write_u16(result_at, ERR_AE_EVENT_NOT_HANDLED as u16).map_err(&e)?;
        c.set_areg(7, result_at);
        r.d0 = 0;
        return Ok(format!("[ae] AEProcessAppleEvent what {what} '{}' '{}': no handler -> errAEEventNotHandled", tag(class), tag(id)));
    };
    // The event (an empty 'aevt' record: class and id) and the two descriptors the handler is given.
    let data = m.mm.new_handle(&mut m.mem, &mut m.heap, 8, false).map_err(|x| Refusal(format!("AEProcessAppleEvent: no memory (memFullErr {x})")))?;
    let dp = m.mm.block(data).map(|b| b.ptr).ok_or_else(|| Refusal("AEProcessAppleEvent: the event has no block".into()))?;
    m.mem.write_u32(dp, class).and_then(|_| m.mem.write_u32(dp + 4, id)).map_err(&e)?;
    let descs = m.mm.new_ptr(&mut m.mem, &mut m.heap, 16, true).map_err(|x| Refusal(format!("AEProcessAppleEvent: no memory (memFullErr {x})")))?;
    m.mem.write_u32(descs, FOUR_AEVT).and_then(|_| m.mem.write_u32(descs + 4, data)).and_then(|_| m.mem.write_u32(descs + 8, FOUR_NULL)).and_then(|_| m.mem.write_u32(descs + 12, 0)).map_err(&e)?;
    // [TRAMPOLINE][refcon][reply][aevt][result 2 bytes] below the program's frame; the handler pops all but the result.
    let mut at = sp;
    at -= 2; m.mem.write_u16(at, 0).map_err(&e)?;
    for v in [descs, descs + 8, refcon, crate::guestcall::TRAMPOLINE] { at -= 4; m.mem.write_u32(at, v).map_err(&e)?; }
    let pc = m.trap_pc;
    m.call_stack.push(crate::guestcall::Frame::Ae(AeDone { orig_sp: sp, final_pc: pc.wrapping_add(2), descs, event_data: data }));
    c.set_areg(7, at);
    c.set_pc(m.mem.strip(handler));
    r.d0 = 0;
    Ok(format!("[ae] AEProcessAppleEvent '{}' '{}': handler {handler:#x} runs in the guest (refcon {refcon:#x})", tag(class), tag(id)))
}

impl<U: GuestUi> Mac<U> {
    /// The handler returned: its OSErr becomes `AEProcessAppleEvent`'s, the program's frame is popped, the descriptors are freed.
    pub(crate) fn finish_ae(&mut self, cpu: &mut Cpu, d: AeDone) {
        let err = self.mem.read_u16(cpu.areg(7)).unwrap_or(PARAM_ERR as u16);
        let result_at = d.orig_sp + 4;
        let _ = self.mem.write_u16(result_at, err);
        // RTS (the instruction after the trampoline's ILLEGAL) returns to the program at the word after its trap
        let top = result_at - 4;
        let _ = self.mem.write_u32(top, d.final_pc);
        cpu.set_areg(7, top);
        let _ = self.dispose_handle(d.event_data);
        let _ = self.mm.dispose_ptr(&mut self.heap, d.descs);
        self.log.push(format!("[ae] the handler returned {}", err as i16));
    }
}
