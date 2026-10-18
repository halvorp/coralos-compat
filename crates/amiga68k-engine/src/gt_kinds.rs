//! GadTools gadget kinds beyond BUTTON_KIND / TEXT_KIND, mapped onto the guestui ControlKinds that already exist:
//!
//! | GadTools kind | native control | guest-visible state |
//! |---|---|---|
//! | CHECKBOX_KIND (GTCB_Checked) | `Checkbox` | `Gadget.Flags` GFLG_SELECTED |
//! | STRING_KIND (GTST_String, GTST_MaxChars) | `String` | a real `struct StringInfo` + buffer (`Gadget.SpecialInfo`) |
//! | INTEGER_KIND (GTIN_Number, GTIN_MaxChars) | `String` (digits) | the same, plus `StringInfo.LongInt` |
//! | SLIDER_KIND (GTSL_Min / Max / Level) | `Slider` | host-side level (read back with GT_GetGadgetAttrsA) |
//!
//! CYCLE_KIND and MX_KIND (a choice among N labels), LISTVIEW, PALETTE, SCROLLER and NUMBER stay NAMED refusals:
//! `ControlKind` has no choice-list, list or number control and this lane does not invent one.
//!
//! Native events come back through [`control_changed`]: a Button press, a Checkbox toggle, a String commit and a Slider
//! release become IDCMP_GADGETUP with IAddress = the guest Gadget (Code = checked / level), after the guest-visible
//! state they changed has been updated. GadTools filters its gadgets' raw Intuition messages in GT_GetIMsg (a slider's
//! Code becomes its level); the messages built here already carry the filtered values, so for the kinds bridged
//! GT_GetIMsg is exactly GetMsg. Defaults come from the AROS gadtools classes (facts only): MaxChars 64 (string) / 10
//! (integer), slider Min 0, Max 15, Level = Min; Intuition counts the NUL in StringInfo.MaxChars, GadTools does not.
//! Offsets (m68k gcc -mno-align-int over the AROS field lists): Gadget.Flags 12, SpecialInfo 34; StringInfo size 36,
//! Buffer 0, BufferPos 8, MaxChars 10, NumChars 16, LongInt 28.
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::format;
use crate::{gadtools::{GadgetInfo, BUTTON_KIND, TEXT_KIND}, idcmp, machine::Amiga, memory, run::Delivered, tags, translate::{self, gadtools}, Refusal};
use guestui_abi::{ControlKind, ControlValue, GuestUi, Intent};
use retro_m68k::Cpu;

pub const LVO_GT_GET_GADGET_ATTRS_A: i32 = -174;

pub const CHECKBOX_KIND: u32 = 2;
pub const INTEGER_KIND: u32 = 3;
pub const SLIDER_KIND: u32 = 11;
pub const STRING_KIND: u32 = 12;

const GT_TAGBASE: u32 = tags::TAG_USER + 0x0008_0000;
pub const GTCB_CHECKED: u32 = GT_TAGBASE + 4;
pub const GTTX_TEXT: u32 = GT_TAGBASE + 11;
pub const GTSL_MIN: u32 = GT_TAGBASE + 38;
pub const GTSL_MAX: u32 = GT_TAGBASE + 39;
pub const GTSL_LEVEL: u32 = GT_TAGBASE + 40;
pub const GTST_STRING: u32 = GT_TAGBASE + 45;
pub const GTST_MAXCHARS: u32 = GT_TAGBASE + 46;
pub const GTIN_NUMBER: u32 = GT_TAGBASE + 47;
pub const GTIN_MAXCHARS: u32 = GT_TAGBASE + 48;
/// GA_Disabled = GA_Dummy (TAG_USER + 0x30000) + 14, intuition/gadgetclass.h.
pub const GA_DISABLED: u32 = tags::TAG_USER + 0x30000 + 14;

const GFLG_SELECTED: u16 = 0x0080;
const GD_FLAGS: u32 = 12;
const GD_SPECIAL_INFO: u32 = 34;
const SI_SIZE: u32 = 36;
const SI_MAX_CHARS: u32 = 10;

