//! The Apple Event Manager for PowerPC programs (Inside Macintosh: Interapplication Communication; `AppleEvents.h`, `AEDataModel.h`).
//!
//! * The DATA MODEL - descriptors, lists, records, events, attributes, parameters, coercion - is the shared Mac semantics in
//!   [`heirloom_mac68k_engine::aedesc`]; the calls here only move the PowerPC arguments (r3.., then the caller's parameter area).
//! * HANDLERS: a table per process (`AEInstallEventHandler`, `AERemoveEventHandler`, `AEGetEventHandler`); system handlers (isSysHandler)
//!   are one table for every process. Lookup order for an event (class, id): exact, (class, '****'), ('****', id), ('****', '****'), first in the
//!   process's table, then the system table.
//! * The Finder's first act on launching a program is to send it "open application": the first `AEInstallEventHandler('aevt','oapp')` of a
//!   process queues that event for it.
//! * DELIVERY between processes ([`crate::process_c`]): `AESend` resolves the event's `keyAddressAttr` (typeApplSignature 'sign' = the process
//!   with that creator, typeProcessSerialNumber 'psn ', kCurrentProcess = the sender), copies the event into the target's queue of high-level
//!   events (its `addr` attribute becomes the sender's serial number) and, for kAEWaitReply, the SENDER'S TASK WAITS (the scheduler runs the
//!   target) until the target's handler returns or the timeout passes (kAEDefaultTimeout -1 = 3600 ticks, kNoTimeOut -2 = forever;
//!   errAETimeout -1712). kAENoReply returns at once; kAEQueueReply returns at once and the reply comes later as an 'aevt'/'ansr' event.
//!   The target fetches the event with WaitNextEvent (kHighLevelEvent, message = class, where = id) and hands it to `AEProcessAppleEvent`,
//!   which runs the handler (the program's own code, through the callback machine) with the event and a reply ('aevt'/'ansr' when a reply
//!   is wanted, else the null descriptor). When the handler returns, a nonzero OSErr adds keyErrorNumber 'errn' to the reply (unless the
//!   handler put one); the reply is copied into the sender's reply descriptor and the sender is woken. With no handler the reply carries
//!   errAEEventNotHandled (-1708) and AESend returns it. A target that ends first answers connectionInvalid (-609).
//!   Sending to oneself runs the handler directly. The idle and filter procs of a waiting AESend are not called (the other processes run
//!   instead; logged).
use crate::callback::{Continuation, Frame, Saved};
use crate::clib::arg;
use crate::machine::MacPpc;
use crate::sched::{Wait, WaitOn};
use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_mac68k_engine::aedesc::{self as ad, fc, tag, AeErr, Blob, Entry, Kind};
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

pub const ERR_AE_EVENT_NOT_HANDLED: i32 = -1708;
pub const ERR_AE_UNKNOWN_SEND_MODE: i32 = -1710;
pub const ERR_AE_TIMEOUT: i32 = -1712;
pub const ERR_AE_NO_USER_INTERACTION: i32 = -1713;
pub const ERR_AE_UNKNOWN_ADDRESS_TYPE: i32 = -1716;
pub const ERR_AE_HANDLER_NOT_FOUND: i32 = -1717;
const PROC_NOT_FOUND: i32 = -600;
/// kAEDefaultTimeout: "about one minute".
const DEFAULT_TIMEOUT_TICKS: u32 = 3600;
const NO_REPLY: u32 = 1;
const QUEUE_REPLY: u32 = 2;
const WAIT_REPLY: u32 = 3;
const AEVT: u32 = fc(b"aevt");
const ANSR: u32 = fc(b"ansr");
const WILD: u32 = ad::TYPE_WILD_CARD;
const TYPE_PSN: u32 = fc(b"psn ");
const TYPE_SIGN: u32 = fc(b"sign");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AeFn {
    InstallEventHandler, RemoveEventHandler, GetEventHandler, ProcessAppleEvent, SetInteractionAllowed, GetInteractionAllowed, InteractWithUser,
    CreateDesc, DisposeDesc, DuplicateDesc, CreateList, CountItems, PutPtr, PutDesc, GetNthPtr, GetNthDesc, SizeOfNthItem, DeleteItem,
    PutParamPtr, PutParamDesc, GetParamPtr, GetParamDesc, SizeOfParam, DeleteParam,
    PutAttributePtr, PutAttributeDesc, GetAttributePtr, GetAttributeDesc, SizeOfAttribute,
    CoerceDesc, CoercePtr, CreateAppleEvent, Send, GetDescDataSize, GetDescData, ReplaceDescData,
}
impl AeFn {
    pub fn by_name(name: &str) -> Option<AeFn> {
        use AeFn::*;
        Some(match name {
            "AEInstallEventHandler" => InstallEventHandler, "AERemoveEventHandler" => RemoveEventHandler, "AEGetEventHandler" => GetEventHandler,
            "AEProcessAppleEvent" => ProcessAppleEvent, "AESetInteractionAllowed" => SetInteractionAllowed, "AEGetInteractionAllowed" => GetInteractionAllowed,
            "AEInteractWithUser" => InteractWithUser,
            "AECreateDesc" => CreateDesc, "AEDisposeDesc" => DisposeDesc, "AEDuplicateDesc" => DuplicateDesc, "AECreateList" => CreateList,
            "AECountItems" => CountItems, "AEPutPtr" => PutPtr, "AEPutDesc" => PutDesc, "AEGetNthPtr" => GetNthPtr, "AEGetNthDesc" => GetNthDesc,
            "AESizeOfNthItem" => SizeOfNthItem, "AEDeleteItem" => DeleteItem,
            "AEPutParamPtr" | "AEPutKeyPtr" => PutParamPtr, "AEPutParamDesc" | "AEPutKeyDesc" => PutParamDesc,
            "AEGetParamPtr" | "AEGetKeyPtr" => GetParamPtr, "AEGetParamDesc" | "AEGetKeyDesc" => GetParamDesc,
            "AESizeOfParam" | "AESizeOfKeyDesc" => SizeOfParam, "AEDeleteParam" | "AEDeleteKeyDesc" => DeleteParam,
            "AEPutAttributePtr" => PutAttributePtr, "AEPutAttributeDesc" => PutAttributeDesc, "AEGetAttributePtr" => GetAttributePtr,
            "AEGetAttributeDesc" => GetAttributeDesc, "AESizeOfAttribute" => SizeOfAttribute,
            "AECoerceDesc" => CoerceDesc, "AECoercePtr" => CoercePtr, "AECreateAppleEvent" => CreateAppleEvent, "AESend" => Send,
            "AEGetDescDataSize" => GetDescDataSize, "AEGetDescData" => GetDescData, "AEReplaceDescData" => ReplaceDescData,
            _ => return None,
        })
    }
}

