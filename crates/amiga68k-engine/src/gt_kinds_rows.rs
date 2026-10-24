//! The GadTools kinds that hold ROWS or a POSITION, projected onto the guestui roles `Choice`, `List`, `ScrollBar` and `Label`:
//!
//! | GadTools kind | native control | tags (AROS include/libraries/gadtools.h) |
//! |---|---|---|
//! | CYCLE_KIND (7) | `Choice{Cycle}` | GTCY_Labels, GTCY_Active |
//! | MX_KIND (5) | `Choice{Radio}` | GTMX_Labels, GTMX_Active (GTMX_Spacing is layout: the host lays the radios out) |
//! | LISTVIEW_KIND (4) | `List` (one headless column) | GTLV_Labels (a `struct List` of Nodes, `ln_Name` = the row; `~0` = detached), GTLV_Top, GTLV_Selected, GTLV_ReadOnly |
//! | SCROLLER_KIND (9) | `ScrollBar` | GTSC_Top / Total / Visible, PGA_Freedom (LORIENT_VERT, anything but LORIENT_HORIZ = vertical; default horizontal) |
//! | NUMBER_KIND (6) | `Label` (read-only) | GTNM_Number, GTNM_Border, GTNM_Format (default `%ld`) |
//! | PALETTE_KIND (8) | `Swatches` | GTPA_Depth (2^depth colours) or GTPA_NumColors (wins; default 2), GTPA_Color (a PEN), GTPA_ColorOffset, GTPA_ColorTable |
//!
//! PALETTE_KIND (paletteclass.c): the colours are the pens `offset + i` (or `ColorTable[i]`) read from the screen palette when the gadget is made or
//! its colour set changes (a later LoadRGB does not repaint the swatches); GTPA_Color is a PEN, held as an index (`pd_Color`) and read back as a pen
//! (`GetPalettePen`); a pick ends with Code = the PEN. GTPA_IndicatorWidth / Height and GA_LabelPlace are layout the host owns.
//!
//! Facts (AROS workbench/libs/gadtools, APL, read for behaviour only): `cycleclass.c` (GTCY_Labels resets the active entry to 0;
//! a click ends with GMR_VERIFY and Code = active), `mxclass.c` (GACT_IMMEDIATE, so the message is GADGETDOWN; OM_SET knows only
//! GTMX_Active, so the labels are fixed at creation), `listviewclass.c` (GTLV_Labels resets Top to 0 and keeps Selected; a click
//! ends with Code = the selected row; GTLV_Top is clamped into the list; a GTLV_ShowSelected gadget is mirrored into by the
//! class), `scrollerclass.c` / `gt_filterimsg.c` (Code = GTSC_Top; a release that CHANGED the position reaches the program as
//! IDCMP_MOUSEMOVE), `textclass.c` (NUMBER_KIND runs RawDoFmt(format, &number) on every change).
//! Where AROS leaves something undefined the engine chooses and says so: GTLV_Selected starts at `~0` (AmigaOS autodoc), because
//! AROS's zeroed instance would select row 0; a list's Top is clamped to its last row (the real bound needs the shown height).
//!
//! NUMBER_KIND's format is the guest's own RawDoFmt template, so it is held to the one shape the number can feed: literal text
//! plus at most ONE of `%[-][0][width]` `[l]` `d|u|x|X` (the engine's `fmt::format` does the formatting, so `%d` reads 16 bits and
//! `%lx` prints upper case exactly as exec does). Anything else (`%s`, `%b`, `%c`, a second directive, a width beyond 256) would
//! read a guest pointer or argument the number does not supply: a named refusal.
use alloc::{format, string::{String, ToString}, vec, vec::Vec};
use crate::{boopsi_prop::PGA_FREEDOM, fmt, gadtools::GadgetInfo, gt_kinds::*, machine::Amiga, memory, tags, Refusal};
use guestui_abi::{ChoiceStyle, ControlKind, ControlValue, GuestUi, Intent};