/// What the host holds for a gadget beyond the generic record (the "native side's" copy of its value).
#[derive(Debug, Clone, Default)]
pub struct KindState {
    pub checked: bool,
    pub disabled: bool,
    pub min: i32,
    pub max: i32,
    pub level: i32,
    pub max_chars: u32,
    /// Guest StringInfo and its buffer (0 = none), and every block this gadget owns (freed with it).
    pub string_info: u32,
    pub buffer: u32,
    pub allocs: Vec<(u32, u32)>,
}

pub fn bridged(kind: u32) -> bool { matches!(kind, TEXT_KIND | BUTTON_KIND | CHECKBOX_KIND | INTEGER_KIND | SLIDER_KIND | STRING_KIND) }

/// Why a kind is refused, by name.
pub fn refusal(kind: u32) -> Refusal {
    let why = match kind {
        5 => "MX_KIND: guestui ControlKind has no radio-group (choice among N labels) control",
        7 => "CYCLE_KIND: guestui ControlKind has no cycle (choice among N labels) control",
        4 => "LISTVIEW_KIND: guestui ControlKind has no list control",
        8 => "PALETTE_KIND: guestui ControlKind has no palette control",
        9 => "SCROLLER_KIND: guestui ControlKind has no scrollbar control",
        6 => "NUMBER_KIND: guestui ControlKind has no number display (TEXT_KIND is bridged)",
        _ => "no such GadTools kind is bridged",
    };
    Refusal(format!("CreateGadgetA: kind {kind} not bridged: {why}"))
}

fn opt_str<U: GuestUi>(a: &Amiga<U>, p: u32, what: &str) -> Result<String, Refusal> {
    if p == 0 { Ok(String::new()) } else { a.mem.read_cstr(p).map_err(Refusal::mem(what)) }
}

/// `s` cut to at most `n` bytes at a character boundary.
fn truncate(s: &str, n: u32) -> String {
    let mut end = (n as usize).min(s.len());
    while !s.is_char_boundary(end) { end -= 1; }
    s[..end].to_string()
}

/// The kind's state and its initial text (TEXT: GTTX_Text, STRING: GTST_String, INTEGER: GTIN_Number) from the tags.
pub fn state_from_tags<U: GuestUi>(a: &Amiga<U>, kind: u32, t: &[(u32, u32)]) -> Result<(KindState, String), Refusal> {
    let get = |tag| tags::get(t, tag);
    let mut st = KindState { disabled: get(GA_DISABLED).unwrap_or(0) != 0, ..Default::default() };
    let mut text = String::new();
    match kind {
        TEXT_KIND => text = opt_str(a, get(GTTX_TEXT).unwrap_or(0), "GTTX_Text")?,
        CHECKBOX_KIND => st.checked = get(GTCB_CHECKED).unwrap_or(0) != 0,
        SLIDER_KIND => {
            (st.min, st.max) = (get(GTSL_MIN).unwrap_or(0) as i32, get(GTSL_MAX).unwrap_or(15) as i32);
            if st.min > st.max { return Err(Refusal(format!("CreateGadgetA: SLIDER_KIND GTSL_Min {} > GTSL_Max {}", st.min, st.max))); }
            st.level = (get(GTSL_LEVEL).unwrap_or(st.min as u32) as i32).clamp(st.min, st.max);
        }
        STRING_KIND => {
            st.max_chars = get(GTST_MAXCHARS).unwrap_or(64) & 0xFFFF;
            text = truncate(&opt_str(a, get(GTST_STRING).unwrap_or(0), "GTST_String")?, st.max_chars);
        }
        INTEGER_KIND => {
            st.max_chars = get(GTIN_MAXCHARS).unwrap_or(10) & 0xFFFF;
            text = (get(GTIN_NUMBER).unwrap_or(0) as i32).to_string();
            if text.len() as u32 > st.max_chars { return Err(Refusal(format!("CreateGadgetA: GTIN_Number {text} does not fit GTIN_MaxChars {}", st.max_chars))); }
        }
        _ => {}
    }
    Ok((st, text))
}