/// A high-level event waiting for (or just delivered to) a process.
#[derive(Debug, Clone)]
pub(crate) struct Hle { pub class: u32, pub id: u32, pub event: Vec<u8>, pub txn: Option<u32> }

/// An AESend that wants a reply.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Txn { sender_task: u32, sender_psn: u32, target_psn: u32, reply_at: u32, mode: u32 }

/// A handler running: its guest block (event desc at +0, reply desc at +8) and where its result goes.
#[derive(Debug, Clone, Copy)]
struct InFlight { block: u32, txn: Option<u32>, /// AESend to oneself: (the sender's reply descriptor, send mode)
    direct: Option<(u32, u32)> }

#[derive(Default)]
pub(crate) struct AeState {
    /// (process serial low word, 0 for a system handler; class, id) -> (handler TVector, refcon).
    handlers: BTreeMap<(u32, u32, u32), (u32, u32)>,
    oapp_sent: BTreeSet<u32>,
    interaction: BTreeMap<u32, u32>,
    pub(crate) queues: BTreeMap<u32, VecDeque<Hle>>,
    pub(crate) delivered: BTreeMap<u32, Hle>,
    pub(crate) txns: BTreeMap<u32, Txn>,
    next_txn: u32,
    in_flight: Vec<InFlight>,
    idle_noted: bool,
}

fn os(e: AeErr) -> Result<i32, String> { match e { AeErr::Os(e) => Ok(i32::from(e)), AeErr::Fault(w) => Err(w) } }

impl<U: GuestUi> MacPpc<U> {
    fn ae_bytes(&self, p: u32, n: u32, who: &str) -> Result<Vec<u8>, String> {
        if n == 0 { return Ok(Vec::new()) }
        if p == 0 { return Err(format!("{who}: dataPtr is NIL for {n} bytes")) }
        self.mac.mem.span(p, n).and_then(|s| self.mac.mem.bytes(s).map(<[u8]>::to_vec)).map_err(|e| format!("{who}: the data at {p:#x}: {e}"))
    }

    fn ae_handler_for(&self, psn: u32, class: u32, id: u32) -> Option<(u32, u32)> {
        for owner in [psn, 0] {
            for (c, i) in [(class, id), (class, WILD), (WILD, id), (WILD, WILD)] {
                if let Some(&h) = self.rt.ae.handlers.get(&(owner, c, i)) { return Some(h) }
            }
        }
        None
    }

    fn psn_key(&self, sys: bool) -> u32 { if sys { 0 } else { self.mac.current_psn() } }

