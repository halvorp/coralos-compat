//! The List class: the entries a program inserts, the active row, and the rows a native list control shows.
//! Facts: Zune's list.c (MUIM_List_* ids and argument orders, the MUIV_List_Insert_* / _Active_* positions, entries are C
//! strings when no hooks are set, the column count comes from `MUIA_List_Format`'s commas) and mui.h.
//!
//! Entries are the guest's own pointers (kept as given, handed back by `MUIM_List_GetEntry`); the text of each row is read
//! from them, or produced by the program's display hook ([`super::hooks`]).
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use super::classes::Kind;
use super::ids_gen::*;
use crate::boopsi::{Ext, Objects};
use crate::{machine::Amiga, Refusal};
use guestui_abi::{ControlKind, GuestUi};

/// One row: the program's entry pointer and the text of its cells.
#[derive(Debug, Clone, Default)]
pub struct Entry { pub ptr: u32, pub cells: Vec<String> }

#[derive(Debug, Clone)]
pub struct ListData { pub entries: Vec<Entry>, pub active: i32, pub first: u32 }
impl Default for ListData { fn default() -> Self { Self { entries: Vec::new(), active: -1, first: 0 } } }

fn data<'a>(o: &'a Objects, list: u32) -> Option<&'a ListData> {
    match o.map.get(&list).map(|x| &x.ext) { Some(Ext::List(d)) => Some(d), _ => None }
}
fn data_mut<'a, U: GuestUi>(a: &'a mut Amiga<U>, list: u32) -> Result<&'a mut ListData, Refusal> {
    match a.objects.map.get_mut(&list).map(|x| &mut x.ext) { Some(Ext::List(d)) => Ok(d), _ => Err(Refusal(format!("{list:#x} is not a MUI List"))) }
}

/// Number of columns: one more than the commas of `MUIA_List_Format` (a list without a format has one).
pub fn columns(o: &Objects, list: u32) -> usize { o.text(list, MUIA_List_Format).matches(',').count() + 1 }

/// The List a Listview shows, or the object itself when it is a List.
pub fn list_of(o: &Objects, obj: u32) -> Option<u32> {
    match o.kind(obj)? { Kind::List => Some(obj), Kind::Listview => o.children(obj).first().copied(), _ => None }
}

/// The list control for List `obj`.
pub fn control(o: &Objects, obj: u32) -> Option<ControlKind> {
    let d = data(o, obj)?;
    let n = columns(o, obj);
    let title = o.text(obj, MUIA_List_Title);
    let columns: Vec<String> = if title.is_empty() && n == 1 { Vec::new() } else { (0..n).map(|i| title.split('\t').nth(i).unwrap_or("").into()).collect() };
    let width = columns.len().max(1);
    let rows: Vec<Vec<String>> = d.entries.iter().map(|e| (0..width).map(|i| e.cells.get(i).cloned().unwrap_or_default()).collect()).collect();
    let selected = (d.active >= 0 && (d.active as usize) < rows.len()).then_some(d.active as u32);
    Some(ControlKind::List { columns, rows, selected, top: d.first.min(d.entries.len().saturating_sub(1) as u32) })
}

/// The attributes of a List the engine computes (GetAttr).
pub fn get<U: GuestUi>(a: &mut Amiga<U>, obj: u32, attr: u32) -> Result<u32, Refusal> {
    let Some(list) = list_of(&a.objects, obj) else {
        // MUIA_Listview_* forwarding is the Listview's own stored attributes; only a Listview asking for List_Active lands here.
        return Ok(a.objects.num(obj, attr).unwrap_or(0));
    };
    let d = data(&a.objects, list).ok_or_else(|| Refusal("MUI List: no list state".into()))?;
    Ok(match attr {
        MUIA_List_Entries => d.entries.len() as u32,
        MUIA_List_First => d.first,
        MUIA_List_Visible => d.entries.len() as u32,
        _ => d.active as u32, // MUIA_List_Active: -1 (MUIV_List_Active_Off) when no row is chosen
    })
}

/// Release what the List holds in the guest (nothing: entry pointers are the program's own).
pub fn release<U: GuestUi>(_a: &mut Amiga<U>, _list: u32) {}

fn text_at<U: GuestUi>(a: &Amiga<U>, ptr: u32) -> Result<String, Refusal> {
    if ptr == 0 { return Ok(String::new()); }
    a.mem.read_cstr(ptr).map_err(Refusal::mem("MUI List entry string"))
}

fn hook_refusal(list: u32, attr: u32) -> Refusal {
    Refusal(format!("MUI List {list:#x}: {} is set, and building or reading rows through a guest hook is not bridged yet", super::attrs::name(attr)))
}

fn no_hooks(o: &Objects, list: u32) -> Result<(), Refusal> {
    for h in [MUIA_List_ConstructHook, MUIA_List_DisplayHook, MUIA_List_DestructHook] { if o.num(list, h).is_some_and(|v| v != 0) { return Err(hook_refusal(list, h)); } }
    Ok(())
}

