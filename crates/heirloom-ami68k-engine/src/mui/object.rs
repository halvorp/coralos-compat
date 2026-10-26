//! MUI object lifecycle and attributes: creation from a tag list (children adopted), SetAttrs / GetAttr, disposal.
//! The state is host-side ([`crate::boopsi::Obj`]); what a change does to the screen is [`super::win::refresh`], what it
//! triggers in the program is [`super::notify`]. An attribute outside the class tree is a NAMED refusal ([`super::attrs`]).
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use super::attrs::{self, Ty};
use super::classes::Kind;
use super::ids_gen::*;
use super::{notify, win};
use crate::boopsi::{self, Ext, Objects};
use crate::{machine::Amiga, ports, Refusal};
use heirloom_ui_abi::GuestUi;

impl Objects {
    pub fn kind(&self, o: u32) -> Option<Kind> { self.map.get(&o).map(|x| x.kind) }
    /// The stored longword, if the attribute was ever given.
    pub fn num(&self, o: u32, attr: u32) -> Option<u32> { self.map.get(&o).and_then(|x| x.attrs.get(&attr).copied()) }
    pub fn num_or(&self, o: u32, attr: u32, default: u32) -> u32 { self.num(o, attr).unwrap_or(default) }
    pub fn flag(&self, o: u32, attr: u32) -> bool { self.num(o, attr).unwrap_or(0) != 0 }
    pub fn text(&self, o: u32, attr: u32) -> &str { self.map.get(&o).and_then(|x| x.strs.get(&attr)).map_or("", |s| s.as_str()) }
    pub fn entries(&self, o: u32, attr: u32) -> &[String] { self.map.get(&o).and_then(|x| x.arrs.get(&attr)).map_or(&[], |v| v.as_slice()) }
    /// The window's focus chain as the program set it, the objects that still exist (MUIM_Window_SetCycleChain).
    pub fn cycle_chain(&self, win: u32) -> Vec<u32> { self.map.get(&win).map_or_else(Vec::new, |x| x.cycle_chain.iter().copied().filter(|c| self.map.contains_key(c)).collect()) }
    pub fn children(&self, o: u32) -> &[u32] { self.map.get(&o).map_or(&[], |x| x.children.as_slice()) }
}

fn kind_of<U: GuestUi>(a: &Amiga<U>, obj: u32) -> Result<Kind, Refusal> {
    a.objects.kind(obj).ok_or_else(|| Refusal(format!("{obj:#x} is not a MUI object")))
}

fn unknown(kind: Kind, attr: u32) -> Refusal { Refusal(format!("MUI {kind:?} class: attribute {} is not implemented", attrs::name(attr))) }

/// A new object of `kind` with the defaults its class has before any tag is looked at (facts: Zune's classes' OM_NEW:
/// Numeric 0..100, Gauge max 100, String.mui MaxLen 80).
fn defaults(kind: Kind) -> &'static [(u32, u32)] {
    match kind {
        k if k.is_a(Kind::Numeric) => &[(MUIA_Numeric_Min, 0), (MUIA_Numeric_Max, 100), (MUIA_Numeric_Value, 0)],
        Kind::Gauge => &[(MUIA_Gauge_Max, 100)],
        Kind::String => &[(MUIA_String_MaxLen, 80)],
        Kind::Application => &[(MUIA_Application_Active, 1)], // application.c OM_NEW: app_Active = 1
        _ => &[],
    }
}

/// Build an object of `kind` from `tags`. On any failure the half-built tree AND the not-yet-adopted children named
/// in the tags are disposed (MUI semantics: a NULL child makes the parent NULL, nothing leaks).
pub fn create<U: GuestUi>(a: &mut Amiga<U>, kind: Kind, tags: &[(u32, u32)]) -> Result<u32, Refusal> {
    let obj = boopsi::alloc_object(a, kind)?;
    match init(a, obj, kind, tags) {
        Ok(()) => Ok(obj),
        Err(e) => {
            let _ = dispose_tree(a, obj);
            for &(tag, data) in tags {
                if attrs::adopts(kind, tag) && a.objects.map.get(&data).is_some_and(|c| c.parent.is_none()) { let _ = dispose_tree(a, data); }
            }
            Err(e)
        }
    }
}