    pub(crate) fn ae_call(&mut self, f: AeFn, cpu: &mut Cpu) -> Result<(), String> {
        use AeFn::*;
        let a = |m: &Self, i: usize| arg(m, cpu, i);
        let (a0, a1, a2, a3) = (a(self, 0)?, a(self, 1).unwrap_or(0), a(self, 2).unwrap_or(0), a(self, 3).unwrap_or(0));
        let ret = cpu.lr & !3;
        let m = &mut self.mac;
        let r: Result<i32, AeErr> = match f {
            InstallEventHandler => { // (class, id, handler UPP, refcon, isSysHandler)
                let sys = a(self, 4)? & 0xFF != 0;
                let tv = self.proc_tvector(a2)?;
                let key = self.psn_key(sys);
                self.rt.ae.handlers.insert((key, a0, a1), (tv, a3));
                self.mac.log.push(format!("[ae] AEInstallEventHandler '{}'/'{}' -> {tv:#x}{}", tag(a0), tag(a1), if sys { " (system)" } else { "" }));
                let psn = self.mac.current_psn();
                if (a0, a1) == (AEVT, fc(b"oapp")) && self.rt.ae.oapp_sent.insert(psn) {
                    let mut b = Blob::new(Kind::Event);
                    b.attrs.push(Entry { key: ad::KEY_EVENT_CLASS, ty: ad::TYPE_TYPE, structured: false, read: false, data: AEVT.to_be_bytes().to_vec() });
                    b.attrs.push(Entry { key: ad::KEY_EVENT_ID, ty: ad::TYPE_TYPE, structured: false, read: false, data: fc(b"oapp").to_be_bytes().to_vec() });
                    self.rt.ae.queues.entry(psn).or_default().push_back(Hle { class: AEVT, id: fc(b"oapp"), event: b.encode(), txn: None });
                }
                Ok(0)
            }
            RemoveEventHandler => { let key = self.psn_key(a(self, 3)? & 0xFF != 0); Ok(if self.rt.ae.handlers.remove(&(key, a0, a1)).is_some() { 0 } else { ERR_AE_HANDLER_NOT_FOUND }) }
            GetEventHandler => { // (class, id, &handler, &refcon, isSysHandler)
                let key = self.psn_key(a(self, 4)? & 0xFF != 0);
                match self.rt.ae.handlers.get(&(key, a0, a1)).copied() {
                    Some((tv, refcon)) => { self.put32(a2, tv, "AEGetEventHandler")?; self.put32(a3, refcon, "AEGetEventHandler")?; Ok(0) }
                    None => Ok(ERR_AE_HANDLER_NOT_FOUND),
                }
            }
            ProcessAppleEvent => return self.ae_process(a0, cpu, ret),
            SetInteractionAllowed => { let p = self.mac.current_psn(); self.rt.ae.interaction.insert(p, a0); Ok(0) }
            GetInteractionAllowed => { let v = self.rt.ae.interaction.get(&self.mac.current_psn()).copied().unwrap_or(1); self.put32(a0, v, "AEGetInteractionAllowed")?; Ok(0) }
            InteractWithUser => Ok(if self.in_background() { self.mac.log.push(String::from("[ae] AEInteractWithUser in a background process -> errAENoUserInteraction")); ERR_AE_NO_USER_INTERACTION } else { 0 }),
            CreateDesc => { let d = self.ae_bytes(a1, a2, "AECreateDesc")?; self.mac.ae_create_desc(a3, a0, &d).map(|_| 0) }
            DisposeDesc => m.ae_dispose_desc(a0).map(|_| 0),
            DuplicateDesc => m.ae_duplicate_desc(a0, a1).map(|_| 0),
            CreateList => m.ae_create_list(a3, a2 & 0xFF != 0).map(|_| 0),
            CountItems => match m.ae_count_items(a0) { Ok(n) => { self.put32(a1, n, "AECountItems")?; Ok(0) } Err(e) => Err(e) },
            PutPtr => { let d = self.ae_bytes(a3, a(self, 4)?, "AEPutPtr")?; self.mac.ae_put_item(a0, a1 as i32, Entry { key: 0, ty: a2, structured: false, read: false, data: d }).map(|_| 0) }
            PutDesc => match m.ae_entry_of(0, a2) { Ok(e) => m.ae_put_item(a0, a1 as i32, e).map(|_| 0), Err(e) => Err(e) },
            GetNthPtr => { // (list, index, desired, &keyword, &type, data, max, &actual)
                let (data, max, actual) = (a(self, 5)?, a(self, 6)?, a(self, 7)?);
                match self.mac.ae_nth(a0, a1 as i32) {
                    Ok(e) => { self.put32(a3, e.key, "AEGetNthPtr")?; self.mac.ae_copy_out(&e, a2, a(self, 4)?, data, max, actual).map(|_| 0) }
                    Err(e) => Err(e),
                }
            }
            GetNthDesc => match self.mac.ae_nth(a0, a1 as i32) { // (list, index, desired, &keyword, &result)
                Ok(e) => { self.put32(a3, e.key, "AEGetNthDesc")?; let out = a(self, 4)?; self.ae_desc_out(&e, a2, out) }
                Err(e) => Err(e),
            },
            SizeOfNthItem => match self.mac.ae_nth(a0, a1 as i32) { Ok(e) => { self.put32(a2, e.ty, "AESizeOfNthItem")?; self.put32(a3, e.data.len() as u32, "AESizeOfNthItem")?; Ok(0) } Err(e) => Err(e) },
            DeleteItem => m.ae_delete_item(a0, a1 as i32).map(|_| 0),
            PutParamPtr | PutAttributePtr => { let d = self.ae_bytes(a3, a(self, 4)?, "AEPut*Ptr")?; self.mac.ae_put_key(a0, f == PutAttributePtr, Entry { key: a1, ty: a2, structured: false, read: false, data: d }).map(|_| 0) }
            PutParamDesc | PutAttributeDesc => match m.ae_entry_of(a1, a2) { Ok(e) => m.ae_put_key(a0, f == PutAttributeDesc, e).map(|_| 0), Err(e) => Err(e) },
            GetParamPtr | GetAttributePtr => { // (event, key, desired, &type, data, max, &actual)
                let (data, max, actual) = (a(self, 4)?, a(self, 5)?, a(self, 6)?);
                let got = self.mac.ae_get_key(a0, f == GetAttributePtr, a1);
                match got { Ok(e) => self.mac.ae_copy_out(&e, a2, a3, data, max, actual).map(|_| 0), Err(e) => Err(e) }
            }
            GetParamDesc | GetAttributeDesc => match self.mac.ae_get_key(a0, f == GetAttributeDesc, a1) { Ok(e) => self.ae_desc_out(&e, a2, a3), Err(e) => Err(e) },
            SizeOfParam | SizeOfAttribute => {
                let b = self.mac.ae_blob(a0);
                match b.and_then(|b| b.key(f == SizeOfAttribute, a1).cloned().ok_or(AeErr::Os(ad::ERR_AE_DESC_NOT_FOUND))) {
                    Ok(e) => { self.put32(a2, e.ty, "AESizeOf*")?; self.put32(a3, e.data.len() as u32, "AESizeOf*")?; Ok(0) }
                    Err(e) => Err(e),
                }
            }
            DeleteParam => m.ae_delete_key(a0, a1).map(|_| 0),
            CoerceDesc => match m.ae_entry_of(0, a0) { Ok(e) => self.ae_desc_out(&e, a1, a2), Err(e) => Err(e) },
            CoercePtr => { // (type, data, size, toType, &result)
                let d = self.ae_bytes(a1, a2, "AECoercePtr")?;
                let out = a(self, 4)?;
                self.ae_desc_out(&Entry { key: 0, ty: a0, structured: false, read: false, data: d }, a3, out)
            }
            CreateAppleEvent => { // (class, id, target*, returnID (short), transactionID, &result)
                let (txn, out) = (a(self, 4)?, a(self, 5)?);
                self.mac.ae_create_apple_event(out, a0, a1, a2, a3 as u16 as i16, txn).map(|_| 0)
            }
            Send => return self.ae_send(cpu, ret),
            GetDescDataSize => match m.ae_desc_bytes(a0) { Ok(b) => Ok(b.len() as i32), Err(e) => Err(e) },
            GetDescData => match m.ae_desc_bytes(a0) {
                Ok(b) => { let n = (b.len() as u32).min(a2) as usize; if n > 0 { self.mac.mem.write_bytes(a1, &b[..n]).map_err(|e| format!("AEGetDescData: {e}"))?; } Ok(0) }
                Err(e) => Err(e),
            },
            ReplaceDescData => { // (type, data, size, desc*)
                let d = self.ae_bytes(a1, a2, "AEReplaceDescData")?;
                let _ = self.mac.ae_dispose_desc(a3);
                self.mac.ae_create_desc(a3, a0, &d).map(|_| 0)
            }
        };
        let v = match r { Ok(v) => v, Err(e) => os(e)? };
        if v < 0 && v != ERR_AE_HANDLER_NOT_FOUND { self.mac.log.push(format!("[ae] {f:?} -> {v}")); }
        cpu.gpr[3] = v as u32;
        Ok(())
    }