const GT_TAGBASE: u32 = tags::TAG_USER + 0x0008_0000;
pub const GTLV_TOP: u32 = GT_TAGBASE + 5;
pub const GTLV_LABELS: u32 = GT_TAGBASE + 6;
pub const GTLV_READONLY: u32 = GT_TAGBASE + 7;
pub const GTMX_LABELS: u32 = GT_TAGBASE + 9;
pub const GTMX_ACTIVE: u32 = GT_TAGBASE + 10;
pub const GTNM_NUMBER: u32 = GT_TAGBASE + 13;
pub const GTCY_LABELS: u32 = GT_TAGBASE + 14;
pub const GTCY_ACTIVE: u32 = GT_TAGBASE + 15;
pub const GTSC_TOP: u32 = GT_TAGBASE + 21;
pub const GTSC_TOTAL: u32 = GT_TAGBASE + 22;
pub const GTSC_VISIBLE: u32 = GT_TAGBASE + 23;
pub const GTSC_ARROWS: u32 = GT_TAGBASE + 59;
pub const GTLV_SHOWSELECTED: u32 = GT_TAGBASE + 53;
pub const GTLV_SELECTED: u32 = GT_TAGBASE + 54;
pub const GTNM_BORDER: u32 = GT_TAGBASE + 58;
pub const GTPA_DEPTH: u32 = GT_TAGBASE + 16;
pub const GTPA_COLOR: u32 = GT_TAGBASE + 17;
pub const GTPA_COLOR_OFFSET: u32 = GT_TAGBASE + 18;
pub const GTPA_NUM_COLORS: u32 = GT_TAGBASE + 70;
pub const GTPA_COLOR_TABLE: u32 = GT_TAGBASE + 82;
pub const GTNM_FORMAT: u32 = GT_TAGBASE + 75;
/// intuition/gadgetclass.h: LORIENT_HORIZ (the default); any other PGA_Freedom value is vertical (gadgets.c makescroller).
const LORIENT_HORIZ: u32 = 1;
/// GTLV_Selected / the `GTLV_Labels` "detached" marker: no row / no list.
pub const NONE: u32 = u32::MAX;
/// A walk over a guest array or list stops here: a longer one is a cycle, not a program.
const MAX_WALK: usize = 1 << 20;
const MAX_WIDTH: usize = 256;

/// What the host holds for the five kinds.
#[derive(Debug, Clone)]
pub struct RowState {
    pub labels: Vec<String>,
    /// The guest's Labels pointer, read back by GT_GetGadgetAttrsA.
    pub labels_ptr: u32,
    /// The active entry / selected row (`NONE` = no row).
    pub selected: u32,
    pub top: u32,
    pub total: u32,
    pub visible: u32,
    pub vertical: bool,
    pub read_only: bool,
    pub number: i32,
    pub format: String,
    pub format_ptr: u32,
    /// PALETTE_KIND: how many colours, the pen of index 0 (no table), the guest's GTPA_ColorTable pointer and its pens, and the swatch colours (0xRRGGBB).
    pub num_colors: u32,
    pub color_offset: u8,
    pub table_ptr: u32,
    pub table: Vec<u8>,
    pub colors: Vec<u32>,
}
impl Default for RowState {
    fn default() -> Self {
        Self { labels: vec![], labels_ptr: 0, selected: 0, top: 0, total: 0, visible: 0, vertical: false, read_only: false, number: 0, format: "%ld".to_string(), format_ptr: 0, num_colors: 2, color_offset: 0, table_ptr: 0, table: vec![], colors: vec![] }
    }
}

pub fn owns(kind: u32) -> bool { matches!(kind, CYCLE_KIND | MX_KIND | LISTVIEW_KIND | SCROLLER_KIND | NUMBER_KIND | PALETTE_KIND) }

fn kind_name(kind: u32) -> &'static str {
    match kind { CYCLE_KIND => "CYCLE_KIND", MX_KIND => "MX_KIND", LISTVIEW_KIND => "LISTVIEW_KIND", SCROLLER_KIND => "SCROLLER_KIND", PALETTE_KIND => "PALETTE_KIND", _ => "NUMBER_KIND" }
}

/// A NULL-terminated STRPTR array (GTCY_Labels / GTMX_Labels); NULL = no labels.
fn read_str_array<U: GuestUi>(a: &Amiga<U>, ptr: u32) -> Result<Vec<String>, Refusal> {
    let mut labels = Vec::new();
    if ptr == 0 { return Ok(labels); }
    for i in 0..MAX_WALK {
        let p = a.mem.read_u32(ptr + 4 * i as u32).map_err(Refusal::mem("Labels array"))?;
        if p == 0 { return Ok(labels); }
        labels.push(a.mem.read_cstr(p).map_err(Refusal::mem("Labels string"))?);
    }
    Err(Refusal(format!("Labels: a STRPTR array of more than {MAX_WALK} entries (no NULL terminator?)")))
}