/// The native control a gadget is.
pub fn control_kind(info: &GadgetInfo) -> ControlKind {
    let s = &info.state;
    match info.kind {
        TEXT_KIND => ControlKind::Label { text: info.text.clone(), framed: info.framed },
        CHECKBOX_KIND => ControlKind::Checkbox { checked: s.checked },
        STRING_KIND | INTEGER_KIND => ControlKind::String { text: info.text.clone(), max_len: s.max_chars },
        SLIDER_KIND => ControlKind::Slider { min: s.min, max: s.max, value: s.level },
        _ => ControlKind::Button,
    }
}

fn number(text: &str) -> Result<i32, Refusal> {
    let t = text.trim();
    if t.is_empty() { return Ok(0); }
    t.parse().map_err(|_| Refusal(format!("INTEGER_KIND: {text:?} is not a number")))
}

/// Write `text` into the gadget's StringInfo buffer (NUL-terminated), NumChars, the cursor and, for INTEGER_KIND,
/// LongInt (the shared strgclass store, boopsi_strg.rs).
fn write_buffer<U: GuestUi>(a: &mut Amiga<U>, kind: u32, st: &KindState, text: &str) -> Result<(), Refusal> {
    if st.string_info == 0 { return Ok(()); }
    if kind == INTEGER_KIND { number(text)?; }
    crate::boopsi_strg::store_text(a, st.string_info, &truncate(text, st.max_chars), kind == INTEGER_KIND)
}

fn set_selected<U: GuestUi>(a: &mut Amiga<U>, g: u32, on: bool) -> Result<(), Refusal> {
    let f = a.mem.read_u16(g + GD_FLAGS).map_err(Refusal::mem("Gadget.Flags"))?;
    a.mem.write_u16(g + GD_FLAGS, if on { f | GFLG_SELECTED } else { f & !GFLG_SELECTED }).map_err(Refusal::mem("Gadget.Flags"))
}

/// Give the freshly made gadget `g` its guest-visible kind state (SELECTED flag, StringInfo + buffer).
pub fn init_guest<U: GuestUi>(a: &mut Amiga<U>, g: u32) -> Result<(), Refusal> {
    let info = a.gadgets.get(&g).cloned().ok_or_else(|| Refusal(format!("init_guest: {g:#x} is not a gadget")))?;
    match info.kind {
        CHECKBOX_KIND => set_selected(a, g, info.state.checked),
        STRING_KIND | INTEGER_KIND => {
            let si = memory::alloc_zeroed(a, SI_SIZE, "StringInfo")?;
            let own = |a: &mut Amiga<U>, block: (u32, u32)| a.gadgets.get_mut(&g).map(|i| i.state.allocs.push(block));
            own(a, (si, SI_SIZE));
            let size = info.state.max_chars + 1;
            let buf = memory::alloc_zeroed(a, size, "StringInfo.Buffer")?;
            own(a, (buf, size));
            let st = a.gadgets.get_mut(&g).map(|i| { (i.state.string_info, i.state.buffer) = (si, buf); i.state.clone() }).unwrap_or_default();
            let m = Refusal::mem;
            a.mem.write_u32(si, buf).map_err(m("StringInfo.Buffer"))?;
            a.mem.write_u16(si + SI_MAX_CHARS, size as u16).map_err(m("StringInfo.MaxChars"))?; // Intuition counts the NUL
            a.mem.write_u32(g + GD_SPECIAL_INFO, si).map_err(m("Gadget.SpecialInfo"))?;
            write_buffer(a, info.kind, &st, &info.text)?;
            Ok(())
        }
        _ => Ok(()),
    }
}