fn init<U: GuestUi>(a: &mut Amiga<U>, obj: u32, kind: Kind, tags: &[(u32, u32)]) -> Result<(), Refusal> {
    if kind == Kind::Application {
        let bit = ports::alloc_signal_bit(a, -1)?.ok_or_else(|| Refusal("MUI Application: no free signal bit".into()))?;
        if let Some(o) = a.objects.map.get_mut(&obj) { o.ext = Ext::App { bit, broker: None, pending: Default::default(), inbox: Default::default() }; }
        a.sched.app_task.insert(obj, a.sched.cur); // the task a ReturnID signals: the one that created the Application
    }
    for &(tag, v) in defaults(kind) { store_num(a, obj, tag, v); }
    for &(tag, data) in tags {
        match attrs::lookup(kind, tag).ok_or_else(|| unknown(kind, tag))? {
            Ty::Num => store_num(a, obj, tag, data),
            Ty::Str => store_str(a, obj, tag, data)?,
            Ty::StrOrFlag => if data < 0x100 { store_num(a, obj, tag, data) } else { store_str(a, obj, tag, data)? },
            Ty::StrArr => store_arr(a, obj, tag, data)?,
            Ty::Adopt => adopt(a, obj, tag, data)?,
        }
    }
    clamp_numeric(a, obj);
    if kind == Kind::Application { super::broker::create(a, obj, tags)?; }
    if kind == Kind::List { super::list::ask_titles(a, obj)?; }
    if kind == Kind::Listview && a.objects.children(obj).is_empty() {
        // Listview.mui builds its own List when none is given (Zune listview.c OM_NEW).
        let list = create(a, Kind::List, &[])?;
        adopt(a, obj, MUIA_Listview_List, list)?;
    }
    Ok(())
}

pub(crate) fn store_num<U: GuestUi>(a: &mut Amiga<U>, obj: u32, attr: u32, v: u32) {
    if let Some(o) = a.objects.map.get_mut(&obj) { o.attrs.insert(attr, v); }
    if attr == MUIA_UserData { super::mirror::user_data(a, obj, v); }
}

fn store_str<U: GuestUi>(a: &mut Amiga<U>, obj: u32, attr: u32, ptr: u32) -> Result<(), Refusal> {
    // NM_BARLABEL (-1) as a menu item's title is the separator bar (the same "-" MUI_MakeObjectA's NewMenu path stores).
    let bar = ptr == u32::MAX && matches!(attr, MUIA_Menuitem_Title | MUIA_Menu_Title);
    let s = if bar { "-".into() } else if ptr == 0 { String::new() } else { a.mem.read_cstr_long(ptr).map_err(Refusal::mem("MUI string attribute"))? };
    set_text(a, obj, attr, s);
    Ok(())
}

/// Hold `s` as the string attribute `attr` (the guest copy of an older value is released).
pub(crate) fn set_text<U: GuestUi>(a: &mut Amiga<U>, obj: u32, attr: u32, s: String) {
    let old = a.objects.map.get_mut(&obj).and_then(|o| { o.strs.insert(attr, s); o.bufs.remove(&attr) });
    if let Some((addr, size)) = old { let _ = a.heap.free(addr, size); }
}

fn store_arr<U: GuestUi>(a: &mut Amiga<U>, obj: u32, attr: u32, ptr: u32) -> Result<(), Refusal> {
    let mut v = Vec::new();
    let mut at = ptr;
    while at != 0 && v.len() < 4096 {
        let p = a.mem.read_u32(at).map_err(Refusal::mem("MUI string array"))?;
        if p == 0 { break; }
        v.push(a.mem.read_cstr(p).map_err(Refusal::mem("MUI string array entry"))?);
        at += 4;
    }
    if let Some(o) = a.objects.map.get_mut(&obj) { o.arrs.insert(attr, v); o.attrs.insert(attr, ptr); }
    Ok(())
}