/// The `ln_Name` of every Node of an Exec List (lh_Head 0; a Node's ln_Succ 0, ln_Name 10; the tail marker has ln_Succ 0).
/// NULL and `~0` are "no list".
fn read_list_names<U: GuestUi>(a: &Amiga<U>, list: u32) -> Result<Vec<String>, Refusal> {
    let mut names = Vec::new();
    if list == 0 || list == NONE { return Ok(names); }
    let mut node = a.mem.read_u32(list).map_err(Refusal::mem("GTLV_Labels List.lh_Head"))?;
    for _ in 0..MAX_WALK {
        let succ = a.mem.read_u32(node).map_err(Refusal::mem("GTLV_Labels ln_Succ"))?;
        if succ == 0 { return Ok(names); }
        let name = a.mem.read_u32(node + 10).map_err(Refusal::mem("GTLV_Labels ln_Name"))?;
        names.push(if name == 0 { String::new() } else { a.mem.read_cstr(name).map_err(Refusal::mem("GTLV_Labels name"))? });
        node = succ;
    }
    Err(Refusal(format!("GTLV_Labels: an Exec List of more than {MAX_WALK} nodes (cyclic?)")))
}

fn max_top(rs: &RowState) -> u32 { rs.total.saturating_sub(rs.visible) }

/// The state's invariants, applied after any change: a refusal for an active entry outside the labels, the clamps AROS makes.
fn settle(kind: u32, rs: &mut RowState, what: &str) -> Result<(), Refusal> {
    match kind {
        CYCLE_KIND | MX_KIND => {
            let ok = if rs.labels.is_empty() { rs.selected == 0 } else { (rs.selected as usize) < rs.labels.len() };
            if !ok { return Err(Refusal(format!("{what}: {} active entry {} outside its {} labels", kind_name(kind), rs.selected, rs.labels.len()))); }
        }
        LISTVIEW_KIND => rs.top = rs.top.min(rs.labels.len().saturating_sub(1) as u32),
        SCROLLER_KIND => rs.top = rs.top.min(max_top(rs)),
        PALETTE_KIND if rs.selected >= rs.num_colors => return Err(Refusal(format!("{what}: PALETTE_KIND colour index {} outside its {} colours", rs.selected, rs.num_colors))),
        _ => {}
    }
    Ok(())
}

/// The kind's initial state (and a NUMBER_KIND's text) from the CreateGadgetA tags.
pub fn from_tags<U: GuestUi>(a: &mut Amiga<U>, kind: u32, t: &[(u32, u32)], text: &mut String) -> Result<RowState, Refusal> {
    let get = |tag| tags::get(t, tag);
    let mut rs = RowState::default();
    match kind {
        CYCLE_KIND | MX_KIND => {
            let (lt, at) = if kind == CYCLE_KIND { (GTCY_LABELS, GTCY_ACTIVE) } else { (GTMX_LABELS, GTMX_ACTIVE) };
            rs.labels_ptr = get(lt).unwrap_or(0);
            rs.labels = read_str_array(a, rs.labels_ptr)?;
            rs.selected = get(at).unwrap_or(0);
        }
        LISTVIEW_KIND => {
            if get(GTLV_SHOWSELECTED).is_some_and(|g| g != NONE) {
                return Err(Refusal("CreateGadgetA: LISTVIEW_KIND GTLV_ShowSelected: mirroring the selected row into another gadget is not bridged".into()));
            }
            rs.labels_ptr = get(GTLV_LABELS).unwrap_or(NONE);
            rs.labels = read_list_names(a, rs.labels_ptr)?;
            rs.top = get(GTLV_TOP).unwrap_or(0);
            rs.selected = row_word(get(GTLV_SELECTED).unwrap_or(NONE));
            rs.read_only = get(GTLV_READONLY).unwrap_or(0) != 0;
        }
        SCROLLER_KIND => {
            (rs.top, rs.total, rs.visible) = (get(GTSC_TOP).unwrap_or(0), get(GTSC_TOTAL).unwrap_or(0), get(GTSC_VISIBLE).unwrap_or(2));
            rs.vertical = get(PGA_FREEDOM).is_some_and(|f| f != LORIENT_HORIZ);
            if get(GTSC_ARROWS).is_some_and(|w| w != 0) {
                a.refused.push("gadtools.library/CreateGadgetA: GTSC_Arrows: the arrow gadgets are not made; the native bar scrolls without them".into());
            }
        }
        PALETTE_KIND => palette_from_tags(a, t, &mut rs)?,
        _ => {
            rs.number = get(GTNM_NUMBER).unwrap_or(0) as i32;
            rs.format_ptr = get(GTNM_FORMAT).unwrap_or(0);
            if rs.format_ptr != 0 { rs.format = a.mem.read_cstr(rs.format_ptr).map_err(Refusal::mem("GTNM_Format"))?; }
            *text = format_number(a, rs.number, &rs.format)?;
        }
    }
    settle(kind, &mut rs, "CreateGadgetA")?;
    Ok(rs)
}

