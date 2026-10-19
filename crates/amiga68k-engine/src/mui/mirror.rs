//! The part of a MUI object a guest subclass reads straight out of the object's memory. A class written in C reaches the data of
//! the classes it inherits from through macros that are plain structure offsets (Zune's macros.h `muiGlobalInfo(obj)`, `_app(obj)`,
//! `muiUserData(obj)`, `_left(obj)`, `_mwidth(obj)`, `_rp(obj)`, ...: `struct __dummyAreaData__ { struct MUI_NotifyData mnd; struct
//! MUI_AreaData mad; }` laid over the object pointer). The state itself stays host-side ([`crate::boopsi`]); this module WRITES the
//! public prefix of those two structures into the instance block rootclass allocated, and the guest only ever reads it (a guest write
//! there is never read back). Only objects of a guest class have such a block; a native object has none to mirror.
//!
//! Layout (computed by hand from AROS workbench/libs/muimaster/classes/notify.h and area.h with m68k alignment, -mno-align-int):
//! `MUI_NotifyData` = mnd_GlobalInfo 0, mnd_UserData 4, mnd_ObjectID 8, four private longs = 28 bytes; `MUI_AreaData` (at 28)
//! = mad_RenderInfo 0, mad_Background 4, mad_Font 8, mad_MinMax 12 (six WORDs), mad_Box 24 (four WORDs), mad_addleft/addtop/
//! subwidth/subheight 32 (four BYTEs), mad_Flags 36, mad_Flags2 40; the private tail is not mirrored, and its room is reserved
//! ([`AREA_SIZE`]: a generous bound over the fields named in area.h, which are zero here). `struct MUI_GlobalInfo` = priv0 0,
//! mgi_ApplicationObject 4, then private pointers.
use super::classes::Kind;
use super::ids_gen::MUIA_UserData;
use crate::{machine::Amiga, Refusal};
use guestui_abi::GuestUi;

/// sizeof(struct MUI_NotifyData) and the room reserved for struct MUI_AreaData (the class instance sizes of Notify.mui / Area.mui).
pub const NOTIFY_SIZE: u32 = 28;
pub const AREA_SIZE: u32 = 256;
const MND_GLOBAL_INFO: u32 = 0;
const MND_USER_DATA: u32 = 4;
/// `struct MUI_GlobalInfo`: priv0, mgi_ApplicationObject, and room for the private pointers.
pub const GLOBAL_INFO_SIZE: u32 = 32;
const MGI_APPLICATION_OBJECT: u32 = 4;
/// Offset of `mad` in the object (after `mnd`).
pub const MAD: u32 = NOTIFY_SIZE;

/// The block of `obj` when a guest class made it (so a guest can read it); `None` for a native object.
fn block<U: GuestUi>(a: &Amiga<U>, obj: u32) -> Option<u32> { a.classes.objs.contains_key(&obj).then_some(obj) }

fn put<U: GuestUi>(a: &mut Amiga<U>, at: u32, v: u32, what: &'static str) -> Result<(), Refusal> { a.mem.write_u32(at, v).map_err(Refusal::mem(what)) }

/// The Application above `obj` (or `obj` itself).
fn app_of<U: GuestUi>(a: &Amiga<U>, obj: u32) -> Option<u32> {
    if a.objects.kind(obj) == Some(Kind::Application) { Some(obj) } else { super::object::ancestor(&a.objects, obj, Kind::Application) }
}

/// The `struct MUI_GlobalInfo` of `app`, made on first need.
fn global_info<U: GuestUi>(a: &mut Amiga<U>, app: u32) -> Result<u32, Refusal> {
    if let Some(&g) = a.objects.global_info.get(&app) { return Ok(g); }
    let g = crate::memory::alloc_zeroed(a, GLOBAL_INFO_SIZE, "MUI_GlobalInfo")?;
    put(a, g + MGI_APPLICATION_OBJECT, app, "MUI_GlobalInfo.mgi_ApplicationObject")?;
    a.objects.global_info.insert(app, g);
    Ok(g)
}

/// Write `obj`'s `MUI_NotifyData`: its user data and, when it hangs below an Application, that application's global info.
pub fn notify_data<U: GuestUi>(a: &mut Amiga<U>, obj: u32) -> Result<(), Refusal> {
    let Some(b) = block(a, obj) else { return Ok(()) };
    let user = a.objects.num_or(obj, MUIA_UserData, 0);
    put(a, b + MND_USER_DATA, user, "MUI_NotifyData.mnd_UserData")?;
    let g = match app_of(a, obj) { Some(app) => global_info(a, app)?, None => 0 };
    put(a, b + MND_GLOBAL_INFO, g, "MUI_NotifyData.mnd_GlobalInfo")
}

/// `child` (and everything below it) was just attached to a tree: the objects now below an Application learn its global info.
pub fn connected<U: GuestUi>(a: &mut Amiga<U>, child: u32) -> Result<(), Refusal> {
    notify_data(a, child)?;
    let kids: alloc::vec::Vec<u32> = a.objects.children(child).to_vec();
    kids.into_iter().try_for_each(|k| connected(a, k))
}

/// `MUIA_UserData` was stored: the mirror follows.
pub fn user_data<U: GuestUi>(a: &mut Amiga<U>, obj: u32, v: u32) {
    if let Some(b) = block(a, obj) { let _ = a.mem.write_u32(b + MND_USER_DATA, v); }
}

/// The application is gone: its global info goes back to the heap.
pub fn forget_application<U: GuestUi>(a: &mut Amiga<U>, app: u32) {
    if let Some(g) = a.objects.global_info.remove(&app) { let _ = a.heap.free(g, GLOBAL_INFO_SIZE); }
}

/// The instance size the native class of `kind` has: Notify.mui holds a `MUI_NotifyData`, Area.mui a `MUI_AreaData`; every other
/// class's data is the engine's own (host-side), so a subclass's data follows right after its nearest sized ancestor.
pub const fn instance_size(kind: Kind) -> u16 {
    match kind { Kind::Notify => NOTIFY_SIZE as u16, Kind::Area => AREA_SIZE as u16, _ => 0 }
}