    /// An entry as a new descriptor at `out`, coerced to `desired`.
    fn ae_desc_out(&mut self, e: &Entry, desired: u32, out: u32) -> Result<i32, AeErr> {
        if e.structured {
            if desired != WILD && desired != e.ty && !matches!((desired, e.ty), (ad::TYPE_RECORD, _) | (ad::TYPE_LIST, ad::TYPE_LIST)) { return Err(AeErr::Os(ad::ERR_AE_COERCION_FAIL)) }
            self.mac.ae_desc_from_entry(out, e)?;
            return Ok(0)
        }
        match ad::coerce(e.ty, &e.data, desired) {
            Ok((ty, d)) => { self.mac.ae_create_desc(out, ty, &d)?; Ok(0) }
            Err(err) => { self.mac.log.push(format!("[ae] no coercion '{}' -> '{}'", tag(e.ty), tag(desired))); Err(AeErr::Os(err)) }
        }
    }

    /// `AESend(event, reply, sendMode, priority, timeout, idleProc, filterProc)`.
    fn ae_send(&mut self, cpu: &mut Cpu, ret: u32) -> Result<(), String> {
        let a = |m: &Self, i: usize| arg(m, cpu, i);
        let (ev, reply_at, mode_full, timeout, idle) = (a(self, 0)?, a(self, 1)?, a(self, 2)?, a(self, 4)?, a(self, 5).unwrap_or(0));
        if reply_at != 0 { self.mac.ae_set_desc(reply_at, ad::TYPE_NULL, 0).map_err(|e| format!("AESend: {e:?}"))?; }
        let mode = mode_full & 3;
        let fail = |m: &mut Self, cpu: &mut Cpu, e: i32, why: String| { m.mac.log.push(format!("[ae] AESend: {why} -> {e}")); cpu.gpr[3] = e as u32; Ok(()) };
        if mode == 0 { return fail(self, cpu, ERR_AE_UNKNOWN_SEND_MODE, format!("send mode {mode_full:#x}")) }
        let mut blob = match self.mac.ae_blob(ev) { Ok(b) if b.kind == Kind::Event => b, Ok(_) => return fail(self, cpu, i32::from(ad::ERR_AE_NOT_APPLE_EVENT), String::from("not an Apple event")), Err(e) => { cpu.gpr[3] = os(e)? as u32; return Ok(()) } };
        let (class, id) = match self.mac.ae_class_id(ev) { Ok(v) => v, Err(e) => { cpu.gpr[3] = os(e)? as u32; return Ok(()) } };
        let me = self.mac.current_psn();
        let target = match blob.key(true, ad::KEY_ADDRESS) {
            Some(e) if e.ty == TYPE_SIGN && e.data.len() == 4 => {
                let sig = u32::from_be_bytes([e.data[0], e.data[1], e.data[2], e.data[3]]);
                let own = self.mac.fm.app.map(|n| u32::from_be_bytes(self.mac.fm.tree.get(n).finfo.creator));
                self.mac.procs.by_creator(sig).or_else(|| (self.mac.procs.table.is_empty() && own == Some(sig)).then_some(me))
            }
            Some(e) if e.ty == TYPE_PSN && e.data.len() == 8 => {
                let p = (u32::from_be_bytes([e.data[0], e.data[1], e.data[2], e.data[3]]), u32::from_be_bytes([e.data[4], e.data[5], e.data[6], e.data[7]]));
                self.mac.resolve_psn(p)
            }
            Some(e) => return fail(self, cpu, ERR_AE_UNKNOWN_ADDRESS_TYPE, format!("address type '{}'", tag(e.ty))),
            None => return fail(self, cpu, ERR_AE_UNKNOWN_ADDRESS_TYPE, String::from("the event has no target address")),
        };
        let Some(target) = target else { return fail(self, cpu, PROC_NOT_FOUND, format!("'{}'/'{}': the target process is not running", tag(class), tag(id))) };
        // the receiver sees the sender's serial number as the event's address
        let mut psn = Vec::from(0u32.to_be_bytes()); psn.extend_from_slice(&me.to_be_bytes());
        blob.put_key(true, Entry { key: ad::KEY_ADDRESS, ty: TYPE_PSN, structured: false, read: false, data: psn });
        if target == me {
            // to oneself: the handler runs now (no queue)
            let Some((tv, refcon)) = self.ae_handler_for(me, class, id) else { return fail(self, cpu, ERR_AE_EVENT_NOT_HANDLED, format!("'{}'/'{}' to itself: no handler", tag(class), tag(id))) };
            let block = self.ae_handler_block(&blob, mode != NO_REPLY).map_err(|e| format!("AESend: {e:?}"))?;
            self.rt.ae.in_flight.push(InFlight { block, txn: None, direct: Some((reply_at, mode)) });
            let frame = Frame { saved: Saved::of(cpu, ret), cont: Continuation::AeHandler };
            let sp = cpu.gpr[1];
            self.enter_tvector(cpu, tv, &[block, block + 8, refcon], sp).map_err(|e| format!("AESend: the handler: {e}"))?;
            self.rt.callbacks.push(frame);
            return Ok(());
        }
        let txn = (mode != NO_REPLY).then(|| {
            self.rt.ae.next_txn += 1;
            let t = self.rt.ae.next_txn;
            self.rt.ae.txns.insert(t, Txn { sender_task: self.rt.sched.current, sender_psn: me, target_psn: target, reply_at, mode });
            t
        });
        self.rt.ae.queues.entry(target).or_default().push_back(Hle { class, id, event: blob.encode(), txn });
        self.mac.log.push(format!("[ae] AESend '{}'/'{}' from {{0,{me:#x}}} to {{0,{target:#x}}} mode {mode_full:#x} timeout {}", tag(class), tag(id), timeout as i32));
        if mode != WAIT_REPLY { cpu.gpr[3] = 0; return Ok(()) }
        if idle != 0 && !self.rt.ae.idle_noted { self.rt.ae.idle_noted = true; self.mac.log.push(String::from("[ae] AESend kAEWaitReply: the idle/filter procs are not called while the sender waits (the other processes run instead)")); }
        let now = self.mac.ev.ticks;
        let deadline = match timeout as i32 { -2 => None, -1 => Some(now.wrapping_add(DEFAULT_TIMEOUT_TICKS)), t => Some(now.wrapping_add(t.max(1) as u32)) };
        let t = txn.expect("a reply is wanted");
        cpu.gpr[3] = 0;
        self.block(cpu, Wait { on: WaitOn::AeReply(t), deadline, out: [0; 3] }, ret, "AESend kAEWaitReply")
    }