/// The value as this gadget can hold it (truncated to MaxChars, a number checked), or a named refusal for a value its
/// kind cannot hold or a slider level outside its range.
fn normalize(info: &GadgetInfo, v: ControlValue) -> Result<ControlValue, Refusal> {
    let s = &info.state;
    match (info.kind, v) {
        (TEXT_KIND, t @ ControlValue::Text(_)) => Ok(t),
        (CHECKBOX_KIND, c @ ControlValue::Checked(_)) => Ok(c),
        (STRING_KIND, ControlValue::Text(t)) => Ok(ControlValue::Text(truncate(&t, s.max_chars))),
        (INTEGER_KIND, ControlValue::Text(t)) => {
            number(&t)?;
            if t.len() as u32 > s.max_chars { return Err(Refusal(format!("INTEGER_KIND: {t:?} does not fit MaxChars {}", s.max_chars))); }
            Ok(ControlValue::Text(t))
        }
        (SLIDER_KIND, ControlValue::Level(l)) if (s.min..=s.max).contains(&l) => Ok(ControlValue::Level(l)),
        (SLIDER_KIND, ControlValue::Level(l)) => Err(Refusal(format!("SLIDER_KIND: level {l} outside {}..={}", s.min, s.max))),
        (k, v) => Err(Refusal(format!("a {v:?} value cannot be held by a kind-{k} gadget"))),
    }
}

/// Record `v` as the gadget's value, host side and guest side.
fn store<U: GuestUi>(a: &mut Amiga<U>, g: u32, v: &ControlValue) -> Result<(), Refusal> {
    let info = a.gadgets.get_mut(&g).ok_or_else(|| Refusal(format!("{g:#x} is not a GadTools gadget")))?;
    match v {
        ControlValue::Checked(b) => info.state.checked = *b,
        ControlValue::Text(t) => info.text = t.clone(),
        ControlValue::Level(l) => info.state.level = *l,
        ControlValue::Pressed | ControlValue::Scroll { .. } | ControlValue::Cancelled | ControlValue::Choice(_) | ControlValue::Rows { .. } => {}
    }
    let (kind, st) = (info.kind, info.state.clone());
    match v {
        ControlValue::Checked(b) => set_selected(a, g, *b),
        ControlValue::Text(t) if kind == STRING_KIND || kind == INTEGER_KIND => write_buffer(a, kind, &st, t),
        _ => Ok(()),
    }
}

/// A native control changed: update the gadget's state and post IDCMP_GADGETUP (IAddress = the gadget).
pub fn control_changed<U: GuestUi>(a: &mut Amiga<U>, win: u32, g: u32, intent: Intent) -> Result<Delivered, Refusal> {
    let info = a.gadgets[&g].clone();
    let row = translate::lookup(gadtools::TABLE, &control_kind(&info), &intent)?;
    let value = match intent {
        Intent::SetValue(v) => Some(normalize(&info, v)?),
        Intent::CommitText(t) => Some(normalize(&info, ControlValue::Text(t))?),
        _ => None,
    };
    let (class, code) = translate::message(row, 0, value.as_ref()).ok_or_else(|| Refusal("deliver: a GadTools row sent no message".into()))?;
    let changed = matches!(&value, Some(ControlValue::Level(l)) if *l != info.state.level);
    let class = gadtools::filter_class(class, info.kind, changed);
    let msg = if idcmp::subscribed(a, win, class)? { Some(idcmp::reserve(a)?) } else { None };
    if let Some(v) = &value {
        if let Err(e) = store(a, g, v) { if let Some(m) = msg { let _ = a.heap.free(m, idcmp::INTUIMESSAGE_SIZE); } return Err(e); }
    }
    match msg { Some(m) => idcmp::queue(a, win, m, class, code, g), None => Ok(Delivered::IgnoredUnsubscribed) }
}

