//! MUIM_Notify: when an attribute is set to a matching value, call a method on another object.
use alloc::format;
use alloc::vec::Vec;
use super::{ids::*, method};
use crate::boopsi::{Kind, Notify};
use crate::{machine::Amiga, Refusal};
use guestui_abi::GuestUi;

/// Notify nesting beyond this is a loop (a notification whose action sets the attribute that triggered it).
const MAX_DEPTH: u32 = 16;

/// MUIM_Notify args: `[TrigAttr, TrigVal, DestObj, FollowParams, follow...]`; follow = method id + its arguments.
pub fn add<U: GuestUi>(a: &mut Amiga<U>, obj: u32, args: &[u32]) -> Result<u32, Refusal> {
    let n = args.get(3).copied().unwrap_or(0) as usize;
    if n == 0 || args.len() < 4 + n { return Err(Refusal("MUIM_Notify: no follow-up method (FollowParams is 0 or the message is short)".into())); }
    let dest = args[2];
    if !(MUIV_NOTIFY_SELF..=MUIV_NOTIFY_PARENT).contains(&dest) && !a.objects.map.contains_key(&dest) {
        return Err(Refusal(format!("MUIM_Notify: destination {dest:#x} is not a MUI object")));
    }
    let rec = Notify { attr: args[0], val: args[1], dest, params: args[4..4 + n].to_vec() };
    a.objects.map.get_mut(&obj).ok_or_else(|| Refusal("MUIM_Notify: no such object".into()))?.notifies.push(rec);
    Ok(0)
}

fn ancestor<U: GuestUi>(a: &Amiga<U>, mut obj: u32, want: Option<Kind>) -> Option<u32> {
    loop {
        let p = a.objects.map.get(&obj)?.parent?;
        if want.is_none_or(|k| a.objects.map.get(&p).is_some_and(|o| o.kind == k)) { return Some(p); }
        obj = p;
    }
}

/// The root of the tree `obj` is in (itself when it has no parent).
pub fn root<U: GuestUi>(a: &Amiga<U>, mut obj: u32) -> u32 { while let Some(p) = a.objects.map.get(&obj).and_then(|o| o.parent) { obj = p; } obj }

fn resolve<U: GuestUi>(a: &Amiga<U>, obj: u32, dest: u32) -> Result<u32, Refusal> {
    let r = match dest {
        MUIV_NOTIFY_SELF => Some(obj),
        MUIV_NOTIFY_WINDOW => ancestor(a, obj, Some(Kind::Window)),
        MUIV_NOTIFY_APPLICATION => ancestor(a, obj, Some(Kind::Application)),
        MUIV_NOTIFY_PARENT => ancestor(a, obj, None),
        d => Some(d),
    };
    r.filter(|d| a.objects.map.contains_key(d)).ok_or_else(|| Refusal(format!("MUIM_Notify: destination {dest:#x} no longer resolves")))
}

/// `attr` of `obj` was just set to `val`: run every matching notification (every set fires, even to the same value).
pub fn trigger<U: GuestUi>(a: &mut Amiga<U>, obj: u32, attr: u32, val: u32) -> Result<(), Refusal> {
    let hits: Vec<Notify> = a.objects.map.get(&obj).map(|o| o.notifies.iter().filter(|n| n.attr == attr && (n.val == MUIV_EVERY_TIME || n.val == val)).cloned().collect()).unwrap_or_default();
    if hits.is_empty() { return Ok(()); }
    if a.objects.depth >= MAX_DEPTH { return Err(Refusal(format!("MUIM_Notify: more than {MAX_DEPTH} nested notifications on {} (a loop?)", attr_name(attr)))); }
    a.objects.depth += 1;
    let mut result = Ok(());
    for n in hits {
        let r = resolve(a, obj, n.dest).and_then(|dest| {
            let args: Vec<u32> = n.params[1..].iter().map(|&p| match p { MUIV_TRIGGER_VALUE => val, MUIV_NOT_TRIGGER_VALUE => (val == 0) as u32, p => p }).collect();
            method(a, dest, n.params[0], &args).map(|_| ())
        });
        if result.is_ok() { result = r; }
    }
    a.objects.depth -= 1;
    result
}