    /// The guest block a handler gets: the event descriptor at +0 (a new handle holding `event`), the reply at +8.
    fn ae_handler_block(&mut self, event: &Blob, reply: bool) -> Result<u32, AeErr> {
        let block = self.mac.mm.new_ptr(&mut self.mac.mem, &mut self.mac.heap, 16, true).map_err(|_| AeErr::Os(ad::MEM_FULL))?;
        let h = self.mac.ae_new_handle(&event.encode())?;
        self.mac.ae_set_desc(block, AEVT, h)?;
        if reply {
            let mut r = Blob::new(Kind::Event);
            r.attrs.push(Entry { key: ad::KEY_EVENT_CLASS, ty: ad::TYPE_TYPE, structured: false, read: false, data: AEVT.to_be_bytes().to_vec() });
            r.attrs.push(Entry { key: ad::KEY_EVENT_ID, ty: ad::TYPE_TYPE, structured: false, read: false, data: ANSR.to_be_bytes().to_vec() });
            if let Some(rt) = event.key(true, ad::KEY_RETURN_ID) { r.attrs.push(rt.clone()); }
            let rh = self.mac.ae_new_handle(&r.encode())?;
            self.mac.ae_set_desc(block + 8, AEVT, rh)?;
        } else { self.mac.ae_set_desc(block + 8, ad::TYPE_NULL, 0)?; }
        Ok(block)
    }

