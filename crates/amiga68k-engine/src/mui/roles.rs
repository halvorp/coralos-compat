//! What a MUI object IS on screen: the role its class contract gives it (ruling 17, piece 3: toolkit adapters, never inferred
//! from pixels). One decision per class, read from the object's own attributes:
//!
//! | class / state | role |
//! |---|---|
//! | Text, `MUIA_InputMode` None | label (`framed` = it has a frame); a `MUIO_Label` is the label of the control after it |
//! | Text / Rectangle / Image, `MUIA_InputMode` RelVerify or Immediate | button (Area's input contract: `MUIA_Pressed`) |
//! | Text / Rectangle, `MUIA_InputMode` Toggle | toggle button (`MUIA_Selected`) |
//! | Image, `MUIA_Image_Spec` = `MUII_CheckMark`, Toggle | checkbox (`MUIA_Selected`) |
//! | String | text field (`MUIA_String_Contents`; `MUIA_String_Accept` of digits makes it an integer field) |
//! | Cycle / Radio | choice (`MUIA_Cycle_Active` / `MUIA_Radio_Active` over the entries) |
//! | Register | choice, tabs (`MUIA_Group_ActivePage` over `MUIA_Register_Titles`) |
//! | Slider | slider (`MUIA_Numeric_Value` within min..max) |
//! | Gauge | gauge (`MUIA_Gauge_Current` of `MUIA_Gauge_Max`) |
//! | Prop, Scrollbar | scrollbar (`MUIA_Prop_First`, `_Visible` of `_Entries`) |
//! | List | list (rows from the entries; `MUIA_List_Active`); the Listview around it adds no role |
//! | a Group with a frame | group (`MUIA_FrameTitle`) |
//! | Rectangle (spacer, bar), Balance, Area, other Images, Scrollgroup, Virtgroup, plain Group | none: layout only |
//!
//! Facts: Zune's area.c (handle_press / handle_release define what InputMode means), text.c, string.c, cycle.c, radio.c,
//! register.c, numeric.c, slider.c, gauge.c, prop.c, list.c; the MUIV_Frame and MUII_ values are mui.h's.
use alloc::string::String;
use alloc::vec::Vec;
use super::classes::Kind;
use super::ids_gen::*;
use crate::boopsi::{Ext, Objects};
use guestui_abi::{ChoiceStyle, ControlKind, GroupStyle};

/// The object's `MUIA_InputMode` (None 0, RelVerify 1, Immediate 2, Toggle 3).
pub fn input_mode(o: &Objects, obj: u32) -> u32 { o.num_or(obj, MUIA_InputMode, MUIV_InputMode_None) }

pub fn is_label(o: &Objects, obj: u32) -> bool { matches!(o.map.get(&obj).map(|x| &x.ext), Some(Ext::Text { label: true })) }

/// Is `obj` shown? MUIA_ShowMe FALSE hides it and everything below it.
pub fn shown(o: &Objects, obj: u32) -> bool {
    let mut cur = Some(obj);
    while let Some(c) = cur {
        if o.num(c, MUIA_ShowMe) == Some(0) { return false; }
        cur = o.map.get(&c).and_then(|x| x.parent);
    }
    true
}

/// Is `obj` enabled? MUIA_Disabled on it or on any group around it ghosts it.
pub fn enabled(o: &Objects, obj: u32) -> bool {
    let mut cur = Some(obj);
    while let Some(c) = cur {
        if o.flag(c, MUIA_Disabled) { return false; }
        cur = o.map.get(&c).and_then(|x| x.parent);
    }
    true
}

/// A Text's displayed string: MUI's `ESC` formatting sequences removed (`ESC c/l/r/b/u/i/n/-` and `ESC 2..8`/`ESC P[..]`), and the
/// `_` that marks the hotkey character removed when the object says it does (`MUIA_Text_HiCharIdx` = '_').
pub fn display_text(o: &Objects, obj: u32, attr: u32) -> String { strip_format(o.text(obj, attr), o.num(obj, MUIA_Text_HiCharIdx) == Some('_' as u32)) }

/// `raw` without MUI's `ESC` formatting sequences (and, when `hotkey`, the first `_`).
pub fn strip_format(raw: &str, hotkey: bool) -> String {
    let mut out = String::new();
    let mut it = raw.chars().peekable();
    let mut hi_done = false;
    while let Some(c) = it.next() {
        match c {
            '\u{1b}' => {
                let sel = it.next();
                if matches!(sel, Some('P' | 'I')) && it.peek() == Some(&'[') { for d in it.by_ref() { if d == ']' { break; } } }
            }
            '_' if hotkey && !hi_done => hi_done = true,
            c => out.push(c),
        }
    }
    out
}

fn frame(o: &Objects, obj: u32) -> bool { o.num_or(obj, MUIA_Frame, MUIV_Frame_None) != MUIV_Frame_None }

