//! A native control's intent, applied to its MUI object exactly as MUI's own input handling would (ruling 17 piece 8): the
//! MUI table ([`crate::translate::mui`]) lists the `set` calls for the control's role; this is its adapter. The program is
//! told only through the notifications those `set` calls run, which is what MUI does.
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use super::classes::Kind;
use super::ids_gen::*;
use super::object::{self, Setting};
use super::roles;
use crate::translate::mui::{self as table, Attr, Val};
use crate::translate::On;
use crate::{machine::Amiga, Refusal};
use guestui_abi::{ControlValue, GuestUi, Intent};

/// The real attribute number of `attr` on `kind`.
fn attr_id(kind: Kind, attr: Attr) -> Result<u32, Refusal> {
    Ok(match (attr, kind) {
        (Attr::Selected, _) => MUIA_Selected,
        (Attr::Pressed, _) => MUIA_Pressed,
        (Attr::Timer, _) => MUIA_Timer,
        (Attr::Active, Kind::Cycle) => MUIA_Cycle_Active,
        (Attr::Active, Kind::Radio) => MUIA_Radio_Active,
        (Attr::Active, Kind::Register) => MUIA_Group_ActivePage,
        (Attr::Value, _) => MUIA_Numeric_Value,
        (Attr::Contents, _) => MUIA_String_Contents,
        (Attr::Acknowledge, _) => MUIA_String_Acknowledge,
        (Attr::First, _) => MUIA_Prop_First,
        (Attr::ListActive, _) => MUIA_List_Active,
        (Attr::Active, k) => return Err(Refusal(format!("MUI table: a {k:?} has no Active attribute"))),
    })
}

/// The number an intent carries (the new level, state, choice, row or scroll position).
fn intent_number(i: &Intent) -> Option<u32> {
    Some(match i {
        Intent::SetValue(ControlValue::Checked(b)) => *b as u32,
        Intent::SetValue(ControlValue::Level(l)) | Intent::Drag(ControlValue::Level(l)) => *l as u32,
        Intent::SetValue(ControlValue::Choice(c)) => *c,
        Intent::SelectRow(r) => *r,
        Intent::ScrollTo(t) => *t,
        Intent::Drag(ControlValue::Scroll { top, .. }) => *top,
        _ => return None,
    })
}

fn intent_text(i: &Intent) -> Option<&String> { match i { Intent::CommitText(t) | Intent::Drag(ControlValue::Text(t)) => Some(t), _ => None } }

/// Does this intent press a popup button whose requester the engine does not bridge?
fn popup_refusal(a: &crate::boopsi::Objects, obj: u32, on: On) -> Option<Refusal> {
    let parent = a.map.get(&obj).and_then(|o| o.parent).and_then(|p| a.kind(p))?;
    (matches!(on, On::Press | On::Activate) && parent.is_a(Kind::Popstring) && a.kind(obj) != Some(Kind::String))
        .then(|| Refusal(format!("MUI {parent:?} class: the popup button was pressed, and the requester or window behind it is not bridged yet")))
}

/// The table row `intent` means for `obj`'s control, or the named refusal (checked when the event arrives AND when it is applied).
pub fn validate<U: GuestUi>(a: &Amiga<U>, obj: u32, intent: &Intent) -> Result<&'static table::Row, Refusal> {
    let control = roles::leaf(&a.objects, obj).ok_or_else(|| Refusal(format!("Intent for {obj:#x}: the object is not a control any more")))?;
    let row = table::lookup(&control, intent)?;
    if let Some(r) = popup_refusal(&a.objects, obj, row.on) { return Err(r); }
    Ok(row)
}

/// Apply `intent` to `obj` (the program's next `NewInput` calls this for every queued native event).
pub fn apply<U: GuestUi>(a: &mut Amiga<U>, obj: u32, intent: &Intent) -> Result<(), Refusal> {
    let row = validate(a, obj, intent)?;
    let selected = a.objects.flag(obj, MUIA_Selected);
    if matches!(row.on, On::ReleaseInside | On::ReleaseOutside) && !selected { return Ok(()); } // MUI: a release only acts on a pressed object
    // The host already shows what the user did; tell the projection so the refresh below does not echo it back.
    let shown = match intent { Intent::SetValue(v @ (ControlValue::Checked(_) | ControlValue::Level(_) | ControlValue::Choice(_))) | Intent::Drag(v @ ControlValue::Level(_)) => Some(v.clone()), Intent::CommitText(t) | Intent::Drag(ControlValue::Text(t)) => Some(ControlValue::Text(t.clone())), _ => None };
    if let Some(v) = shown { a.objects.proj.adopt(obj as u64, &v); }
    let kind = a.objects.kind(obj).ok_or_else(|| Refusal(format!("{obj:#x} is not a MUI object")))?;
    for step in row.effect {
        let mut sets: Vec<(u32, Setting)> = Vec::new();
        for w in step.writes {
            let id = attr_id(kind, w.attr)?;
            let cur = a.objects.num_or(obj, id, 0);
            let value = match w.val {
                Val::Zero => Setting::Num(0),
                Val::One => Setting::Num(1),
                Val::Flip => Setting::Num((cur == 0) as u32),
                Val::Tick => Setting::Num(cur.wrapping_add(1)),
                Val::Intent => Setting::Num(intent_number(intent).ok_or_else(|| Refusal(format!("MUI table: {intent:?} carries no number for {w:?}")))?),
                Val::Text => Setting::Text(intent_text(intent).cloned().ok_or_else(|| Refusal(format!("MUI table: {intent:?} carries no text for {w:?}")))?),
            };
            sets.push((id, value));
        }
        object::set_many(a, obj, &sets, step.notify)?;
    }
    Ok(())
}
