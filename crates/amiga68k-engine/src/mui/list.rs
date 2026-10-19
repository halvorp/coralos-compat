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
use super::hooks::{After, HookCall, Msg};
use crate::boopsi::{Ext, Objects};
use crate::{machine::Amiga, Refusal};
use guestui_abi::{ControlKind, GuestUi};

/// One row: a stable id, the program's entry pointer (what the construct hook returned, when there is one), the text of its
/// cells, and the guest copy the engine made for a `MUIV_List_ConstructHook_String` list.
#[derive(Debug, Clone, Default)]
pub struct Entry { pub id: u32, pub ptr: u32, pub cells: Vec<String>, pub owned: Option<(u32, u32)> }

#[derive(Debug, Clone)]
pub struct ListData { pub entries: Vec<Entry>, pub active: i32, pub first: u32, pub next_id: u32, pub titles: Vec<String>, pub titles_asked: bool, /** The pool the List made for itself when none was given (0 = not yet). */ pub own_pool: u32 }
impl Default for ListData { fn default() -> Self { Self { entries: Vec::new(), active: -1, first: 0, next_id: 1, titles: Vec::new(), titles_asked: false, own_pool: 0 } } }

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
    let from_hook = o.num(obj, MUIA_List_Title) == Some(1); // MUIA_List_Title TRUE: the display hook, called with a NULL entry, gives the headings
    let heading = |i: usize| -> String { if from_hook { d.titles.get(i).cloned().unwrap_or_default() } else { title.split('\t').nth(i).unwrap_or("").into() } };
    let columns: Vec<String> = if title.is_empty() && !from_hook && n == 1 { Vec::new() } else { (0..n).map(heading).collect() };
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

/// Release what the List holds in the guest: the string copies of a `MUIV_List_ConstructHook_String` list.
pub fn release<U: GuestUi>(a: &mut Amiga<U>, list: u32) {
    let owned: Vec<(u32, u32)> = data(&a.objects, list).map(|d| d.entries.iter().filter_map(|e| e.owned).collect()).unwrap_or_default();
    for (addr, size) in owned { let _ = a.heap.free(addr, size); }
    let own = data(&a.objects, list).map_or(0, |d| d.own_pool);
    if own != 0 { crate::memory::drop_pool(a, own); }
}

/// The pool the construct and destruct hooks get in A2: `MUIA_List_Pool`, else the List's own (list.c OM_SET: "No memory pool given, so we create our own").
pub(super) fn pool_of<U: GuestUi>(a: &mut Amiga<U>, list: u32) -> u32 {
    let given = a.objects.num_or(list, MUIA_List_Pool, 0);
    if given != 0 { return given; }
    let own = data(&a.objects, list).map_or(0, |d| d.own_pool);
    if own != 0 { return own; }
    let p = crate::memory::new_pool(a, 0);
    if let Ok(d) = data_mut(a, list) { d.own_pool = p; }
    p
}

fn text_at<U: GuestUi>(a: &Amiga<U>, ptr: u32) -> Result<String, Refusal> {
    if ptr == 0 { return Ok(String::new()); }
    a.mem.read_cstr(ptr).map(|t| super::roles::strip_format(&t, false)).map_err(Refusal::mem("MUI List entry string"))
}

/// The list's hook `which` as a guest `struct Hook *` (0 = none, `MUIV_List_*Hook_String` = -1 is not a hook).
fn hook_of(o: &Objects, list: u32, which: u32) -> u32 { match o.num_or(list, which, 0) { u32::MAX => 0, h => h } }
fn string_mode(o: &Objects, list: u32, which: u32) -> bool { o.num(list, which) == Some(u32::MAX) }

/// Queue the display hook for row `id` (`entry` is what `A1` carries) so its cell pointers fill the row's cells.
fn queue_display<U: GuestUi>(a: &mut Amiga<U>, list: u32, id: Option<u32>, entry: u32, position: i32) -> Result<(), Refusal> {
    let hook = hook_of(&a.objects, list, MUIA_List_DisplayHook);
    if hook == 0 { return Ok(()); }
    let cols = columns(&a.objects, list).max(1);
    let size = 4 * (cols as u32 + 2);
    let block = a.heap.alloc(size).ok_or_else(|| Refusal("MUI List: no memory for the display hook's array (NOMEM)".into()))?;
    a.mem.zero(block, size).map_err(Refusal::mem("List display array"))?;
    a.mem.write_u32(block, position as u32).map_err(Refusal::mem("List display array"))?; // array[-1] is the row's position
    a.objects.hooks.push_back(HookCall { hook, obj: block + 4, msg: Msg::Word(entry), after: After::Displayed { list, id, block, size, cols } });
    Ok(())
}