/// The parent takes ownership of `child` (tag `tag`), if a child of that kind belongs there.
pub(crate) fn adopt<U: GuestUi>(a: &mut Amiga<U>, parent: u32, tag: u32, child: u32) -> Result<(), Refusal> {
    let pk = kind_of(a, parent)?;
    let name = attrs::name(tag);
    if child == 0 { return Err(Refusal(format!("{name}: the child object is NULL (its creation failed)"))); }
    let (ck, owned) = a.objects.map.get(&child).map(|c| (c.kind, c.parent.is_some()))
        .ok_or_else(|| Refusal(format!("{name}: {child:#x} is not a MUI object")))?;
    if owned { return Err(Refusal(format!("{name}: {child:#x} already has a parent"))); }
    let ok = match tag {
        MUIA_Family_Child | MUIA_Group_Child if pk.is_a(Kind::Family) => match pk { Kind::Menustrip => ck == Kind::Menu, _ => ck == Kind::Menuitem },
        MUIA_Application_Window => ck == Kind::Window,
        MUIA_Application_Menustrip | MUIA_Window_Menustrip => ck == Kind::Menustrip,
        MUIA_Window_RootObject | MUIA_Group_Child | MUIA_Scrollgroup_Contents | MUIA_Popstring_String | MUIA_Popstring_Button | MUIA_Popobject_Object => ck.is_area(),
        MUIA_Listview_List => ck == Kind::List,
        _ => false,
    };
    if !ok { return Err(Refusal(format!("{name}: a {ck:?} object cannot be a child of a {pk:?}"))); }
    if let Some(c) = a.objects.map.get_mut(&child) { c.parent = Some(parent); }
    if let Some(p) = a.objects.map.get_mut(&parent) { p.children.push(child); if matches!(tag, MUIA_Popstring_String | MUIA_Popstring_Button | MUIA_Window_Menustrip) { p.attrs.insert(tag, child); } }
    super::mirror::connected(a, child)
}

/// Value ranges MUI keeps: a Numeric's value stays within its min..max, a Gauge's current within 0..max.
fn clamp_numeric<U: GuestUi>(a: &mut Amiga<U>, obj: u32) {
    let Some(kind) = a.objects.kind(obj) else { return };
    if kind.is_a(Kind::Numeric) {
        let (lo, hi) = (a.objects.num_or(obj, MUIA_Numeric_Min, 0) as i32, a.objects.num_or(obj, MUIA_Numeric_Max, 100) as i32);
        let v = (a.objects.num_or(obj, MUIA_Numeric_Value, 0) as i32).clamp(lo.min(hi), hi.max(lo));
        store_num(a, obj, MUIA_Numeric_Value, v as u32);
    } else if kind == Kind::Gauge {
        let v = a.objects.num_or(obj, MUIA_Gauge_Current, 0).min(a.objects.num_or(obj, MUIA_Gauge_Max, 100));
        store_num(a, obj, MUIA_Gauge_Current, v);
    }
}

/// The value an attribute really takes when `val` is set (MUI's symbolic values resolved against the object).
fn resolve_value<U: GuestUi>(a: &Amiga<U>, obj: u32, kind: Kind, attr: u32, val: u32) -> u32 {
    let n = a.objects.children(obj).len() as i32;
    match (kind, attr) {
        (k, MUIA_Group_ActivePage) if k.is_a(Kind::Group) => {
            let cur = a.objects.num_or(obj, MUIA_Group_ActivePage, 0) as i32;
            match val {
                MUIV_Group_ActivePage_First => 0,
                MUIV_Group_ActivePage_Last => (n - 1).max(0) as u32,
                MUIV_Group_ActivePage_Prev => (cur - 1).max(0) as u32,
                MUIV_Group_ActivePage_Next => (cur + 1).min((n - 1).max(0)) as u32,
                MUIV_Group_ActivePage_Advance => if n == 0 { 0 } else { ((cur + 1) % n) as u32 },
                v => v,
            }
        }
        (Kind::Cycle, MUIA_Cycle_Active) | (Kind::Radio, MUIA_Radio_Active) => {
            let (cur, count) = (a.objects.num_or(obj, attr, 0) as i32, a.objects.entries(obj, if kind == Kind::Cycle { MUIA_Cycle_Entries } else { MUIA_Radio_Entries }).len() as i32);
            match val {
                MUIV_Cycle_Active_Next => if count == 0 { 0 } else { ((cur + 1) % count) as u32 },
                MUIV_Cycle_Active_Prev => if count == 0 { 0 } else { ((cur - 1 + count) % count) as u32 },
                v => v,
            }
        }
        (k, MUIA_Numeric_Value) if k.is_a(Kind::Numeric) => {
            let (lo, hi) = (a.objects.num_or(obj, MUIA_Numeric_Min, 0) as i32, a.objects.num_or(obj, MUIA_Numeric_Max, 100) as i32);
            (val as i32).clamp(lo.min(hi), hi.max(lo)) as u32
        }
        (Kind::Gauge, MUIA_Gauge_Current) => val.min(a.objects.num_or(obj, MUIA_Gauge_Max, 100)),
        _ => val,
    }
}