/// The role of a leaf object (one that is drawn by a control of its own), with its label text.
pub fn leaf(o: &Objects, obj: u32) -> Option<ControlKind> {
    let kind = o.kind(obj)?;
    let mode = input_mode(o, obj);
    let pushable = matches!(mode, MUIV_InputMode_RelVerify | MUIV_InputMode_Immediate);
    let selected = o.flag(obj, MUIA_Selected);
    Some(match kind {
        Kind::Text => {
            let text = display_text(o, obj, MUIA_Text_Contents);
            if is_label(o, obj) { ControlKind::Label { text, framed: false } }
            else if pushable { ControlKind::Button }
            else if mode == MUIV_InputMode_Toggle { ControlKind::ToggleButton { on: selected } }
            else { ControlKind::Label { text, framed: frame(o, obj) } }
        }
        Kind::Rectangle => {
            let title = display_text(o, obj, MUIA_Rectangle_BarTitle);
            if pushable { ControlKind::Button } else if mode == MUIV_InputMode_Toggle { ControlKind::ToggleButton { on: selected } }
            else if !title.is_empty() { ControlKind::Label { text: title, framed: false } } else { return None }
        }
        Kind::Image => {
            if o.num(obj, MUIA_Image_Spec) == Some(MUII_CheckMark) && mode == MUIV_InputMode_Toggle { ControlKind::Checkbox { checked: selected } }
            else if pushable { ControlKind::Button } else { return None }
        }
        Kind::Area if pushable => ControlKind::Button,
        Kind::String => {
            let accept = o.text(obj, MUIA_String_Accept);
            let integer = !accept.is_empty() && accept.chars().all(|c| c.is_ascii_digit() || c == '-');
            ControlKind::Field { text: o.text(obj, MUIA_String_Contents).into(), max_len: o.num_or(obj, MUIA_String_MaxLen, 80).saturating_sub(1), editable: true, integer }
        }
        Kind::Cycle => choice(o, obj, ChoiceStyle::Cycle, MUIA_Cycle_Entries, MUIA_Cycle_Active),
        Kind::Radio => choice(o, obj, ChoiceStyle::Radio, MUIA_Radio_Entries, MUIA_Radio_Active),
        Kind::Register => choice(o, obj, ChoiceStyle::Tabs, MUIA_Register_Titles, MUIA_Group_ActivePage),
        Kind::Slider => ControlKind::Slider { min: o.num_or(obj, MUIA_Numeric_Min, 0) as i32, max: o.num_or(obj, MUIA_Numeric_Max, 100) as i32, value: o.num_or(obj, MUIA_Numeric_Value, 0) as i32 },
        Kind::Gauge => {
            let max = o.num_or(obj, MUIA_Gauge_Max, 100).max(1);
            ControlKind::Gauge { value: o.num_or(obj, MUIA_Gauge_Current, 0).min(max), max }
        }
        Kind::Prop | Kind::Scrollbar => {
            let (total, visible) = (o.num_or(obj, MUIA_Prop_Entries, 0), o.num_or(obj, MUIA_Prop_Visible, 0));
            let horiz = o.flag(obj, MUIA_Prop_Horiz) || o.flag(obj, MUIA_Group_Horiz);
            ControlKind::ScrollBar { vertical: !horiz, total, visible: visible.min(total), top: o.num_or(obj, MUIA_Prop_First, 0).min(total.saturating_sub(visible)) }
        }
        Kind::List => return super::list::control(o, obj),
        _ => return None,
    })
}

/// The caption of a button or toggle: the text the object shows (a Text's contents, a Rectangle's bar title).
pub fn caption(o: &Objects, obj: u32) -> String {
    match o.kind(obj) { Some(Kind::Rectangle) => display_text(o, obj, MUIA_Rectangle_BarTitle), Some(Kind::Text) => display_text(o, obj, MUIA_Text_Contents), _ => String::new() }
}

fn choice(o: &Objects, obj: u32, style: ChoiceStyle, entries: u32, active: u32) -> ControlKind {
    let labels: Vec<String> = o.entries(obj, entries).iter().map(|s| strip_format(s, false)).collect();
    let selected = if labels.is_empty() { 0 } else { o.num_or(obj, active, 0).min(labels.len() as u32 - 1) };
    ControlKind::Choice { style, labels, selected }
}

/// A container that draws a frame is a group around its children (title from `MUIA_FrameTitle`).
pub fn frame_group(o: &Objects, obj: u32) -> Option<ControlKind> {
    let kind = o.kind(obj)?;
    if !kind.is_container() || kind == Kind::Register || !frame(o, obj) { return None; }
    Some(ControlKind::Group { title: o.text(obj, MUIA_FrameTitle).into(), style: GroupStyle::Plain })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn display_text_drops_formatting_escapes_and_the_hotkey_marker_only_when_declared() {
        let mut o = Objects::default();
        o.map.insert(1, crate::boopsi::Obj { kind: Kind::Text, block: 0, attrs: Default::default(), strs: Default::default(), arrs: Default::default(), bufs: Default::default(),
            children: Vec::new(), parent: None, notifies: Vec::new(), ext: Ext::None });
        o.map.get_mut(&1).unwrap().strs.insert(MUIA_Text_Contents, "\u{1b}c_Play \u{1b}P[3]now".into());
        assert_eq!(display_text(&o, 1, MUIA_Text_Contents), "_Play now", "no HiCharIdx: the underscore is the program's text");
        o.map.get_mut(&1).unwrap().attrs.insert(MUIA_Text_HiCharIdx, '_' as u32);
        assert_eq!(display_text(&o, 1, MUIA_Text_Contents), "Play now");
    }
}
