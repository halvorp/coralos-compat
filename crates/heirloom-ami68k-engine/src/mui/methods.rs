//! The methods a program can DoMethod on a MUI object: how many longwords follow the method id in the guest's message
//! ([`arity`]) and what each does ([`method`]). A method no class of the object's tree has is a NAMED refusal.
//! Facts: the `MUIP_*` message structs of Zune's class headers (notify.h, application.h, window.h, group.h, family.h, numeric.h,
//! list.h) and mui.h; intuition/classusr.h for `OM_ADDMEMBER` / `OM_REMMEMBER`.
use alloc::format;
use super::classes::Kind;
use super::hooks::{After, HookCall, Msg};
use super::ids_gen::*;
use super::{app, attrs, handlers, list, notify, object, store};
use crate::boopsi_ids::OM_GET;
use crate::{machine::Amiga, Refusal};
use heirloom_ui_abi::GuestUi;

/// intuition/classusr.h.
pub const OM_ADDMEMBER: u32 = 0x109;
pub const OM_REMMEMBER: u32 = 0x10A;

/// How many longwords follow the method id in a message the guest built at `msg`.
pub fn arity<U: GuestUi>(a: &Amiga<U>, id: u32, msg: u32) -> Result<u32, Refusal> {
    let word = |i: u32| a.mem.read_u32(msg + 4 * i).map_err(Refusal::mem("DoMethod message"));
    let until_null = |from: u32| -> Result<u32, Refusal> { let mut n = from; while n < from + 4096 && word(n + 1)? != 0 { n += 1; } Ok(n) };
    Ok(match id {
        MUIM_Notify => 4 + word(4)?.min(64),
        MUIM_Set | MUIM_NoNotifySet | OM_GET | MUIM_GetConfigItem | MUIM_Dataspace_Find | MUIM_KillNotifyObj | MUIM_WriteLong | MUIM_WriteString | MUIM_Family_Insert
        | MUIM_List_InsertSingle | MUIM_List_GetEntry | MUIM_List_Exchange | MUIM_List_Move | MUIM_Window_SetMenuCheck | MUIM_Window_SetMenuState => 2,
        MUIM_SetUData | MUIM_SetUDataOnce | MUIM_GetUData | MUIM_List_Insert | MUIM_List_Select | MUIM_Dataspace_Add | MUIM_Dataspace_ReadIFF | MUIM_Dataspace_WriteIFF => 3,
        MUIM_MultiSet => until_null(2)?,
        MUIM_Application_PushMethod => 2 + word(2)?.min(64),
        MUIM_Application_NewInput | MUIM_Application_Input | MUIM_Application_ReturnID | MUIM_Application_OpenConfigWindow | MUIM_Application_AddInputHandler
        | MUIM_Application_RemInputHandler | MUIM_FindUData | MUIM_KillNotify | MUIM_Family_AddHead | MUIM_Family_AddTail | MUIM_Family_Remove
        | OM_ADDMEMBER | OM_REMMEMBER | MUIM_Numeric_Increase | MUIM_Numeric_Decrease | MUIM_List_Remove | MUIM_List_Redraw | MUIM_List_Jump
        | MUIM_Window_AddEventHandler | MUIM_Window_RemEventHandler | MUIM_Window_Snapshot | MUIM_Window_GetMenuCheck | MUIM_Window_GetMenuState
        | MUIM_List_NextSelected | MUIM_Popstring_Close | MUIM_Dataspace_Remove | MUIM_Dataspace_Merge | MUIM_Application_Load | MUIM_Application_Save => 1,
        MUIM_Window_SetCycleChain | MUIM_Group_Sort => until_null(0)?,
        MUIM_SetAsString => super::set_string::arity(a, msg)?,
        _ => 0,
    })
}

fn bad_kind(kind: Kind, id: u32) -> Refusal { Refusal(format!("MUI {kind:?} class: {} is not implemented", attrs::method_name(id))) }