/// SetAttrs on a live object: store, apply the side effect on the native UI, then fire the notifications.
pub fn set_attr<U: GuestUi>(a: &mut Amiga<U>, obj: u32, attr: u32, val: u32) -> Result<(), Refusal> { set_attr_with(a, obj, attr, val, true) }

/// The tags of ONE SetAttrs / OM_SET call applied to `obj`, in order. Notify.mui's OM_SET looks through the whole list first: a `MUIA_NoNotify, TRUE`
/// anywhere in it suppresses the notifications of every tag of that call (`nnset` is exactly that), not only of the tags after it.
pub fn set_list<U: GuestUi>(a: &mut Amiga<U>, obj: u32, tags: &[(u32, u32)]) -> Result<(), Refusal> {
    let quiet = tags.iter().any(|&(t, v)| t == MUIA_NoNotify && v == 1);
    for &(tag, v) in tags { if quiet { set_attr_quiet(a, obj, tag, v)? } else { set_attr(a, obj, tag, v)? } }
    Ok(())
}

/// One value for [`set_many`].
pub enum Setting { Num(u32), Text(String) }

/// One SetAttrs call carrying several attributes: all are stored (and the screen updated) first, then the notifications run
/// in tag order (Zune's Notify class handles the notifications after every class has stored its tags). `notify = false` is
/// `nnset`. A text is stored directly; its notification carries the pointer to the object's copy, as MUI's does.
pub fn set_many<U: GuestUi>(a: &mut Amiga<U>, obj: u32, sets: &[(u32, Setting)], notify_too: bool) -> Result<(), Refusal> {
    let mut stored: Vec<(u32, bool)> = Vec::new();
    for (attr, s) in sets {
        match s {
            Setting::Num(v) => set_attr_with(a, obj, *attr, *v, false)?,
            Setting::Text(t) => { set_text(a, obj, *attr, t.clone()); win::refresh(a, obj)?; }
        }
        stored.push((*attr, matches!(s, Setting::Text(_))));
    }
    if notify_too {
        for (attr, text) in stored {
            // string.c: Acknowledge is set to the contents buffer itself, so its notification carries the same pointer.
            let v = if text { string_pointer(a, obj, if attr == MUIA_String_Acknowledge { MUIA_String_Contents } else { attr })? } else { a.objects.num_or(obj, attr, 0) };
            notify::trigger(a, obj, attr, v)?;
        }
    }
    Ok(())
}

/// MUIM_NoNotifySet: the same store and screen update, no notifications.
pub fn set_attr_quiet<U: GuestUi>(a: &mut Amiga<U>, obj: u32, attr: u32, val: u32) -> Result<(), Refusal> { set_attr_with(a, obj, attr, val, false) }

