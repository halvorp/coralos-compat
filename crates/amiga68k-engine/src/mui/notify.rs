//! MUIM_Notify: when an attribute is set to a matching value, call a method on another object.
use alloc::format;
use alloc::vec::Vec;
use super::classes::Kind;
use super::ids_gen::*;
use super::hooks::{After, HookCall, Msg};
use super::{method, object::ancestor};
use crate::boopsi::Notify;
use crate::{machine::Amiga, Refusal};
use guestui_abi::GuestUi;

/// Notify nesting beyond this is a loop (a notification whose action sets the attribute that triggered it).
const MAX_DEPTH: u32 = 16;

/// MUIM_Notify args: `[TrigAttr, TrigVal, DestObj, FollowParams, follow...]`; follow = method id + its arguments.
pub fn add<U: GuestUi>(a: &mut Amiga<U>, obj: u32, args: &[u32]) -> Result<u32, Refusal> {
    let n = args.get(3).copied().unwrap_or(0) as usize;
    if n == 0 || args.len() < 4 + n { return Err(Refusal("MUIM_Notify: no follow-up method (FollowParams is 0 or the message is short)".into())); }
    let dest = args[2];
    if !(MUIV_Notify_Self..=MUIV_Notify_Parent).contains(&dest) && !a.objects.map.contains_key(&dest) && a.classes.class_of(dest).is_none() {
        return Err(Refusal(format!("MUIM_Notify: destination {dest:#x} is neither a MUI object nor a BOOPSI object")));
    }
    let rec = Notify { attr: args[0], val: args[1], dest, params: args[4..4 + n].to_vec() };
    a.objects.map.get_mut(&obj).ok_or_else(|| Refusal("MUIM_Notify: no such object".into()))?.notifies.push(rec);
    Ok(0)
}

fn resolve<U: GuestUi>(a: &Amiga<U>, obj: u32, dest: u32) -> Result<u32, Refusal> {
    let r = match dest {
        MUIV_Notify_Self => Some(obj),
        MUIV_Notify_Window => ancestor(&a.objects, obj, Kind::Window),
        MUIV_Notify_Application => ancestor(&a.objects, obj, Kind::Application),
        MUIV_Notify_Parent => a.objects.map.get(&obj).and_then(|o| o.parent),
        d => Some(d),
    };
    r.filter(|d| a.objects.map.contains_key(d) || a.classes.class_of(*d).is_some()).ok_or_else(|| Refusal(format!("MUIM_Notify: destination {dest:#x} no longer resolves")))
}

/// `attr` of `obj` was just set to `val`: run every matching notification (every set fires, even to the same value).
pub fn trigger<U: GuestUi>(a: &mut Amiga<U>, obj: u32, attr: u32, val: u32) -> Result<(), Refusal> {
    let hits: Vec<Notify> = a.objects.map.get(&obj).map(|o| o.notifies.iter().filter(|n| n.attr == attr && (n.val == MUIV_EveryTime || n.val == val)).cloned().collect()).unwrap_or_default();
    if hits.is_empty() { return Ok(()); }
    if a.objects.depth >= MAX_DEPTH { return Err(Refusal(format!("MUIM_Notify: more than {MAX_DEPTH} nested notifications on {} (a loop?)", super::attrs::name(attr)))); }
    a.objects.depth += 1;
    let mut result = Ok(());
    for n in hits {
        let r = resolve(a, obj, n.dest).and_then(|dest| {
            let args: Vec<u32> = n.params[1..].iter().map(|&p| match p { MUIV_TriggerValue => val, MUIV_NotTriggerValue => (val == 0) as u32, p => p }).collect();
            send(a, dest, n.params[0], &args).map(|_| ())
        });
        if result.is_ok() { result = r; }
    }
    a.objects.depth -= 1;
    result
}

/// Send method `id` to `dest` the way MUI's own notification does: an object of a program's OWN class (a BOOPSI object, or a MUI
/// object of a subclass) is sent it through the class dispatcher, as a guest call with A0 = class, A2 = object, A1 = the message
/// (method id first); a plain MUI object's method runs here.
pub fn send<U: GuestUi>(a: &mut Amiga<U>, dest: u32, id: u32, args: &[u32]) -> Result<u32, Refusal> {
    let guest_class = a.classes.class_of(dest).filter(|c| a.classes.recs.get(c).is_some_and(|r| r.native.is_none()));
    let Some(cl) = guest_class else { return method(a, dest, id, args) };
    let mut msg = alloc::vec![id];
    msg.extend_from_slice(args);
    a.objects.hooks.push_back(HookCall { hook: cl, obj: dest, msg: Msg::Params(msg), after: After::Nothing });
    Ok(0)
}