/// GTLV_Selected is a WORD in the class: `~0` and any negative word are "none".
fn row_word(v: u32) -> u32 { if (v as u16 as i16) < 0 { NONE } else { v as u16 as u32 } }

/// Check a RawDoFmt template against the one shape a number can feed (see the module doc).
fn check_template(fmt: &str) -> Result<(), Refusal> {
    let bad = |why: String| Refusal(format!("NUMBER_KIND: GTNM_Format {fmt:?}: {why}"));
    let b = fmt.as_bytes();
    let (mut i, mut directives) = (0, 0);
    while i < b.len() {
        if b[i] != b'%' { i += 1; continue; }
        i += 1;
        if b.get(i) == Some(&b'%') { i += 1; continue; }
        while matches!(b.get(i), Some(b'-' | b'0')) { i += 1; }
        let start = i;
        while b.get(i).is_some_and(u8::is_ascii_digit) { i += 1; }
        if i - start > 3 || fmt[start..i].parse::<usize>().unwrap_or(0) > MAX_WIDTH { return Err(bad(format!("a width beyond {MAX_WIDTH}"))); }
        if b.get(i) == Some(&b'l') { i += 1; }
        match b.get(i) {
            Some(b'd' | b'u' | b'x' | b'X') => directives += 1,
            other => return Err(bad(format!("directive {:?} needs an argument a number does not supply (only d, u, x, X)", other.map(|c| *c as char)))),
        }
        i += 1;
    }
    if directives > 1 { return Err(bad("more than one directive for a single number".into())); }
    Ok(())
}

/// textclass.c: RawDoFmt(format, &number). The number goes to a four-byte guest block so the engine's own formatter reads it.
fn format_number<U: GuestUi>(a: &mut Amiga<U>, number: i32, fmt_str: &str) -> Result<String, Refusal> {
    check_template(fmt_str)?;
    let at = memory::alloc_zeroed(a, 4, "GTNM_Number")?;
    let done = (|| {
        a.mem.write_u32(at, number as u32).map_err(Refusal::mem("GTNM_Number"))?;
        fmt::format(&a.mem, fmt_str.as_bytes(), at, false)
    })();
    a.heap.free(at, 4).map_err(|e| Refusal(format!("GTNM_Number block: {e}")))?;
    Ok(String::from_utf8_lossy(&done?.bytes).into_owned())
}

/// The native control a gadget of one of the five kinds is.
pub fn control_kind(info: &GadgetInfo) -> ControlKind {
    let s = &info.state.rows;
    match info.kind {
        CYCLE_KIND => ControlKind::Choice { style: ChoiceStyle::Cycle, labels: s.labels.clone(), selected: s.selected },
        MX_KIND => ControlKind::Choice { style: ChoiceStyle::Radio, labels: s.labels.clone(), selected: s.selected },
        LISTVIEW_KIND => ControlKind::List {
            columns: vec![],
            rows: s.labels.iter().map(|l| vec![l.clone()]).collect(),
            selected: (s.selected as usize).lt(&s.labels.len()).then_some(s.selected),
            top: s.top,
        },
        SCROLLER_KIND => ControlKind::ScrollBar { vertical: s.vertical, total: s.total, visible: s.visible.min(s.total), top: s.top },
        PALETTE_KIND => ControlKind::Swatches { colors: s.colors.clone(), selected: s.selected },
        _ => ControlKind::Label { text: info.text.clone(), framed: info.framed },
    }
}