fn set_attr_with<U: GuestUi>(a: &mut Amiga<U>, obj: u32, attr: u32, val: u32, notify_too: bool) -> Result<(), Refusal> {
    let kind = kind_of(a, obj)?;
    // Group.mui's OM_SET hands every tag to its children (group.c:567-612) and Listview.mui keeps no List attribute itself (listview.c:253):
    // a List attribute set on the Listview is set on its List.
    if kind == Kind::Listview && super::list::forwarded(attr) {
        let l = super::list::list_of(&a.objects, obj).ok_or_else(|| Refusal("MUI Listview has no List".into()))?;
        return set_attr_with(a, l, attr, val, notify_too);
    }
    let ty = attrs::lookup(kind, attr).ok_or_else(|| unknown(kind, attr))?;
    let mut shown = val;
    match ty {
        // A subclass hands its OM_NEW tag list to OM_SET again after DoSuperMethod made the object (Zune's PrefsEditor does): the child is
        // already adopted, and Zune's OM_SET has no case for it (group.c), so the repeat is a no-op, not a second adoption.
        Ty::Adopt if a.objects.children(obj).contains(&val) => return Ok(()),
        Ty::Adopt => return Err(Refusal(format!("{}: can be given only when the object is created", attrs::name(attr)))),
        Ty::Str => {
            store_str(a, obj, attr, val)?;
            if (kind, attr) == (Kind::Window, MUIA_Window_Title) { win::push_title(a, obj)?; }
        }
        Ty::StrOrFlag => if val < 0x100 { store_num(a, obj, attr, val); } else { store_str(a, obj, attr, val)?; },
        Ty::StrArr => store_arr(a, obj, attr, val)?,
        Ty::Num => {
            shown = resolve_value(a, obj, kind, attr, val);
            if (kind, attr) == (Kind::List, MUIA_List_Active) { shown = super::list::set_active(a, obj, val)?; }
            if (kind, attr) == (Kind::Application, MUIA_Application_Active) { shown = (val != 0) as u32; super::broker::sync_active(a, obj, val != 0); }
            if (kind, attr) == (Kind::Window, MUIA_Window_Open) {
                shown = (val != 0) as u32;
                win::set_open(a, obj, val != 0)?;
            }
            store_num(a, obj, attr, shown);
            if kind.is_a(Kind::Numeric) && matches!(attr, MUIA_Numeric_Min | MUIA_Numeric_Max) { clamp_numeric(a, obj); }
            if kind == Kind::Gauge && attr == MUIA_Gauge_Max { clamp_numeric(a, obj); }
        }
    }
    win::refresh(a, obj)?;
    if notify_too { notify::trigger(a, obj, attr, shown)?; }
    Ok(())
}

/// A pointer to a guest copy of the string attribute (made on first read, released when the string changes or the object goes).
fn string_pointer<U: GuestUi>(a: &mut Amiga<U>, obj: u32, attr: u32) -> Result<u32, Refusal> {
    let s = String::from(a.objects.text(obj, attr));
    let size = (s.len() as u32 + 1).max(16);
    if let Some(&(addr, have)) = a.objects.map.get(&obj).and_then(|o| o.bufs.get(&attr)) {
        if have >= s.len() as u32 + 1 { a.mem.write_bytes(addr, s.as_bytes()).map_err(Refusal::mem("string attribute copy"))?; a.mem.write_u8(addr + s.len() as u32, 0).map_err(Refusal::mem("string attribute copy"))?; return Ok(addr); }
    }
    let addr = a.heap.alloc(size).ok_or_else(|| Refusal("MUI: no memory for a string attribute (NOMEM)".into()))?;
    a.mem.write_bytes(addr, s.as_bytes()).map_err(Refusal::mem("string attribute copy"))?;
    a.mem.write_u8(addr + s.len() as u32, 0).map_err(Refusal::mem("string attribute copy"))?;
    let old = a.objects.map.get_mut(&obj).and_then(|o| o.bufs.insert(attr, (addr, size)));
    if let Some((x, sz)) = old { let _ = a.heap.free(x, sz); }
    Ok(addr)
}

/// The nearest ancestor of `obj` that is (a descendant of) `want`.
pub fn ancestor(objs: &Objects, mut obj: u32, want: Kind) -> Option<u32> {
    while let Some(p) = objs.map.get(&obj).and_then(|o| o.parent) { if objs.kind(p).is_some_and(|k| k.is_a(want)) { return Some(p); } obj = p; }
    None
}

