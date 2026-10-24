//! Normalized model types (plan §4: WindowModel, ControlModel, MenuModel, CallbackToken...).

use alloc::vec::Vec;
use alloc::string::String;
/// Launch-local object identity plus generation (a stale ref is refused, never aliased).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UiObjectRef { pub id: u32, pub generation: u32 }

/// Opaque adapter-owned continuation identity: lets the engine map an event back to its
/// guest object (e.g. a Gadget pointer) without the UI side ever seeing a guest address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CallbackToken(pub u64);

/// A rectangle in the GUEST coordinate domain (distinct from the native domain, which the
/// real implementation owns).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestRect { pub left: i32, pub top: i32, pub width: i32, pub height: i32 }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowRole { Document, Dialog, Utility }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowModel {
    pub title: String,
    pub rect: GuestRect,
    pub role: WindowRole,
    pub closable: bool,
    pub resizable: bool,
    pub token: CallbackToken,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    pub label: String,
    pub enabled: bool,
    /// Keyboard shortcut character (command-key meaning), if any.
    pub shortcut: Option<char>,
    /// `Some(checked)` for a checkable item.
    pub check: Option<bool>,
    pub sub_items: Vec<MenuItem>,
    pub token: CallbackToken,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Menu { pub title: String, pub enabled: bool, pub items: Vec<MenuItem> }

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MenuModel { pub menus: Vec<Menu> }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlKind {
    Button,
    Checkbox { checked: bool },
    String { text: String, max_len: u32 },
    Slider { min: i32, max: i32, value: i32 },
    /// Read-only text the guest updates (GadTools TEXT_KIND; `framed` = GTTX_Border's recessed box).
    Label { text: String, framed: bool },
    /// A two-state push button that stays down (an Intuition GACT_TOGGLESELECT boolean gadget).
    ToggleButton { on: bool },
    /// A scrollbar over `total` units of which `visible` show, the first shown being `top` (all in the guest's own units).
    /// `top` is at most `total - visible`; a `visible >= total` bar has nothing to scroll. The host draws the knob, the
    /// arrows are separate [`ControlKind::ArrowButton`]s when the guest has them.
    ScrollBar { vertical: bool, total: u32, visible: u32, top: u32 },
    /// A one-line text or integer field. `editable` false = shown but not typed into; `integer` = digits (and a leading
    /// minus) only; `max_len` counts characters.
    Field { text: String, max_len: u32, editable: bool, integer: bool },
    /// A static framed group around or behind other controls (frameiclass; no interaction, no events). Plan 16 §2a.1 `Group`.
    Group { title: String, style: GroupStyle },
    /// A button that shows a direction arrow instead of a label (scrollbar arrows, spinners).
    ArrowButton { dir: ArrowDir },
    /// An invisible hit area: no visuals, but pointer press/release inside it are events (a gadget with nothing to draw).
    Hotspot,
    /// One choice among `labels` (a MUI Cycle / Radio / Register, a GadTools MX, a Mac pop-up or radio button): `selected` indexes `labels`,
    /// which are the guest's own strings in the guest's own order, or is [`CHOICE_NONE`] for NO label chosen (a Radio style only: a radio
    /// group whose buttons are all off). The `style` is how the host offers them.
    Choice { style: ChoiceStyle, labels: Vec<String>, selected: u32 },
    /// A table of text rows under optional `columns` headings (a MUI List/Listview): every row has one cell per column
    /// (a headless single-column list has one). `selected` is a row index into `rows`; `top` the first row in view.
    List { columns: Vec<String>, rows: Vec<Vec<String>>, selected: Option<u32>, top: u32 },
    /// A read-only progress bar: `value` of `max` (a MUI Gauge). `max` is at least 1.
    Gauge { value: u32, max: u32 },
    /// A palette: one of several colour swatches the user picks from (a GadTools PALETTE_KIND). `colors` are `0x00RRGGBB` in the guest's own
    /// order (never empty); `selected` indexes them. The host draws the swatches and reports the pick as `SetValue(Choice(index))`.
    Swatches { colors: Vec<u32>, selected: u32 },
}

impl ControlKind {
    /// The shape rules a host enforces when a control is added (`add_control` refuses a kind that breaks them, by name).
    pub fn validate(&self) -> Result<(), UiRefusal> {
        match self {
            ControlKind::Slider { min, max, value } if min > max || value < min || value > max => Err(UiRefusal::InvalidGeometry("slider range")),
            ControlKind::Choice { style, selected, .. } if *selected == CHOICE_NONE && !style.allows_none() => Err(UiRefusal::InvalidGeometry("a choice with nothing selected is a radio's")),
            ControlKind::Choice { labels, selected, .. } if *selected != CHOICE_NONE && !labels.is_empty() && *selected as usize >= labels.len() => Err(UiRefusal::InvalidGeometry("choice index past its labels")),
            ControlKind::List { columns, rows, selected, .. } => {
                let width = columns.len().max(1);
                if rows.iter().any(|r| r.len() != width) { return Err(UiRefusal::InvalidGeometry("list row with the wrong number of cells")); }
                if selected.is_some_and(|i| i as usize >= rows.len()) { return Err(UiRefusal::InvalidGeometry("list selection past its rows")); }
                Ok(())
            }
            ControlKind::Gauge { value, max } if *max == 0 || value > max => Err(UiRefusal::InvalidGeometry("gauge value outside 0..=max")),
            ControlKind::Swatches { colors, selected } if colors.is_empty() || *selected as usize >= colors.len() => Err(UiRefusal::InvalidGeometry("swatch selection outside its colours")),
            _ => Ok(()),
        }
    }
}

