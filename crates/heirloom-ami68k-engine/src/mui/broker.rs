//! The Application's commodity: Zune (classes/application.c, APL, read for facts only) makes a CxBroker for EVERY Application, so Exchange (or
//! any `BrokerCommand`) can disable, enable, show, hide or kill it. Done through the commodities lane's object graph ([`crate::commodities`]):
//! * OM_NEW (unless `MUIA_Application_UseCommodities` is FALSE): a message port, and `CxBroker({NB_VERSION, Name = MUIA_Application_Title or
//!   "Unnamed", Title = MUIA_Application_Version (a leading "$VER: " removed) or "Unnamed", Descr = MUIA_Application_Description or "?", Unique 0,
//!   COF_SHOW_HIDE, Pri = MUIA_Application_BrokerPri, Port})`; the object graph keeps name, title, description, priority and port (CopyBrokerList
//!   hands them to Exchange; nb_Flags is not kept, as in AROS), activated when `MUIA_Application_Active` (default TRUE) is.
//! * `MUIA_Application_Active` set: ActivateCxObj on the broker. GetAttr: `MUIA_Application_Broker` / `_BrokerPort` the object / the port.
//! * Inside `MUIM_Application_NewInput` (the signal of the port is part of the mask it returns): every message on the port is a CxMsg; a
//!   CXM_COMMAND is handled CXCMD_DISABLE = `set(app, Active, FALSE)`, CXCMD_ENABLE = `set(app, Active, TRUE)`, CXCMD_KILL = `SetSignal(CTRL_C)` of
//!   the running task (the program's own break handling then quits), CXCMD_APPEAR / DISAPPEAR = NOTHING (application.c: "application needs to be
//!   in control of this": only the hook sees them, and shows or hides its own windows). Then `MUIA_Application_BrokerHook` is called as
//!   `hook(A0 = hook, A2 = the Application, A1 = the CxMsg)` and the message is replied after the hook returns.
//!   Zune runs this when the port's signal is among those passed to NewInput; here the port is drained at every NewInput (a message always signals).
//! * OM_DISPOSE: DeleteCxObjAll, the unread messages are replied, the port is deleted.
use alloc::string::String;
use crate::boopsi::Ext;
use crate::commodities_cmd::{CXCMD_DISABLE, CXCMD_ENABLE, CXCMD_KILL, CXM_COMMAND, CXM_ID, CXM_TYPE};
use crate::mui::hooks::{After, HookCall, Msg};
use crate::{commodities, exec_wait, machine::Amiga, ports, Refusal};
use super::ids_gen::*;
use heirloom_ui_abi::GuestUi;

const NB_NAME_DEFAULT: &str = "Unnamed";
const SIGBREAKF_CTRL_C: u32 = 1 << 12;
const MP_MSGLIST: u32 = 20;

/// An Application's commodity: the CxBroker object and the port its commands arrive on.
#[derive(Debug, Clone, Copy)]
pub struct Broker { pub co: u32, pub port: u32 }

fn broker_of<U: GuestUi>(a: &Amiga<U>, app: u32) -> Option<Broker> {
    match a.objects.map.get(&app).map(|o| &o.ext) { Some(Ext::App { broker, .. }) => *broker, _ => None }
}