/// GT_SetGadgetAttrsA(a0 = gadget, a1 = window, a2 = requester, a3 = tags): GTTX_Text (text), GTCB_Checked, GTST_String,
/// GTIN_Number and GTSL_Level change the native control and the guest-visible state. A tag the gadget's kind does not
/// have is refused; GA_Disabled and GTSL_Min/Max after creation are refused by name (no native call for them), after
/// the other tags were applied. Other tags are ignored, as GadTools ignores tags a kind does not know.
pub fn set_gadget_attrs<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let g = cpu.areg(0);
    let t = tags::walk(&a.mem, cpu.areg(3))?;
    let info = a.gadgets.get(&g).cloned().ok_or_else(|| Refusal(format!("GT_SetGadgetAttrsA: {g:#x} is not a GadTools gadget")))?;
    let (mut changed, mut unsupported) = (String::new(), Vec::new());
    for (tag, data) in t {
        let v = match (tag, info.kind) {
            (GTTX_TEXT, TEXT_KIND) => ControlValue::Text(opt_str(a, data, "GTTX_Text")?),
            (GTCB_CHECKED, CHECKBOX_KIND) => ControlValue::Checked(data != 0),
            (GTST_STRING, STRING_KIND) => ControlValue::Text(opt_str(a, data, "GTST_String")?),
            (GTIN_NUMBER, INTEGER_KIND) => ControlValue::Text((data as i32).to_string()),
            (GTSL_LEVEL, SLIDER_KIND) => ControlValue::Level((data as i32).clamp(info.state.min, info.state.max)),
            (GTTX_TEXT | GTCB_CHECKED | GTST_STRING | GTIN_NUMBER | GTSL_LEVEL, k) => {
                return Err(Refusal(format!("GT_SetGadgetAttrsA: tag {tag:#x} on a kind-{k} gadget")));
            }
            (GA_DISABLED, _) => { unsupported.push("GA_Disabled: GuestUi has no control enabled-state call"); continue }
            (GTSL_MIN | GTSL_MAX, SLIDER_KIND) => { unsupported.push("GTSL_Min/GTSL_Max after creation: GuestUi has no slider-range update"); continue }
            _ => continue,
        };
        let v = normalize(&info, v)?;
        if let Some((win, ctl)) = a.gt_ctls.get(u64::from(g)).map(|p| (p.window, p.native)) {
            a.ui.set_control(win, ctl, v.clone()).map_err(Refusal::ui)?;
            a.gt_ctls.adopt(u64::from(g), &v);
        }
        store(a, g, &v)?;
        changed += &format!(" {v:?}");
    }
    if !unsupported.is_empty() { return Err(Refusal(format!("GT_SetGadgetAttrsA: {}", unsupported.join("; ")))); }
    Ok(format!("[gadtools] GT_SetGadgetAttrsA {g:#x}{changed}"))
}

/// GT_GetGadgetAttrsA(a0 = gadget, a1 = window, a2 = requester, a3 = tags) -> how many tags were filled. Each tag's
/// ti_Data points at a longword to fill: GTCB_Checked, GTSL_Level / Min / Max, GTIN_Number, and GTST_String (a pointer
/// to the gadget's guest buffer, holding the text the native side last committed). A tag its kind does not have is not
/// counted and is recorded as a refusal.
pub fn get_gadget_attrs<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let g = cpu.areg(0);
    if g == 0 || cpu.areg(3) == 0 { cpu.set_d0(0); return Ok("[gadtools] GT_GetGadgetAttrsA NULL gadget or tags -> 0".into()); }
    let info = a.gadgets.get(&g).cloned().ok_or_else(|| Refusal(format!("GT_GetGadgetAttrsA: {g:#x} is not a GadTools gadget")))?;
    let s = &info.state;
    let mut n = 0;
    for (tag, dest) in tags::walk(&a.mem, cpu.areg(3))? {
        let v = match (tag, info.kind) {
            (GTCB_CHECKED, CHECKBOX_KIND) => s.checked as u32,
            (GTSL_LEVEL, SLIDER_KIND) => s.level as u32,
            (GTSL_MIN, SLIDER_KIND) => s.min as u32,
            (GTSL_MAX, SLIDER_KIND) => s.max as u32,
            (GTIN_NUMBER, INTEGER_KIND) => number(&info.text)? as u32,
            (GTST_STRING, STRING_KIND) => s.buffer,
            _ => { a.refused.push(format!("gadtools.library/GT_GetGadgetAttrsA: tag {tag:#x} is not readable on a kind-{} gadget", info.kind)); continue }
        };
        a.mem.write_u32(dest, v).map_err(Refusal::mem("GT_GetGadgetAttrsA destination"))?;
        n += 1;
    }
    cpu.set_d0(n);
    Ok(format!("[gadtools] GT_GetGadgetAttrsA {g:#x} -> {n} tags"))
}