    /// `AEProcessAppleEvent(const EventRecord*)`: message = class, where = id; the event is the one WaitNextEvent just delivered to this
    /// process, else an empty event of that class and id.
    fn ae_process(&mut self, rec: u32, cpu: &mut Cpu, ret: u32) -> Result<(), String> {
        let class = self.mac.mem.read_u32(rec + 2).map_err(|_| String::from("AEProcessAppleEvent: the EventRecord leaves guest RAM"))?;
        let id = self.mac.mem.read_u32(rec + 10).map_err(|_| String::from("AEProcessAppleEvent: the EventRecord leaves guest RAM"))?;
        let psn = self.mac.current_psn();
        let hle = self.rt.ae.delivered.remove(&psn).filter(|h| h.class == class && h.id == id);
        let (blob, txn) = match &hle {
            Some(h) => (Blob::decode(&h.event).ok_or_else(|| String::from("AEProcessAppleEvent: a queued event is malformed"))?, h.txn),
            None => {
                let mut b = Blob::new(Kind::Event);
                b.attrs.push(Entry { key: ad::KEY_EVENT_CLASS, ty: ad::TYPE_TYPE, structured: false, read: false, data: class.to_be_bytes().to_vec() });
                b.attrs.push(Entry { key: ad::KEY_EVENT_ID, ty: ad::TYPE_TYPE, structured: false, read: false, data: id.to_be_bytes().to_vec() });
                (b, None)
            }
        };
        let Some((tv, refcon)) = self.ae_handler_for(psn, class, id) else {
            self.mac.log.push(format!("[ae] AEProcessAppleEvent '{}'/'{}': no handler", tag(class), tag(id)));
            if let Some(t) = txn {
                let mut r = Blob::new(Kind::Event);
                r.items.push(Entry { key: ad::KEY_ERROR_NUMBER, ty: ad::TYPE_SHORT, structured: false, read: false, data: (ERR_AE_EVENT_NOT_HANDLED as i16).to_be_bytes().to_vec() });
                self.ae_complete(t, r, ERR_AE_EVENT_NOT_HANDLED)?;
            }
            cpu.gpr[3] = ERR_AE_EVENT_NOT_HANDLED as u32;
            return Ok(());
        };
        let wants_reply = txn.is_some();
        let block = self.ae_handler_block(&blob, wants_reply).map_err(|e| format!("AEProcessAppleEvent: {e:?}"))?;
        self.rt.ae.in_flight.push(InFlight { block, txn, direct: None });
        let frame = Frame { saved: Saved::of(cpu, ret), cont: Continuation::AeHandler };
        let sp = cpu.gpr[1];
        self.enter_tvector(cpu, tv, &[block, block + 8, refcon], sp).map_err(|e| format!("AEProcessAppleEvent: the handler: {e}"))?;
        self.rt.callbacks.push(frame);
        Ok(())
    }