/// The column headings from the display hook (a NULL entry), asked once.
pub(crate) fn ask_titles<U: GuestUi>(a: &mut Amiga<U>, list: u32) -> Result<(), Refusal> {
    let asked = data(&a.objects, list).is_some_and(|d| d.titles_asked);
    if asked || a.objects.num(list, MUIA_List_Title) != Some(1) || hook_of(&a.objects, list, MUIA_List_DisplayHook) == 0 { return Ok(()); }
    data_mut(a, list)?.titles_asked = true;
    queue_display(a, list, None, 0, -1)
}

/// A hook the List queued has returned `d0`: store what it produced.
pub fn hook_returned<U: GuestUi>(a: &mut Amiga<U>, after: &After, d0: u32) -> Result<(), Refusal> {
    match *after {
        After::Nothing | After::ReplyCx(_) => Ok(()),
        After::Constructed { list, id } => {
            let Some(d) = a.objects.map.get_mut(&list).and_then(|o| if let Ext::List(d) = &mut o.ext { Some(d) } else { None }) else { return Ok(()) };
            let Some(i) = d.entries.iter().position(|e| e.id == id) else { return Ok(()) };
            if d0 == 0 { d.entries.remove(i); return super::win::refresh(a, list); } // a construct hook that returns NULL: the entry is not added
            d.entries[i].ptr = d0;
            queue_display_front(a, list, id, d0, i as i32)
        }
        After::Displayed { list, id, block, size, cols } => {
            let mut cells = Vec::new();
            for c in 0..cols { let p = a.mem.read_u32(block + 4 + 4 * c as u32).map_err(Refusal::mem("List display array"))?; cells.push(text_at(a, p)?); }
            let _ = a.heap.free(block, size);
            if let Some(Ext::List(d)) = a.objects.map.get_mut(&list).map(|o| &mut o.ext) {
                match id { Some(id) => if let Some(e) = d.entries.iter_mut().find(|e| e.id == id) { e.cells = cells; }, None => d.titles = cells }
            }
            super::win::refresh(a, list)
        }
    }
}

/// The display call for a freshly constructed row runs before whatever else is queued.
fn queue_display_front<U: GuestUi>(a: &mut Amiga<U>, list: u32, id: u32, entry: u32, position: i32) -> Result<(), Refusal> {
    let before = a.objects.hooks.len();
    queue_display(a, list, Some(id), entry, position)?;
    if a.objects.hooks.len() > before { if let Some(h) = a.objects.hooks.pop_back() { a.objects.hooks.push_front(h); } }
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
    ask_titles(a, list)?;
    let (construct, display) = (hook_of(&a.objects, list, MUIA_List_ConstructHook), hook_of(&a.objects, list, MUIA_List_DisplayHook));
    let strings = string_mode(&a.objects, list, MUIA_List_ConstructHook);
    let mut first = None;
    let mut queued = Vec::new();
    for (i, &p) in ptrs.iter().enumerate() {
        let id = { let d = data_mut(a, list)?; d.next_id += 1; d.next_id - 1 };
        let mut e = Entry { id, ptr: p, cells: Vec::new(), owned: None };
        if display == 0 { e.cells = alloc::vec![text_at(a, p)?]; }
        if strings {
            // MUIV_List_ConstructHook_String: the list keeps its own copy of the string.
            let s = text_at(a, p)?;
            let size = s.len() as u32 + 1;
            let copy = a.heap.alloc(size).ok_or_else(|| Refusal("MUI List: no memory for an entry string (NOMEM)".into()))?;
            a.mem.write_bytes(copy, s.as_bytes()).map_err(Refusal::mem("List entry copy"))?;
            a.mem.write_u8(copy + s.len() as u32, 0).map_err(Refusal::mem("List entry copy"))?;
            (e.ptr, e.owned) = (copy, Some((copy, size)));
        }
        let d = data_mut(a, list)?;
        let at = if pos == MUIV_List_Insert_Sorted || i == 0 { position(d, &e, pos) } else { first.unwrap_or(0) + i };
        first.get_or_insert(at);
        d.entries.insert(at, e);
        if d.active >= at as i32 && d.active >= 0 { d.active += 1; }
        queued.push((id, p, at));
    }
    for (id, p, at) in queued {
        if construct != 0 {
            let pool = pool_of(a, list);
            a.objects.hooks.push_back(HookCall { hook: construct, obj: pool, msg: Msg::Word(p), after: After::Constructed { list, id } });
        } else if display != 0 {
            let entry = data(&a.objects, list).and_then(|d| d.entries.iter().find(|e| e.id == id)).map_or(p, |e| e.ptr);
            queue_display(a, list, Some(id), entry, at as i32)?;
        }
    }
    Ok(first.unwrap_or(0) as u32)
}