/// The value a native intent asks for, checked against the gadget (a stale or impossible one is a named refusal), or `None`
/// when this kind takes the generic path.
pub fn intent_value(info: &GadgetInfo, intent: &Intent) -> Result<Option<ControlValue>, Refusal> {
    let (s, name) = (&info.state.rows, kind_name(info.kind));
    let rows_with = |selected: u32, top: u32| match control_kind(info) {
        ControlKind::List { rows, .. } => ControlValue::Rows { rows, selected: (selected as usize).lt(&s.labels.len()).then_some(selected), top },
        _ => unreachable!("a LISTVIEW_KIND gadget projects as a List"),
    };
    Ok(Some(match (info.kind, intent) {
        (PALETTE_KIND, Intent::SetValue(ControlValue::Choice(i))) => {
            if *i >= s.num_colors { return Err(Refusal(format!("{name}: colour {i} outside its {} colours", s.num_colors))); }
            ControlValue::Choice(*i)
        }
        (CYCLE_KIND | MX_KIND, Intent::SetValue(ControlValue::Choice(i))) => {
            if *i as usize >= s.labels.len() { return Err(Refusal(format!("{name}: choice {i} outside its {} labels", s.labels.len()))); }
            ControlValue::Choice(*i)
        }
        (LISTVIEW_KIND, Intent::SelectRow(r)) => {
            if s.read_only { return Err(Refusal("LISTVIEW_KIND: GTLV_ReadOnly list refuses a row selection".into())); }
            if *r as usize >= s.labels.len() { return Err(Refusal(format!("{name}: row {r} outside its {} rows", s.labels.len()))); }
            rows_with(*r, s.top)
        }
        (LISTVIEW_KIND, Intent::ScrollTo(t)) => {
            if *t as usize >= s.labels.len().max(1) { return Err(Refusal(format!("{name}: top {t} outside its {} rows", s.labels.len()))); }
            rows_with(s.selected, *t)
        }
        (SCROLLER_KIND, Intent::ScrollTo(t)) => {
            if *t > max_top(s) { return Err(Refusal(format!("{name}: top {t} past {} (total {} - visible {})", max_top(s), s.total, s.visible))); }
            ControlValue::Scroll { total: s.total, visible: s.visible, top: *t }
        }
        (k, _) if owns(k) => return Err(Refusal(format!("{name}: this intent means nothing for the kind"))),
        _ => return Ok(None),
    }))
}

/// Record a value from a native intent in the gadget's state.
pub fn store(info: &mut GadgetInfo, v: &ControlValue) {
    let s = &mut info.state.rows;
    match v {
        ControlValue::Choice(i) => s.selected = *i,
        ControlValue::Rows { selected, top, .. } => (s.selected, s.top) = (selected.unwrap_or(NONE), *top),
        ControlValue::Scroll { top, .. } => s.top = *top,
        _ => {}
    }
}

/// Did the position change (a scroller release that did is a MOUSEMOVE, gt_filterimsg.c)?
pub fn moved(info: &GadgetInfo, v: &ControlValue) -> bool {
    matches!((info.kind, v), (SCROLLER_KIND, ControlValue::Scroll { top, .. }) if *top != info.state.rows.top)
}