    /// The handler returned: its OSErr goes into the reply (errn) and back to `AEProcessAppleEvent`'s caller; the waiting sender gets the reply.
    pub(crate) fn ae_handler_returned(&mut self, cpu: &mut Cpu, frame: Frame) -> Result<(), String> {
        let err = cpu.gpr[3] as u16 as i16;
        frame.saved.restore(cpu);
        let Some(f) = self.rt.ae.in_flight.pop() else { return Err(String::from("an Apple Event handler returned with none in flight")) };
        let mut reply = self.mac.ae_blob(f.block + 8).unwrap_or_else(|_| Blob::new(Kind::Event));
        if err != 0 && reply.key(false, ad::KEY_ERROR_NUMBER).is_none() {
            reply.items.push(Entry { key: ad::KEY_ERROR_NUMBER, ty: ad::TYPE_SHORT, structured: false, read: false, data: err.to_be_bytes().to_vec() });
        }
        let _ = self.mac.ae_dispose_desc(f.block);
        self.mac.log.push(format!("[ae] the handler returned {err}{}", if f.txn.is_some() { " (a reply is wanted)" } else { "" }));
        match f.direct {
            Some((reply_at, mode)) => {
                if mode == WAIT_REPLY && reply_at != 0 {
                    // the reply descriptor's handle passes to the sender
                    let _ = self.mac.ae_store_blob(f.block + 8, &reply);
                    let (ty, h) = self.mac.ae_desc(f.block + 8).map_err(|e| format!("{e:?}"))?;
                    self.mac.ae_set_desc(reply_at, ty, h).map_err(|e| format!("{e:?}"))?;
                } else { let _ = self.mac.ae_dispose_desc(f.block + 8); }
                cpu.gpr[3] = 0;
            }
            None => {
                let _ = self.mac.ae_dispose_desc(f.block + 8);
                if let Some(t) = f.txn { self.ae_complete(t, reply, 0)?; }
                cpu.gpr[3] = err as i32 as u32;
            }
        }
        let _ = self.mac.mm.dispose_ptr(&mut self.mac.heap, f.block);
        Ok(())
    }

    /// A transaction's reply is ready: copy it to the waiting sender (and wake it with `status`), or queue it as an 'ansr' event.
    fn ae_complete(&mut self, t: u32, reply: Blob, status: i32) -> Result<(), String> {
        let Some(txn) = self.rt.ae.txns.remove(&t) else { return Ok(()) }; // the sender timed out or ended
        if txn.mode == QUEUE_REPLY {
            self.rt.ae.queues.entry(txn.sender_psn).or_default().push_back(Hle { class: AEVT, id: ANSR, event: reply.encode(), txn: None });
            return Ok(());
        }
        let waiting = self.rt.sched.tasks.get(&txn.sender_task).is_some_and(|tk| matches!(tk.wait, Some(Wait { on: WaitOn::AeReply(x), .. }) if x == t));
        if !waiting {
            self.mac.log.push(format!("[ae] a reply for process {{0,{:#x}}}, which no longer waits for it (timed out or gone): dropped", txn.sender_psn));
            return Ok(())
        }
        if txn.reply_at != 0 {
            let h = self.mac.ae_new_handle(&reply.encode()).map_err(|e| format!("the reply: {e:?}"))?;
            self.mac.ae_set_desc(txn.reply_at, AEVT, h).map_err(|e| format!("the reply: {e:?}"))?;
        }
        self.mac.log.push(format!("[ae] the reply reaches process {{0,{:#x}}} (status {status})", txn.sender_psn));
        self.wake(txn.sender_task, status);
        Ok(())
    }