/// Queue the destruct hook (or free the engine's copy) for a removed entry.
fn destroy<U: GuestUi>(a: &mut Amiga<U>, list: u32, e: Entry) {
    if let Some((addr, size)) = e.owned { let _ = a.heap.free(addr, size); }
    let hook = hook_of(&a.objects, list, MUIA_List_DestructHook);
    if hook != 0 { let pool = pool_of(a, list); a.objects.hooks.push_back(HookCall { hook, obj: pool, msg: Msg::Word(e.ptr), after: After::Nothing }); }
}

/// MUIM_List_Remove(pos).
pub fn remove<U: GuestUi>(a: &mut Amiga<U>, list: u32, pos: u32) -> Result<(), Refusal> {
    let d = data_mut(a, list)?;
    let at = match pos {
        MUIV_List_Remove_First => 0,
        MUIV_List_Remove_Active => if d.active < 0 { return Ok(()) } else { d.active as usize },
        MUIV_List_Remove_Last => d.entries.len().saturating_sub(1),
        MUIV_List_Remove_Selected => return Err(Refusal("MUIM_List_Remove: MUIV_List_Remove_Selected (multi-selection) is not implemented".into())),
        p => p as usize,
    };
    if at < d.entries.len() {
        let e = d.entries.remove(at);
        if d.active == at as i32 { d.active = -1; } else if d.active > at as i32 { d.active -= 1; }
        destroy(a, list, e);
    }
    Ok(())
}

/// MUIM_List_Clear.
pub fn clear<U: GuestUi>(a: &mut Amiga<U>, list: u32) -> Result<(), Refusal> {
    let d = data_mut(a, list)?;
    let all = core::mem::take(&mut d.entries);
    (d.active, d.first) = (-1, 0);
    for e in all { destroy(a, list, e); }
    Ok(())
}

/// MUIM_List_Redraw(pos): the program changed an entry; the display hook is asked again for it (`MUIV_List_Redraw_All` = every row).
pub fn redraw<U: GuestUi>(a: &mut Amiga<U>, list: u32, pos: u32) -> Result<(), Refusal> {
    let d = data(&a.objects, list).ok_or_else(|| Refusal("MUI List: no list state".into()))?;
    let rows: Vec<(u32, u32, usize)> = match pos {
        MUIV_List_Redraw_All => d.entries.iter().enumerate().map(|(i, e)| (e.id, e.ptr, i)).collect(),
        MUIV_List_Redraw_Active => (d.active >= 0).then(|| d.entries.get(d.active as usize).map(|e| (e.id, e.ptr, d.active as usize))).flatten().into_iter().collect(),
        p => d.entries.get(p as usize).map(|e| (e.id, e.ptr, p as usize)).into_iter().collect(),
    };
    for (id, ptr, at) in rows {
        if hook_of(&a.objects, list, MUIA_List_DisplayHook) == 0 { if let Some(Ext::List(d)) = a.objects.map.get_mut(&list).map(|o| &mut o.ext) { let text = a.mem.read_cstr(ptr).unwrap_or_default(); if let Some(e) = d.entries.iter_mut().find(|e| e.id == id) { e.cells = alloc::vec![text]; } } }
        else { queue_display(a, list, Some(id), ptr, at as i32)?; }
    }
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