/// A host or guest method call on `obj`: `args` are the longwords after the method id.
pub fn method<U: GuestUi>(a: &mut Amiga<U>, obj: u32, id: u32, args: &[u32]) -> Result<u32, Refusal> {
    let kind = a.objects.kind(obj).ok_or_else(|| Refusal(format!("DoMethod: {obj:#x} is not a MUI object")))?;
    let arg = |i: usize| args.get(i).copied().ok_or_else(|| Refusal(format!("{}: missing argument {i}", attrs::method_name(id))));
    match id {
        // Listview.mui (listview.c:196-232) redirects a notification on a List attribute to its List.
        MUIM_Notify | MUIM_KillNotify | MUIM_KillNotifyObj if kind == Kind::Listview && args.first().is_some_and(|&t| list::forwarded(t)) => {
            let l = list::list_of(&a.objects, obj).ok_or_else(|| Refusal("MUI Listview has no List".into()))?;
            let mut redirected = args.to_vec();
            let dest = if id == MUIM_Notify { 2 } else { 1 };
            if id != MUIM_KillNotify && redirected.get(dest) == Some(&MUIV_Notify_Self) { redirected[dest] = obj; }
            method(a, l, id, &redirected)
        }
        MUIM_Notify => notify::add(a, obj, args),
        // Notify.mui's MUIM_Set is `SetAttrs(obj, attr, value, TAG_DONE)`: for an object of the program's own class that is OM_SET through its dispatcher.
        MUIM_Set => {
            match super::custom::own_class(a, obj) {
                Some(cl) => super::custom::queue_om_set(a, cl, obj, Msg::Set(alloc::vec![(arg(0)?, arg(1)?)])),
                None => object::set_attr(a, obj, arg(0)?, arg(1)?)?,
            }
            Ok(0)
        }
        MUIM_SetAsString => super::set_string::set_as_string(a, obj, args),
        MUIM_NoNotifySet => { object::set_attr_quiet(a, obj, arg(0)?, arg(1)?)?; Ok(0) }
        OM_GET => { let v = object::get_attr(a, obj, arg(0)?)?; a.mem.write_u32(arg(1)?, v).map_err(Refusal::mem("MUIM_Get storage"))?; Ok(1) }
        MUIM_CallHook => {
            a.objects.hooks.push_back(HookCall { hook: arg(0)?, obj, msg: Msg::Params(args[1..].to_vec()), after: After::Nothing });
            Ok(0)
        }
        // Notify.mui: MUIM_Dataspace_Find on the application's configuration dataspace, which holds only what the user SAVED (the defaults are
        // applied by the class that asks, not stored: configdata.c GetConfigString/GetConfigULong). Nothing is saved here, so no item is found
        // and the caller (an MCC reading its pen specs, a window its spacing) uses its own default.
        MUIM_GetConfigItem => Ok(0),
        MUIM_KillNotify => { let t = arg(0)?; if let Some(o) = a.objects.map.get_mut(&obj) { o.notifies.retain(|n| n.attr != t); } Ok(0) }
        MUIM_KillNotifyObj => { let (t, d) = (arg(0)?, arg(1)?); if let Some(o) = a.objects.map.get_mut(&obj) { o.notifies.retain(|n| !(n.attr == t && n.dest == d)); } Ok(0) }
        MUIM_SetUData | MUIM_SetUDataOnce => { let (u, at, v) = (arg(0)?, arg(1)?, arg(2)?); set_udata(a, obj, u, at, v, id == MUIM_SetUData)?; Ok(0) }
        MUIM_GetUData => match find_udata(a, obj, arg(0)?) { Some(o) => { let v = object::get_attr(a, o, arg(1)?)?; a.mem.write_u32(arg(2)?, v).map_err(Refusal::mem("MUIM_GetUData storage"))?; Ok(1) } None => Ok(0) },
        MUIM_FindUData => Ok(find_udata(a, obj, arg(0)?).unwrap_or(0)),
        MUIM_WriteLong => { a.mem.write_u32(arg(1)?, arg(0)?).map_err(Refusal::mem("MUIM_WriteLong"))?; Ok(0) }
        MUIM_WriteString => { let s = a.mem.read_cstr(arg(0)?).map_err(Refusal::mem("MUIM_WriteString"))?; a.mem.write_bytes(arg(1)?, s.as_bytes()).map_err(Refusal::mem("MUIM_WriteString"))?; a.mem.write_u8(arg(1)? + s.len() as u32, 0).map_err(Refusal::mem("MUIM_WriteString"))?; Ok(0) }
        MUIM_MultiSet => { let (at, v) = (arg(0)?, arg(1)?); for &t in &args[2..] { object::set_attr(a, t, at, v)?; } Ok(0) }
        MUIM_Application_Load if kind == Kind::Application => { let r = store::load(a, obj, arg(0)?)?; a.log.push(format!("[muimaster] MUIM_Application_Load: {r}")); Ok(0) }
        MUIM_Application_Save if kind == Kind::Application => { let r = store::save(a, obj, arg(0)?)?; a.log.push(format!("[muimaster] MUIM_Application_Save: {r}")); Ok(0) }
        MUIM_Application_NewInput if kind == Kind::Application => app::new_input(a, obj, arg(0)?),
        MUIM_Application_Input if kind == Kind::Application => app::input(a, obj, arg(0)?),
        MUIM_Application_ReturnID if kind == Kind::Application => { app::return_id(a, obj, arg(0)?)?; Ok(0) }
        MUIM_Application_PushMethod if kind == Kind::Application => {
            let (dest, n) = (arg(0)?, arg(1)? as usize);
            let follow = args.get(2..2 + n).ok_or_else(|| Refusal("MUIM_Application_PushMethod: the message is short".into()))?;
            app::post(a, obj, app::Inbound::Push { dest, id: follow[0], args: follow[1..].to_vec() })?;
            Ok(0)
        }
        MUIM_Application_AddInputHandler if kind == Kind::Application => { handlers::add_input(a, obj, arg(0)?)?; Ok(1) }
        MUIM_Application_RemInputHandler if kind == Kind::Application => { handlers::rem_input(a, arg(0)?)?; Ok(1) }
        MUIM_Window_AddEventHandler if kind == Kind::Window => { handlers::add_event(a, obj, arg(0)?)?; Ok(1) }
        MUIM_Window_RemEventHandler if kind == Kind::Window => { handlers::rem_event(a, arg(0)?)?; Ok(1) }
        // MUI's own method (Zune's window.c has no case for it: an unknown method falls through to rootclass and answers 0, so the chain stays
        // the MUIA_CycleChain objects in setup order). The program's explicit order is recorded; heirloom-ui-abi has no focus order to hand it to.
        MUIM_Window_SetCycleChain if kind == Kind::Window => {
            if let Some(&bad) = args.iter().find(|c| !a.objects.map.contains_key(c)) { return Err(Refusal(format!("MUIM_Window_SetCycleChain: {bad:#x} is not a MUI object"))); }
            if let Some(o) = a.objects.map.get_mut(&obj) { o.cycle_chain = args.to_vec(); }
            a.log.push(format!("[muimaster] MUIM_Window_SetCycleChain: {} objects recorded; the seam has no focus order, so the host keeps its own", args.len()));
            Ok(0)
        }
        MUIM_Application_CheckRefresh if kind == Kind::Application => Ok(0),
        MUIM_Application_InputBuffered if kind == Kind::Application => { app::pump(a, obj)?; Ok(0) }
        MUIM_Group_InitChange if kind.is_container() => Ok(1),
        MUIM_Group_ExitChange if kind.is_container() => { super::win::refresh(a, obj)?; Ok(1) }
        // Application.mui: the windows are the members of its window family; a window added after creation joins it and learns the application
        // (MUIM_ConnectParent), exactly what MUIA_Application_Window does at creation.
        OM_ADDMEMBER if kind == Kind::Application => { object::adopt(a, obj, MUIA_Application_Window, arg(0)?)?; Ok(1) }
        OM_REMMEMBER if kind == Kind::Application => {
            // Window__MUIM_DisconnectParent: a window leaving the application closes first.
            let w = arg(0)?;
            if a.objects.flag(w, MUIA_Window_Open) { object::set_attr(a, w, MUIA_Window_Open, 0)?; }
            remove_child(a, obj, w)?;
            Ok(1)
        }
        OM_ADDMEMBER if kind.is_container() => { add_child(a, obj, arg(0)?, false)?; Ok(0) }
        OM_REMMEMBER if kind.is_container() => { remove_child(a, obj, arg(0)?)?; Ok(0) }
        MUIM_Family_AddTail | MUIM_Family_AddHead if kind.is_a(Kind::Family) => { add_child(a, obj, arg(0)?, id == MUIM_Family_AddHead)?; Ok(0) }
        MUIM_Family_Remove if kind.is_a(Kind::Family) => { remove_child(a, obj, arg(0)?)?; Ok(0) }
        MUIM_Numeric_Increase | MUIM_Numeric_Decrease if kind.is_a(Kind::Numeric) => {
            let step = arg(0)? as i32;
            let step = if id == MUIM_Numeric_Increase { step } else { -step };
            let v = (a.objects.num_or(obj, MUIA_Numeric_Value, 0) as i32).saturating_add(step);
            object::set_attr(a, obj, MUIA_Numeric_Value, v as u32)?;
            Ok(0)
        }
        MUIM_List_InsertSingle if kind == Kind::List => { let r = list::insert_single(a, obj, arg(0)?, arg(1)?)?; super::win::refresh(a, obj)?; Ok(r) }
        MUIM_List_Insert if kind == Kind::List => { let r = list::insert_array(a, obj, arg(0)?, arg(1)?, arg(2)?)?; super::win::refresh(a, obj)?; Ok(r) }
        MUIM_List_Remove if kind == Kind::List => { list::remove(a, obj, arg(0)?)?; super::win::refresh(a, obj)?; Ok(0) }
        MUIM_List_Clear if kind == Kind::List => { list::clear(a, obj)?; super::win::refresh(a, obj)?; Ok(0) }
        MUIM_List_GetEntry if kind == Kind::List => { list::get_entry(a, obj, arg(0)?, arg(1)?)?; Ok(0) }
        MUIM_List_Redraw if kind == Kind::List => { list::redraw(a, obj, arg(0)?)?; super::win::refresh(a, obj)?; Ok(0) }
        MUIM_List_Jump if kind == Kind::List => { super::win::refresh(a, obj)?; Ok(0) }
        // Listview.mui passes the List methods to its List (listview.c:247-266).
        MUIM_List_Clear | MUIM_List_Exchange | MUIM_List_GetEntry | MUIM_List_Insert | MUIM_List_InsertSingle | MUIM_List_Jump | MUIM_List_NextSelected
        | MUIM_List_Redraw | MUIM_List_Remove | MUIM_List_Select if kind == Kind::Listview => {
            let l = list::list_of(&a.objects, obj).ok_or_else(|| Refusal("MUI Listview has no List".into()))?;
            method(a, l, id, args)
        }
        _ if kind.is_a(Kind::Semaphore) => {
            if let Some(r) = store::semaphore_method(a, obj, id)? { return Ok(r); }
            if kind == Kind::Dataspace { if let Some(r) = store::dataspace_method(a, obj, id, args)? { return Ok(r); } }
            Err(bad_kind(kind, id))
        }
        _ => Err(bad_kind(kind, id)),
    }
}