    /// Process `psn` ended: senders waiting on it get connectionInvalid, its queue, handlers and transactions go.
    pub(crate) fn ae_process_gone(&mut self, psn: u32) {
        let dead: Vec<(u32, Txn)> = self.rt.ae.txns.iter().filter(|(_, t)| t.target_psn == psn || t.sender_psn == psn).map(|(&k, &t)| (k, t)).collect();
        for (k, t) in dead {
            self.rt.ae.txns.remove(&k);
            if t.target_psn == psn && t.mode == WAIT_REPLY { self.wake(t.sender_task, crate::process_c::CONNECTION_INVALID); }
        }
        self.rt.ae.queues.remove(&psn);
        self.rt.ae.delivered.remove(&psn);
        self.rt.ae.oapp_sent.remove(&psn);
        self.rt.ae.interaction.remove(&psn);
        self.rt.ae.handlers.retain(|&(p, _, _), _| p != psn);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use heirloom_ui_abi::RecordingGuestUi;
    type M = MacPpc<RecordingGuestUi>;
    const SP: u32 = 0x0007_0000;
    const D: u32 = 0x4000; // scratch: descriptors at D.., data at D+0x100..

    fn call(m: &mut M, f: AeFn, args: &[u32]) -> i32 {
        let mut cpu = Cpu::new(0x0005_0000, SP);
        cpu.lr = 0x0005_1234;
        for (i, a) in args.iter().enumerate() {
            if i < 8 { cpu.gpr[3 + i] = *a } else { m.mac.mem.write_u32(SP + 24 + 4 * i as u32, *a).unwrap() }
        }
        m.ae_call(f, &mut cpu).unwrap();
        cpu.gpr[3] as i32
    }
    fn put(m: &mut M, at: u32, b: &[u8]) { m.mac.mem.write_bytes(at, b).unwrap() }
    fn rd(m: &M, at: u32) -> u32 { m.mac.mem.read_u32(at).unwrap() }

    #[test]
    fn lists_records_and_an_events_parameters_and_attributes_round_trip_through_the_powerpc_calls() {
        let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
        use AeFn::*;
        let (list, ev, out, target, data) = (D, D + 8, D + 16, D + 24, D + 0x100);
        // a list of two longs; AEPutPtr index 0 appends
        assert_eq!(call(&mut m, CreateList, &[0, 0, 0, list]), 0);
        assert_eq!(rd(&m, list), fc(b"list"));
        put(&mut m, data, &7i32.to_be_bytes());
        assert_eq!(call(&mut m, PutPtr, &[list, 0, fc(b"long"), data, 4]), 0);
        put(&mut m, data, &(-2i16).to_be_bytes());
        assert_eq!(call(&mut m, PutPtr, &[list, 0, fc(b"shor"), data, 2]), 0);
        assert_eq!(call(&mut m, CountItems, &[list, data]), 0);
        assert_eq!(rd(&m, data), 2);
        // item 2 coerced shor -> long; item 3 does not exist
        assert_eq!(call(&mut m, GetNthPtr, &[list, 2, fc(b"long"), data + 8, data + 12, data + 16, 4, data + 20]), 0);
        assert_eq!((rd(&m, data + 12), rd(&m, data + 16) as i32, rd(&m, data + 20)), (fc(b"long"), -2, 4));
        assert_eq!(call(&mut m, GetNthPtr, &[list, 3, WILD, data + 8, data + 12, data + 16, 4, data + 20]), i32::from(ad::ERR_AE_BAD_LIST_ITEM));
        // an Apple event to a signature, with the list as a parameter and a long
        put(&mut m, data, b"SRVR");
        assert_eq!(call(&mut m, CreateDesc, &[fc(b"sign"), data, 4, target]), 0);
        assert_eq!(call(&mut m, CreateAppleEvent, &[fc(b"Test"), fc(b"ping"), target, 0xFFFF, 0, ev]), 0);
        assert_eq!(call(&mut m, PutParamDesc, &[ev, fc(b"----"), list]), 0);
        put(&mut m, data, &41i32.to_be_bytes());
        assert_eq!(call(&mut m, PutParamPtr, &[ev, fc(b"data"), fc(b"long"), data, 4]), 0);
        // the class attribute; the missed keyword is the first parameter nobody read yet
        assert_eq!(call(&mut m, GetAttributePtr, &[ev, fc(b"evcl"), fc(b"type"), data + 8, data + 12, 4, data + 16]), 0);
        assert_eq!(rd(&m, data + 12), fc(b"Test"));
        assert_eq!(call(&mut m, GetAttributePtr, &[ev, fc(b"miss"), fc(b"keyw"), data + 8, data + 12, 4, data + 16]), 0);
        assert_eq!(rd(&m, data + 12), fc(b"----"));
        assert_eq!(call(&mut m, SizeOfParam, &[ev, fc(b"data"), data + 8, data + 12]), 0);
        assert_eq!((rd(&m, data + 8), rd(&m, data + 12)), (fc(b"long"), 4));
        assert_eq!(call(&mut m, GetParamPtr, &[ev, fc(b"data"), fc(b"long"), data + 8, data + 12, 4, data + 16]), 0);
        assert_eq!(rd(&m, data + 12), 41);
        // the nested list comes back out as its own descriptor, still a list of two
        assert_eq!(call(&mut m, GetParamDesc, &[ev, fc(b"----"), fc(b"list"), out]), 0);
        assert_eq!(call(&mut m, CountItems, &[out, data]), 0);
        assert_eq!(rd(&m, data), 2);
        // every parameter read: errAEDescNotFound for 'miss'
        assert_eq!(call(&mut m, GetAttributePtr, &[ev, fc(b"miss"), fc(b"keyw"), data + 8, data + 12, 4, data + 16]), i32::from(ad::ERR_AE_DESC_NOT_FOUND));
        // NEGATIVE CONTROLS: an absent parameter, a coercion this model does not do, a count of a simple descriptor
        assert_eq!(call(&mut m, GetParamPtr, &[ev, fc(b"none"), WILD, data + 8, data + 12, 4, data + 16]), i32::from(ad::ERR_AE_DESC_NOT_FOUND));
        assert_eq!(call(&mut m, GetParamPtr, &[ev, fc(b"data"), fc(b"TEXT"), data + 8, data + 12, 4, data + 16]), i32::from(ad::ERR_AE_COERCION_FAIL));
        assert_eq!(call(&mut m, CountItems, &[target, data]), i32::from(ad::ERR_AE_NOT_AE_DESC));
        // Carbon accessors on the simple descriptor; dispose makes it the null descriptor
        assert_eq!(call(&mut m, GetDescDataSize, &[target]), 4);
        assert_eq!(call(&mut m, GetDescData, &[target, data + 32, 4]), 0);
        assert_eq!(rd(&m, data + 32), fc(b"SRVR"));
        assert_eq!(call(&mut m, DisposeDesc, &[target]), 0);
        assert_eq!((rd(&m, target), rd(&m, target + 4)), (fc(b"null"), 0));
    }
}