/// Make the Application's broker (OM_NEW, after the tags are stored).
pub(super) fn create<U: GuestUi>(a: &mut Amiga<U>, app: u32, tags: &[(u32, u32)]) -> Result<(), Refusal> {
    let use_cx = tags.iter().rev().find(|&&(t, _)| t == MUIA_Application_UseCommodities).map_or(true, |&(_, v)| v != 0);
    if !use_cx { return Ok(()); }
    let title = a.objects.text(app, MUIA_Application_Title);
    let name = String::from(if title.is_empty() { NB_NAME_DEFAULT } else { title });
    let text = |a: &Amiga<U>, attr, default: &str| { let s = a.objects.text(app, attr); String::from(if s.is_empty() { default } else { s }) };
    let btitle = text(a, MUIA_Application_Version, NB_NAME_DEFAULT);
    let btitle = String::from(btitle.strip_prefix("$VER: ").unwrap_or(&btitle)); // application.c: nb_Title += 6
    let descr = text(a, MUIA_Application_Description, "?");
    let pri = a.objects.num_or(app, MUIA_Application_BrokerPri, 0) as i8; // a BYTE
    super::object::store_num(a, app, MUIA_Application_BrokerPri, pri as i32 as u32);
    let Some(port) = ports::new_port(a)? else { return Ok(()) }; // no signal or memory: no commodity, as Zune
    let (co, _) = commodities::add_broker(a, (&name, &btitle, &descr), false, pri, port)?;
    if co == 0 { ports::free_port(a, port)?; return Ok(()); }
    if a.objects.flag(app, MUIA_Application_Active) { commodities::set_active(a, co, true); }
    if let Some(Ext::App { broker, .. }) = a.objects.map.get_mut(&app).map(|o| &mut o.ext) { *broker = Some(Broker { co, port }); }
    Ok(())
}

/// `MUIA_Application_Active` was set: the broker follows.
pub(super) fn sync_active<U: GuestUi>(a: &mut Amiga<U>, app: u32, on: bool) { if let Some(b) = broker_of(a, app) { commodities::set_active(a, b.co, on); } }

/// GetAttr of `MUIA_Application_Broker` / `MUIA_Application_BrokerPort` (0 without a commodity).
pub(super) fn get<U: GuestUi>(a: &Amiga<U>, app: u32, attr: u32) -> u32 {
    broker_of(a, app).map_or(0, |b| if attr == MUIA_Application_Broker { b.co } else { b.port })
}

/// The port's signal, part of what `NewInput` tells the program to wait on.
pub(super) fn signal_mask<U: GuestUi>(a: &Amiga<U>, app: u32) -> u32 {
    broker_of(a, app).map_or(0, |b| a.mem.read_u8(b.port + 15).map_or(0, |bit| 1 << bit))
}

/// Handle every command waiting on the port (inside NewInput).
pub(super) fn service<U: GuestUi>(a: &mut Amiga<U>, app: u32) -> Result<(), Refusal> {
    let Some(b) = broker_of(a, app) else { return Ok(()) };
    loop {
        let msg = exec_wait::rem_head(a, b.port + MP_MSGLIST)?;
        if msg == 0 { return Ok(()); }
        a.delivered_msg(msg);
        let m = Refusal::mem("CxMsg");
        if a.mem.read_u8(msg + CXM_TYPE).map_err(&m)? == CXM_COMMAND {
            match a.mem.read_u32(msg + CXM_ID).map_err(&m)? {
                CXCMD_DISABLE => super::object::set_attr(a, app, MUIA_Application_Active, 0)?,
                CXCMD_ENABLE => super::object::set_attr(a, app, MUIA_Application_Active, 1)?,
                CXCMD_KILL => exec_wait::signal_task(a, a.sched.cur, SIGBREAKF_CTRL_C)?,
                _ => {} // APPEAR / DISAPPEAR: the application is in control (its hook)
            }
        }
        let hook = a.objects.num_or(app, MUIA_Application_BrokerHook, 0);
        if hook != 0 { a.objects.hooks.push_back(HookCall { hook, obj: app, msg: Msg::Addr(msg), after: After::ReplyCx(msg) }); }
        else { exec_wait::reply_to_port(a, msg)?; }
    }
}

/// OM_DISPOSE: the broker and its port go.
pub(super) fn destroy<U: GuestUi>(a: &mut Amiga<U>, app: u32) -> Result<(), Refusal> {
    let Some(b) = broker_of(a, app) else { return Ok(()) };
    commodities::delete_tree(a, b.co)?;
    loop { // Zune replies whatever was never read
        let msg = exec_wait::rem_head(a, b.port + MP_MSGLIST)?;
        if msg == 0 { break; }
        exec_wait::reply_to_port(a, msg)?;
    }
    if let Some(Ext::App { broker, .. }) = a.objects.map.get_mut(&app).map(|o| &mut o.ext) { *broker = None; }
    ports::free_port(a, b.port)
}