/// One GT_SetGadgetAttrsA tag. `Ok(None)` = not a tag of this kind. Otherwise the gadget with the tag applied (the caller
/// commits it after the host has taken it), or a refusal; nothing is changed here.
pub fn set_attr<U: GuestUi>(a: &mut Amiga<U>, info: &GadgetInfo, tag: u32, data: u32) -> Result<Option<GadgetInfo>, Refusal> {
    let mut next = info.clone();
    let rs = &mut next.state.rows;
    match (info.kind, tag) {
        (CYCLE_KIND, GTCY_LABELS) => { rs.labels = read_str_array(a, data)?; (rs.labels_ptr, rs.selected) = (data, 0) }
        (MX_KIND, GTMX_LABELS) => return Err(Refusal("GT_SetGadgetAttrsA: MX_KIND GTMX_Labels: the class takes its labels at creation only (mxclass.c OM_SET)".into())),
        (CYCLE_KIND, GTCY_ACTIVE) | (MX_KIND, GTMX_ACTIVE) => rs.selected = data,
        (LISTVIEW_KIND, GTLV_LABELS) => { rs.labels = read_list_names(a, data)?; (rs.labels_ptr, rs.top) = (data, 0) }
        (LISTVIEW_KIND, GTLV_SELECTED) => rs.selected = row_word(data),
        (LISTVIEW_KIND, GTLV_TOP) => rs.top = if (data as i32) < 0 { 0 } else { data },
        (SCROLLER_KIND, GTSC_TOP) => rs.top = data,
        (SCROLLER_KIND, GTSC_TOTAL) => rs.total = data,
        (SCROLLER_KIND, GTSC_VISIBLE) => rs.visible = data,
        (NUMBER_KIND, GTNM_NUMBER) => rs.number = data as i32,
        (NUMBER_KIND, GTNM_FORMAT) => {
            rs.format_ptr = data;
            rs.format = if data == 0 { "%ld".to_string() } else { a.mem.read_cstr(data).map_err(Refusal::mem("GTNM_Format"))? };
        }
        (PALETTE_KIND, GTPA_COLOR) => rs.selected = palette_index_of_pen(rs, data)?,
        (PALETTE_KIND, GTPA_DEPTH) => { rs.num_colors = palette_depth(data)?; palette_reset(rs) }
        (PALETTE_KIND, GTPA_NUM_COLORS) => { rs.num_colors = data as u16 as u32; palette_reset(rs) }
        (PALETTE_KIND, GTPA_COLOR_OFFSET) => rs.color_offset = data as u8,
        (PALETTE_KIND, GTPA_COLOR_TABLE) => { rs.table_ptr = data; rs.table = Vec::new() }
        _ => return Ok(None),
    }
    if info.kind == PALETTE_KIND { palette_refresh(a, &mut next.state.rows)?; }
    settle(info.kind, &mut next.state.rows, "GT_SetGadgetAttrsA")?;
    if info.kind == NUMBER_KIND { next.text = format_number(a, next.state.rows.number, &next.state.rows.format)?; }
    Ok(Some(next))
}

/// One GT_GetGadgetAttrsA tag of these kinds: the longword to store, or `None` for a tag the kind does not have.
pub fn get_attr(info: &GadgetInfo, tag: u32) -> Option<u32> {
    let s = &info.state.rows;
    Some(match (info.kind, tag) {
        (CYCLE_KIND, GTCY_LABELS) | (MX_KIND, GTMX_LABELS) | (LISTVIEW_KIND, GTLV_LABELS) => s.labels_ptr,
        (CYCLE_KIND, GTCY_ACTIVE) | (MX_KIND, GTMX_ACTIVE) | (LISTVIEW_KIND, GTLV_SELECTED) => s.selected,
        (LISTVIEW_KIND, GTLV_TOP) | (SCROLLER_KIND, GTSC_TOP) => s.top,
        (SCROLLER_KIND, GTSC_TOTAL) => s.total,
        (SCROLLER_KIND, GTSC_VISIBLE) => s.visible,
        (PALETTE_KIND, GTPA_COLOR) => palette_pen(s, s.selected),
        (PALETTE_KIND, GTPA_COLOR_OFFSET) => s.color_offset as u32,
        (PALETTE_KIND, GTPA_COLOR_TABLE) => s.table_ptr,
        (NUMBER_KIND, GTNM_NUMBER) => s.number as u32,
        (NUMBER_KIND, GTNM_FORMAT) => s.format_ptr,
        _ => return None,
    })
}

// ---- PALETTE_KIND (paletteclass.c) ----

/// `GetPalettePen`: the pen of colour index `idx`.
pub fn palette_pen(rs: &RowState, idx: u32) -> u32 {
    if rs.table.is_empty() { idx.wrapping_add(rs.color_offset as u32) & 0xFFFF } else { rs.table.get(idx as usize).copied().unwrap_or(0) as u32 }
}