/// GetAttr: the stored value (the class default, or 0, when never set); attributes the object computes are answered here.
pub fn get_attr<U: GuestUi>(a: &mut Amiga<U>, obj: u32, attr: u32) -> Result<u32, Refusal> {
    let kind = kind_of(a, obj)?;
    // Listview.mui answers the List attributes from its List (listview.c:126-157).
    if kind == Kind::Listview && super::list::forwarded(attr) {
        let l = super::list::list_of(&a.objects, obj).ok_or_else(|| Refusal("MUI Listview has no List".into()))?;
        return get_attr(a, l, attr);
    }
    let ty = attrs::lookup(kind, attr).ok_or_else(|| unknown(kind, attr))?;
    match attr {
        MUIA_ApplicationObject => return Ok(ancestor(&a.objects, obj, Kind::Application).unwrap_or(0)),
        MUIA_WindowObject => return Ok(ancestor(&a.objects, obj, Kind::Window).unwrap_or(0)),
        MUIA_Application_Broker | MUIA_Application_BrokerPort if kind == Kind::Application => return Ok(super::broker::get(a, obj, attr)),
        MUIA_Parent => return Ok(a.objects.map.get(&obj).and_then(|o| o.parent).unwrap_or(0)),
        MUIA_Window if kind.is_area() => return Ok(0), // no Intuition Window behind a native window: see docs/AMIGA-TRANSLATION.md
        MUIA_Group_ChildList => return Err(Refusal("MUI Group class: MUIA_Group_ChildList (a struct List of the children) is not implemented".into())),
        MUIA_String_Acknowledge => return string_pointer(a, obj, MUIA_String_Contents),
        MUIA_String_Integer => return Ok(a.objects.text(obj, MUIA_String_Contents).trim().parse::<i64>().unwrap_or(0) as u32),
        MUIA_List_Entries | MUIA_List_First | MUIA_List_Visible | MUIA_List_Active => return super::list::get(a, obj, attr),
        _ => {}
    }
    Ok(match ty {
        Ty::Str => string_pointer(a, obj, attr)?,
        _ => a.objects.num(obj, attr).unwrap_or(0),
    })
}

/// MUI_DisposeObject / DisposeObject: only a root; the tree goes with it.
pub fn dispose<U: GuestUi>(a: &mut Amiga<U>, obj: u32) -> Result<(), Refusal> {
    match a.objects.map.get(&obj) {
        None => Err(Refusal(format!("DisposeObject: {obj:#x} is not a MUI object (or already disposed)"))),
        Some(o) if o.parent.is_some() => Err(Refusal(format!("DisposeObject: {obj:#x} is owned by a parent; dispose the root"))),
        Some(_) => dispose_tree(a, obj),
    }
}

/// The rest of a disposal once the children's destructors have run ([`super::dispose`]): window, signal bit, list data, the record.
pub(crate) fn dispose_remaining<U: GuestUi>(a: &mut Amiga<U>, obj: u32) -> Result<(), Refusal> { dispose_tree(a, obj) }

fn dispose_tree<U: GuestUi>(a: &mut Amiga<U>, obj: u32) -> Result<(), Refusal> {
    let Some(o) = a.objects.map.get(&obj) else { return Ok(()) };
    let (kind, children): (Kind, Vec<u32>) = (o.kind, o.children.clone());
    let bit = if let Ext::App { bit, .. } = o.ext { Some(bit) } else { None };
    if kind == Kind::Application { super::broker::destroy(a, obj)?; super::mirror::forget_application(a, obj); }
    let mut first: Result<(), Refusal> = Ok(());
    if kind == Kind::Window { first = win::close_if_open(a, obj); }
    for c in children { let r = dispose_tree(a, c); if first.is_ok() { first = r; } }
    if let Some(b) = bit { let r = ports::release_signal_bit(a, b as i32); if first.is_ok() { first = r; } }
    if kind == Kind::List { super::list::release(a, obj); }
    a.objects.drawn.remove(&obj);
    let r = boopsi::free_object(a, obj);
    if first.is_ok() { r } else { first }
}

/// The MUI half of an object the BOOPSI core made for a program's own subclass: its tags are applied like `MUI_NewObjectA`'s.
pub(crate) fn init_adopted<U: GuestUi>(a: &mut Amiga<U>, obj: u32, kind: Kind, tags: &[(u32, u32)]) -> Result<(), Refusal> {
    let r = init(a, obj, kind, tags).and_then(|()| super::mirror::notify_data(a, obj));
    if r.is_err() { for &(tag, data) in tags { if attrs::adopts(kind, tag) && a.objects.map.get(&data).is_some_and(|c| c.parent.is_none()) { let _ = dispose_tree(a, data); } } }
    r
}

/// `OM_DISPOSE` reaching the native part of a subclass instance: the tree below it goes (its children's own dispatchers are not
/// sent OM_DISPOSE: only the object the program disposed runs its destructor), and it leaves its parent.
pub(crate) fn dispose_adopted<U: GuestUi>(a: &mut Amiga<U>, obj: u32) -> Result<(), Refusal> {
    if let Some(p) = a.objects.map.get(&obj).and_then(|o| o.parent) { if let Some(po) = a.objects.map.get_mut(&p) { po.children.retain(|&c| c != obj); } }
    dispose_tree(a, obj)
}