/// How a [`ControlKind::Choice`] is offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChoiceStyle { Cycle, Radio, Popup, Tabs }

/// [`ControlKind::Choice`]'s `selected` when no label is chosen: a Mac radio button that is off, a GadTools MX nothing has been picked in.
/// `u32::MAX` can never be a label index (a label list that long is refused by `add_control` long before), so it needs no second field.
pub const CHOICE_NONE: u32 = u32::MAX;

impl ChoiceStyle {
    /// Only a radio offers "none of these": a cycle, a pop-up and a tab strip always show one of their labels.
    pub const fn allows_none(self) -> bool { matches!(self, ChoiceStyle::Radio) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrowDir { Up, Down, Left, Right }

/// What a group frame looks like, as a role (the host maps it onto its own kit parts; Gute Form groups are flat, so the style may be metadata).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupStyle { Plain, Recessed, Raised }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlModel {
    pub kind: ControlKind,
    pub label: String,
    pub rect: GuestRect,
    pub enabled: bool,
    pub token: CallbackToken,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlValue { Pressed, Checked(bool), Text(String), Level(i32), Scroll { total: u32, visible: u32, top: u32 },
    /// The selected index of a [`ControlKind::Choice`] or [`ControlKind::Swatches`].
    Choice(u32),
    /// The whole content of a [`ControlKind::List`].
    Rows { rows: Vec<Vec<String>>, selected: Option<u32>, top: u32 },
    /// The gesture ended outside the control (no activation, nothing changed). Legacy value form; `Intent::Release { inside: false }` is the event.
    Cancelled }

impl ControlModel {
    /// Apply `value` to this model the way a host applies `set_control`: the shared rule for what each kind can hold.
    /// A value the kind cannot hold is `UiRefusal::Unsupported`; a `Scroll` with `top` past the end is clamped to it.
    pub fn apply(&mut self, value: &ControlValue) -> Result<(), UiRefusal> {
        match (&mut self.kind, value) {
            (_, ControlValue::Pressed | ControlValue::Cancelled) => {}
            (ControlKind::Checkbox { checked }, ControlValue::Checked(b)) => *checked = *b,
            (ControlKind::ToggleButton { on }, ControlValue::Checked(b)) => *on = *b,
            (ControlKind::String { text, .. } | ControlKind::Label { text, .. } | ControlKind::Field { text, .. }, ControlValue::Text(t)) => *text = t.clone(),
            (ControlKind::Slider { min, max, value }, ControlValue::Level(l)) => {
                if l < min || l > max { return Err(UiRefusal::InvalidGeometry("slider level outside its range")); }
                *value = *l;
            }
            (ControlKind::ScrollBar { total: t, visible: v, top: p, .. }, ControlValue::Scroll { total, visible, top }) => {
                (*t, *v) = (*total, *visible);
                *p = (*top).min(total.saturating_sub(*visible));
            }
            (ControlKind::Choice { style, labels, selected }, ControlValue::Choice(i)) => {
                if *i == CHOICE_NONE {
                    if !style.allows_none() { return Err(UiRefusal::InvalidGeometry("a choice with nothing selected is a radio's")); }
                } else if (*i as usize) >= labels.len() { return Err(UiRefusal::InvalidGeometry("choice index past its labels")); }
                *selected = *i;
            }
            (ControlKind::List { columns, rows: r, selected, top }, ControlValue::Rows { rows, selected: sel, top: t }) => {
                let width = columns.len().max(1);
                if rows.iter().any(|row| row.len() != width) { return Err(UiRefusal::InvalidGeometry("list row with the wrong number of cells")); }
                if sel.is_some_and(|i| i as usize >= rows.len()) { return Err(UiRefusal::InvalidGeometry("list selection past its rows")); }
                (*r, *selected, *top) = (rows.clone(), *sel, (*t).min(rows.len().saturating_sub(1) as u32));
            }
            (ControlKind::Swatches { colors, selected }, ControlValue::Choice(i)) => {
                if (*i as usize) >= colors.len() { return Err(UiRefusal::InvalidGeometry("swatch index past its colours")); }
                *selected = *i;
            }
            (ControlKind::Gauge { value, max }, ControlValue::Level(l)) => {
                if *l < 0 || *l as u32 > *max { return Err(UiRefusal::InvalidGeometry("gauge value outside 0..=max")); }
                *value = *l as u32;
            }
            _ => return Err(UiRefusal::Unsupported("a value the control's kind cannot hold")),
        }
        Ok(())
    }
}

/// Named refusals (the plan: refuse by name, never silently discard).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiRefusal {
    UnknownObject(UiObjectRef),
    StaleGeneration(UiObjectRef),
    InvalidGeometry(&'static str),
    Unsupported(&'static str),
}
impl core::fmt::Display for UiRefusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            UiRefusal::UnknownObject(r) => write!(f, "unknown UI object {}", r.id),
            UiRefusal::StaleGeneration(r) => write!(f, "stale UI object {} (generation {})", r.id, r.generation),
            UiRefusal::InvalidGeometry(w) => write!(f, "invalid geometry: {w}"),
            UiRefusal::Unsupported(w) => write!(f, "unsupported: {w}"),
        }
    }
}
impl core::error::Error for UiRefusal {}