/// The object below `root` (or `root` itself) whose `MUIA_UserData` is `udata`.
fn find_udata<U: GuestUi>(a: &Amiga<U>, root: u32, udata: u32) -> Option<u32> {
    if a.objects.num(root, MUIA_UserData) == Some(udata) { return Some(root); }
    a.objects.children(root).iter().find_map(|&c| find_udata(a, c, udata))
}

fn set_udata<U: GuestUi>(a: &mut Amiga<U>, root: u32, udata: u32, attr: u32, val: u32, all: bool) -> Result<(), Refusal> {
    if a.objects.num(root, MUIA_UserData) == Some(udata) { object::set_attr(a, root, attr, val)?; if !all { return Ok(()); } }
    for c in a.objects.children(root).to_vec() { set_udata(a, c, udata, attr, val, all)?; }
    Ok(())
}

/// A child added to a live container (OM_ADDMEMBER, MUIM_Family_AddTail / AddHead): the container takes ownership.
fn add_child<U: GuestUi>(a: &mut Amiga<U>, parent: u32, child: u32, head: bool) -> Result<(), Refusal> {
    let tag = if a.objects.kind(parent).is_some_and(|k| k.is_a(Kind::Family)) { MUIA_Family_Child } else { MUIA_Group_Child };
    object::adopt(a, parent, tag, child)?;
    if head { if let Some(p) = a.objects.map.get_mut(&parent) { p.children.pop(); p.children.insert(0, child); } }
    super::win::refresh(a, parent)
}

/// A child taken out of its container: the program owns it again.
fn remove_child<U: GuestUi>(a: &mut Amiga<U>, parent: u32, child: u32) -> Result<(), Refusal> {
    let owned = a.objects.map.get(&child).is_some_and(|c| c.parent == Some(parent));
    if !owned { return Err(Refusal(format!("remove member: {child:#x} is not a child of {parent:#x}"))); }
    if let Some(p) = a.objects.map.get_mut(&parent) { p.children.retain(|&c| c != child); }
    if let Some(c) = a.objects.map.get_mut(&child) { c.parent = None; }
    super::win::refresh(a, parent)
}