/// Resolve a MUIV_List_Insert_* position against the list; `Sorted` goes after the last row that is not greater (the default
/// compare is a plain string comparison, Zune list.c's default `MUIM_List_Compare`).
fn position(d: &ListData, new: &Entry, pos: u32) -> usize {
    match pos {
        MUIV_List_Insert_Top => 0,
        MUIV_List_Insert_Active => if d.active < 0 { 0 } else { d.active as usize },
        MUIV_List_Insert_Bottom => d.entries.len(),
        MUIV_List_Insert_Sorted => d.entries.iter().rposition(|e| e.cells <= new.cells).map_or(0, |i| i + 1),
        p => (p as usize).min(d.entries.len()),
    }
}

/// MUIM_List_InsertSingle(entry, pos) -> the position it went to.
pub fn insert_single<U: GuestUi>(a: &mut Amiga<U>, list: u32, entry: u32, pos: u32) -> Result<u32, Refusal> { insert_many(a, list, &[entry], pos) }

/// MUIM_List_Insert(array, count, pos): the entries are the NULL-or-`count` pointers at `array`.
pub fn insert_array<U: GuestUi>(a: &mut Amiga<U>, list: u32, array: u32, count: u32, pos: u32) -> Result<u32, Refusal> {
    let mut ptrs = Vec::new();
    let mut at = array;
    while (count == u32::MAX || (ptrs.len() as u32) < count) && ptrs.len() < 1 << 20 {
        let p = a.mem.read_u32(at).map_err(Refusal::mem("MUIM_List_Insert array"))?;
        if p == 0 && count == u32::MAX { break; }
        ptrs.push(p);
        at += 4;
    }
    insert_many(a, list, &ptrs, pos)
}

fn insert_many<U: GuestUi>(a: &mut Amiga<U>, list: u32, ptrs: &[u32], pos: u32) -> Result<u32, Refusal> {
    no_hooks(&a.objects, list)?;
    let mut entries = Vec::new();
    for &p in ptrs { entries.push(Entry { ptr: p, cells: alloc::vec![text_at(a, p)?] }); }
    let d = data_mut(a, list)?;
    let mut first = None;
    for (i, e) in entries.into_iter().enumerate() {
        let at = if pos == MUIV_List_Insert_Sorted || i == 0 { position(d, &e, pos) } else { first.unwrap_or(0) + i };
        first.get_or_insert(at);
        d.entries.insert(at, e);
        if d.active >= at as i32 && d.active >= 0 { d.active += 1; }
    }
    Ok(first.unwrap_or(0) as u32)
}

/// MUIM_List_Remove(pos).
pub fn remove<U: GuestUi>(a: &mut Amiga<U>, list: u32, pos: u32) -> Result<(), Refusal> {
    no_hooks(&a.objects, list)?;
    let d = data_mut(a, list)?;
    let at = match pos {
        MUIV_List_Remove_First => 0,
        MUIV_List_Remove_Active => if d.active < 0 { return Ok(()) } else { d.active as usize },
        MUIV_List_Remove_Last => d.entries.len().saturating_sub(1),
        MUIV_List_Remove_Selected => return Err(Refusal("MUIM_List_Remove: MUIV_List_Remove_Selected (multi-selection) is not implemented".into())),
        p => p as usize,
    };
    if at < d.entries.len() {
        d.entries.remove(at);
        if d.active == at as i32 { d.active = -1; } else if d.active > at as i32 { d.active -= 1; }
    }
    Ok(())
}

/// MUIM_List_Clear.
pub fn clear<U: GuestUi>(a: &mut Amiga<U>, list: u32) -> Result<(), Refusal> {
    no_hooks(&a.objects, list)?;
    let d = data_mut(a, list)?;
    (d.entries.clear(), d.active = -1, d.first = 0);
    Ok(())
}

/// MUIM_List_GetEntry(pos, &entry).
pub fn get_entry<U: GuestUi>(a: &mut Amiga<U>, list: u32, pos: u32, out: u32) -> Result<(), Refusal> {
    let d = data(&a.objects, list).ok_or_else(|| Refusal("MUI List: no list state".into()))?;
    let at = match pos { MUIV_List_GetEntry_Active => d.active, p => p as i32 };
    let ptr = if at >= 0 { d.entries.get(at as usize).map_or(0, |e| e.ptr) } else { 0 };
    a.mem.write_u32(out, ptr).map_err(Refusal::mem("MUIM_List_GetEntry storage"))
}

/// A set of `MUIA_List_Active`: the row chosen (symbolic positions resolved), kept within the list.
pub fn set_active<U: GuestUi>(a: &mut Amiga<U>, list: u32, val: u32) -> Result<u32, Refusal> {
    let d = data_mut(a, list)?;
    let n = d.entries.len() as i32;
    let cur = d.active;
    let v = match val {
        MUIV_List_Active_Off => -1,
        MUIV_List_Active_Top => 0,
        MUIV_List_Active_Bottom => n - 1,
        MUIV_List_Active_Up => (cur - 1).max(0),
        MUIV_List_Active_Down => (cur + 1).min(n - 1),
        MUIV_List_Active_PageUp => (cur - 10).max(0),
        MUIV_List_Active_PageDown => (cur + 10).min(n - 1),
        p => p as i32,
    };
    d.active = if v < 0 || v >= n { -1 } else { v };
    Ok(d.active as u32)
}