/// `GTPA_Color` is a pen: palette_set converts it to an index (the table position, or pen - offset as a UBYTE). A pen the table does not hold is a
/// named refusal (AROS would keep the raw number as an index, which can point past the colours).
fn palette_index_of_pen(rs: &RowState, pen: u32) -> Result<u32, Refusal> {
    if rs.table_ptr != 0 && !rs.table.is_empty() {
        return rs.table.iter().position(|p| *p as u32 == pen as u8 as u32).map(|i| i as u32)
            .ok_or_else(|| Refusal(format!("PALETTE_KIND: GTPA_Color pen {pen} is not in the gadget's GTPA_ColorTable")));
    }
    Ok((pen as u8).wrapping_sub(rs.color_offset) as u32)
}

/// GTPA_Depth is a UBYTE shift: 1 << depth colours; a palette of more than 65536 colours is not a palette (GTPA_NumColors is a UWORD).
fn palette_depth(depth: u32) -> Result<u32, Refusal> {
    let d = depth as u8;
    if d > 16 { return Err(Refusal(format!("PALETTE_KIND: GTPA_Depth {d} is beyond 16 bits (65536 colours; GTPA_NumColors is a UWORD)"))); }
    Ok(1u32 << d)
}

/// After a depth / colour count change AROS checks the selection fits: `if (pd_Color > pd_NumColors - 1) pd_Color = 0` (palette_set, relayout).
fn palette_reset(rs: &mut RowState) { if rs.selected >= rs.num_colors { rs.selected = 0; } }

/// Re-read the GTPA_ColorTable (if any, `num_colors` pens) and the swatch colours from the screen palette. A pen outside the screen's 256
/// entries, an empty palette and a table the memory cannot hold are named refusals.
fn palette_refresh<U: GuestUi>(a: &mut Amiga<U>, rs: &mut RowState) -> Result<(), Refusal> {
    if rs.num_colors == 0 { return Err(Refusal("PALETTE_KIND: a palette of 0 colours".into())); }
    if rs.table_ptr != 0 {
        rs.table = (0..rs.num_colors).map(|i| a.mem.read_u8(rs.table_ptr + i).map_err(Refusal::mem("GTPA_ColorTable"))).collect::<Result<_, _>>()?;
    }
    let mut colors = Vec::with_capacity(rs.num_colors as usize);
    for i in 0..rs.num_colors {
        let pen = palette_pen(rs, i);
        let rgb = a.gfx.palette().rgb.get(pen as usize).copied().ok_or_else(|| Refusal(format!("PALETTE_KIND: colour {i} is pen {pen}, outside the screen's 256-entry palette")))?;
        colors.push(rgb & 0x00FF_FFFF);
    }
    rs.colors = colors;
    Ok(())
}

fn palette_from_tags<U: GuestUi>(a: &mut Amiga<U>, t: &[(u32, u32)], rs: &mut RowState) -> Result<(), Refusal> {
    let get = |tag| tags::get(t, tag);
    // palette_set: GTPA_NumColors overrides GTPA_Depth; the default is 2 colours, pen = index + offset.
    rs.color_offset = get(GTPA_COLOR_OFFSET).unwrap_or(0) as u8;
    rs.table_ptr = get(GTPA_COLOR_TABLE).unwrap_or(0);
    let sized = get(GTPA_DEPTH).is_some() || get(GTPA_NUM_COLORS).is_some();
    if let Some(d) = get(GTPA_DEPTH) { rs.num_colors = palette_depth(d)?; }
    if let Some(n) = get(GTPA_NUM_COLORS) { rs.num_colors = n as u16 as u32; }
    palette_refresh_table_only(a, rs)?;
    if let Some(pen) = get(GTPA_COLOR) { rs.selected = palette_index_of_pen(rs, pen)?; }
    if sized { palette_reset(rs); }
    palette_refresh(a, rs)
}

/// The table must be known before GTPA_Color (a pen) can be turned into an index.
fn palette_refresh_table_only<U: GuestUi>(a: &Amiga<U>, rs: &mut RowState) -> Result<(), Refusal> {
    if rs.table_ptr != 0 {
        rs.table = (0..rs.num_colors).map(|i| a.mem.read_u8(rs.table_ptr + i).map_err(Refusal::mem("GTPA_ColorTable"))).collect::<Result<_, _>>()?;
    }
    Ok(())
}
