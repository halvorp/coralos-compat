//! Semaphore.mui, Dataspace.mui and the Application's saved settings (`MUIM_Application_Load` / `Save`).
//!
//! * **Semaphore.mui** (Zune semaphore.c; super: rootclass): `MUIM_Semaphore_Obtain/ObtainShared/Attempt/AttemptShared/Release` over a
//!   `struct SignalSemaphore`'s rules. The scheduler here is cooperative and has no wait queue for a MUI object, so an Obtain that WOULD
//!   block (held by another task, or exclusive over shared holders) is a named refusal, never a hang; Attempt answers FALSE as it does on Exec.
//! * **Dataspace.mui** (dataspace.c): a list of `(id, bytes)`; `Add` replaces an id, `Find` returns the guest address of the bytes (the node data
//!   lives in guest memory, as in Zune), `Remove`, `Clear`, `Merge` (Zune's is a stub that answers 1; so is this). Reading/writing a program's own
//!   `IFFHandle` (`ReadIFF` / `WriteIFF`) needs iffparse.library and is a named refusal.
//! * **Load / Save** (application.c): the file is `ENV:Zune/<MUIA_Application_Base>.cfg` (`MUIV_Application_Load_ENV` = 0), `ENVARC:Zune/...`
//!   (`~0`) or the given name; an IFF `FORM PREF` holding a `PRHD` (a `struct FilePrefHeader`, 6 bytes) and a `MUIO` chunk of
//!   `[id: ULONG BE][len: ULONG BE][bytes]` entries. Save exports the application's windows, Load imports into them. The export/import of the
//!   NATIVE classes follows Zune: a Group passes the method to its children, a Window to its root object, Cycle/Radio store their active entry
//!   as a LONG, Numeric its value, String its contents (with the NUL), any other Area its selected state as a byte, Text nothing; each only
//!   for an object with a `MUIA_ObjectID`. An object of a program's own class would be sent `MUIM_Export` / `MUIM_Import` by Zune; that call is
//!   NOT made here and the log line says so (docs/MUI-NATIVE.md).
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use super::classes::Kind;
use super::ids_gen::*;
use super::object;
use crate::boopsi::Ext;
use crate::{machine::Amiga, Refusal};
use guestui_abi::GuestUi;

/// A `struct SignalSemaphore`'s state: the exclusive holder (task, nesting) and the number of shared holders.
#[derive(Default, Debug)]
pub struct Sem { excl: Option<(u32, u32)>, shared: u32 }

/// One dataspace item: its id and the guest block holding its bytes.
#[derive(Debug)]
pub struct Entry { pub id: u32, pub addr: u32, pub len: u32 }

fn sem<U: GuestUi>(a: &mut Amiga<U>, obj: u32) -> Result<&mut Sem, Refusal> {
    match a.objects.map.get_mut(&obj).map(|o| &mut o.ext) { Some(Ext::Sem(s)) => Ok(s), _ => Err(Refusal(format!("{obj:#x} is not a Semaphore.mui object"))) }
}

/// The Semaphore methods. `None` = not one of them.
pub fn semaphore_method<U: GuestUi>(a: &mut Amiga<U>, obj: u32, id: u32) -> Result<Option<u32>, Refusal> {
    if !matches!(id, MUIM_Semaphore_Obtain | MUIM_Semaphore_ObtainShared | MUIM_Semaphore_Attempt | MUIM_Semaphore_AttemptShared | MUIM_Semaphore_Release) { return Ok(None); }
    let me = a.sched.cur;
    let s = sem(a, obj)?;
    let blocked_excl = s.shared > 0 || s.excl.is_some_and(|(t, _)| t != me);
    let blocked_shared = s.excl.is_some_and(|(t, _)| t != me);
    let name = super::attrs::method_name(id);
    Ok(Some(match id {
        MUIM_Semaphore_Obtain | MUIM_Semaphore_Attempt => {
            if blocked_excl {
                if id == MUIM_Semaphore_Attempt { return Ok(Some(0)); }
                return Err(Refusal(format!("{name} on {obj:#x} would block (held by another task or by shared holders): waiting on a Semaphore.mui is not modelled")));
            }
            s.excl = Some((me, s.excl.map_or(1, |(_, n)| n + 1)));
            1
        }
        MUIM_Semaphore_ObtainShared | MUIM_Semaphore_AttemptShared => {
            if blocked_shared {
                if id == MUIM_Semaphore_AttemptShared { return Ok(Some(0)); }
                return Err(Refusal(format!("{name} on {obj:#x} would block (held exclusively by another task): waiting on a Semaphore.mui is not modelled")));
            }
            match s.excl { Some((t, n)) => s.excl = Some((t, n + 1)), None => s.shared += 1 } // the exclusive owner nests, as Exec lets it
            1
        }
        _ => {
            match (&mut s.excl, s.shared) {
                (Some((t, n)), _) if *t == me => { if *n > 1 { *n -= 1 } else { s.excl = None } }
                (None, n) if n > 0 => s.shared -= 1,
                _ => return Err(Refusal(format!("MUIM_Semaphore_Release on {obj:#x}: this task does not hold it"))),
            }
            0
        }
    }))
}

fn space<U: GuestUi>(a: &mut Amiga<U>, obj: u32) -> Result<&mut Vec<Entry>, Refusal> {
    match a.objects.map.get_mut(&obj).map(|o| &mut o.ext) { Some(Ext::Space(v)) => Ok(v), _ => Err(Refusal(format!("{obj:#x} is not a Dataspace.mui object"))) }
}

/// `MUIM_Dataspace_Add`: copy `len` bytes from `data` under `id`, replacing an item with that id. Zune answers 0 for no data, len < 1 or id < 1.
pub fn space_add<U: GuestUi>(a: &mut Amiga<U>, obj: u32, data: u32, len: i32, id: u32) -> Result<u32, Refusal> {
    if data == 0 || len < 1 || id < 1 { return Ok(0); }
    let bytes: Vec<u8> = (0..len as u32).map(|i| a.mem.read_u8(data + i).map_err(Refusal::mem("MUIM_Dataspace_Add data"))).collect::<Result<_, _>>()?;
    space_put(a, obj, id, &bytes)
}

fn space_put<U: GuestUi>(a: &mut Amiga<U>, obj: u32, id: u32, bytes: &[u8]) -> Result<u32, Refusal> {
    let addr = a.heap.alloc(bytes.len() as u32).ok_or_else(|| Refusal("MUIM_Dataspace_Add: no memory (NOMEM)".into()))?;
    a.mem.write_bytes(addr, bytes).map_err(Refusal::mem("Dataspace node"))?;
    let old = { let v = space(a, obj)?; let at = v.iter().position(|e| e.id == id); let old = at.map(|i| v.remove(i)); v.push(Entry { id, addr, len: bytes.len() as u32 }); old };
    if let Some(e) = old { let _ = a.heap.free(e.addr, e.len); }
    Ok(1)
}

fn space_find<U: GuestUi>(a: &mut Amiga<U>, obj: u32, id: u32) -> Result<Option<(u32, u32)>, Refusal> {
    Ok(space(a, obj)?.iter().find(|e| e.id == id).map(|e| (e.addr, e.len)))
}

fn space_remove<U: GuestUi>(a: &mut Amiga<U>, obj: u32, id: u32) -> Result<u32, Refusal> {
    let v = space(a, obj)?;
    let Some(i) = v.iter().position(|e| e.id == id) else { return Ok(0) };
    let e = v.remove(i);
    let _ = a.heap.free(e.addr, e.len);
    Ok(1)
}

/// The Dataspace methods. `None` = not one of them.
pub fn dataspace_method<U: GuestUi>(a: &mut Amiga<U>, obj: u32, id: u32, args: &[u32]) -> Result<Option<u32>, Refusal> {
    let arg = |i: usize| args.get(i).copied().ok_or_else(|| Refusal(format!("{}: missing argument {i}", super::attrs::method_name(id))));
    Ok(Some(match id {
        MUIM_Dataspace_Add => space_add(a, obj, arg(0)?, arg(1)? as i32, arg(2)?)?,
        MUIM_Dataspace_Find => space_find(a, obj, arg(0)?)?.map_or(0, |(addr, _)| addr),
        MUIM_Dataspace_Remove => space_remove(a, obj, arg(0)?)?,
        MUIM_Dataspace_Clear => { let all: Vec<Entry> = core::mem::take(space(a, obj)?); for e in all { let _ = a.heap.free(e.addr, e.len); } 1 }
        MUIM_Dataspace_Merge => 1, // dataspace.c: `return 1;`
        MUIM_Dataspace_ReadIFF | MUIM_Dataspace_WriteIFF => return Err(Refusal(format!("{}: a program's IFFHandle needs iffparse.library (not modelled); Application Load/Save read and write their files directly", super::attrs::method_name(id)))),
        _ => return Ok(None),
    }))
}

// ---- the file: FORM PREF { PRHD, MUIO } -----------------------------------------------------------------------------------------------

const ID_FORM: &[u8; 4] = b"FORM";
const ID_PREF: &[u8; 4] = b"PREF";
const ID_PRHD: &[u8; 4] = b"PRHD";
const ID_MUIO: &[u8; 4] = b"MUIO";
/// `struct FilePrefHeader { UBYTE ph_Version, ph_Type; UBYTE ph_Flags[4]; }`, PHV_CURRENT 0 (prefs/prefhdr.h).
const PRHD: [u8; 6] = [0; 6];

fn be32(v: u32) -> [u8; 4] { v.to_be_bytes() }

/// The bytes of a settings file holding `items`.
pub fn encode(items: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let muio: Vec<u8> = items.iter().flat_map(|(id, d)| be32(*id).into_iter().chain(be32(d.len() as u32)).chain(d.iter().copied()).collect::<Vec<u8>>()).collect();
    let mut body = Vec::new();
    body.extend_from_slice(ID_PREF);
    for (id, data) in [(ID_PRHD, PRHD.as_slice()), (ID_MUIO, muio.as_slice())] {
        body.extend_from_slice(id); body.extend_from_slice(&be32(data.len() as u32)); body.extend_from_slice(data);
        if data.len() % 2 == 1 { body.push(0); }
    }
    let mut out = Vec::from(*ID_FORM);
    out.extend_from_slice(&be32(body.len() as u32));
    out.extend(body);
    out
}

/// The items of the `MUIO` chunk of a `FORM PREF` file; a file that is not one, or has no such chunk, holds none (ParseIFF would not find it).
pub fn decode(file: &[u8]) -> Vec<(u32, Vec<u8>)> {
    let mut items = Vec::new();
    if file.len() < 12 || &file[0..4] != ID_FORM || &file[8..12] != ID_PREF { return items; }
    let mut at = 12;
    while at + 8 <= file.len() {
        let size = u32::from_be_bytes(file[at + 4..at + 8].try_into().expect("4 bytes")) as usize;
        let data = &file[(at + 8).min(file.len())..(at + 8 + size).min(file.len())];
        if &file[at..at + 4] == ID_MUIO {
            let mut p = 0;
            // dataspace.c ReadIFF: an item whose length runs past the chunk ends the walk.
            while p + 8 <= data.len() {
                let (id, len) = (u32::from_be_bytes(data[p..p + 4].try_into().expect("4")), u32::from_be_bytes(data[p + 4..p + 8].try_into().expect("4")) as usize);
                p += 8;
                if len > data.len() - p { break; }
                items.push((id, data[p..p + len].to_vec()));
                p += len;
            }
            return items;
        }
        at += 8 + size + (size & 1);
    }
    items
}

// ---- export / import over the native classes -------------------------------------------------------------------------------------------

/// Visit `obj` and everything MUIM_Export / MUIM_Import would reach: (object, kind) for every object that stores something.
fn exporters<U: GuestUi>(a: &Amiga<U>, obj: u32, out: &mut Vec<u32>, skipped: &mut Vec<u32>) {
    let Some(kind) = a.objects.kind(obj) else { return };
    if super::custom::own_class(a, obj).is_some() { skipped.push(obj); }
    match kind {
        Kind::Window | Kind::Group | Kind::Virtgroup | Kind::Scrollgroup | Kind::Register => { for &c in a.objects.children(obj) { exporters(a, c, out, skipped); } }
        Kind::Text => {} // text.c: export/import are `#if 0`
        k if k.is_area() => { if a.objects.num_or(obj, MUIA_ObjectID, 0) != 0 { out.push(obj); } }
        _ => {}
    }
}

fn exported<U: GuestUi>(a: &Amiga<U>, obj: u32) -> Option<Vec<u8>> {
    let k = a.objects.kind(obj)?;
    Some(match k {
        Kind::Cycle => a.objects.num_or(obj, MUIA_Cycle_Active, 0).to_be_bytes().to_vec(),
        Kind::Radio => a.objects.num_or(obj, MUIA_Radio_Active, 0).to_be_bytes().to_vec(),
        k if k.is_a(Kind::Numeric) => a.objects.num_or(obj, MUIA_Numeric_Value, 0).to_be_bytes().to_vec(),
        Kind::String => { let mut s = a.objects.text(obj, MUIA_String_Contents).as_bytes().to_vec(); s.push(0); s }
        _ => alloc::vec![a.objects.flag(obj, MUIA_Selected) as u8],
    })
}

fn imported<U: GuestUi>(a: &mut Amiga<U>, obj: u32, data: &[u8]) -> Result<(), Refusal> {
    let Some(k) = a.objects.kind(obj) else { return Ok(()) };
    let long = || data.get(..4).map(|b| u32::from_be_bytes(b.try_into().expect("4")));
    match k {
        Kind::Cycle => if let Some(v) = long() { object::set_attr(a, obj, MUIA_Cycle_Active, v)?; },
        Kind::Radio => if let Some(v) = long() { object::set_attr(a, obj, MUIA_Radio_Active, v)?; },
        k if k.is_a(Kind::Numeric) => if let Some(v) = long() { object::set_attr(a, obj, MUIA_Numeric_Value, v)?; },
        Kind::String => {
            // set(obj, MUIA_String_Contents, s): a C string; here the object copies it from guest memory, so stage it there.
            let text: Vec<u8> = data.iter().copied().take_while(|&b| b != 0).collect();
            let at = a.heap.alloc(text.len() as u32 + 1).ok_or_else(|| Refusal("MUIM_Import: no memory (NOMEM)".into()))?;
            a.mem.write_bytes(at, &text).map_err(Refusal::mem("MUIM_Import string"))?;
            a.mem.write_u8(at + text.len() as u32, 0).map_err(Refusal::mem("MUIM_Import string"))?;
            let r = object::set_attr(a, obj, MUIA_String_Contents, at);
            let _ = a.heap.free(at, text.len() as u32 + 1);
            r?;
        }
        _ => if let Some(&b) = data.first() { object::set_attr(a, obj, MUIA_Selected, b as u32)?; },
    }
    Ok(())
}

/// The path `name` stands for (`MUIV_Application_Load/Save_ENV` = 0, `_ENVARC` = ~0, else a C string).
fn path<U: GuestUi>(a: &Amiga<U>, app: u32, name: u32) -> Result<Option<String>, Refusal> {
    let base = a.objects.text(app, MUIA_Application_Base);
    if base.is_empty() { return Ok(None); } // application.c: `if (!data->app_Base) return 0;`
    Ok(Some(match name {
        0 => format!("ENV:Zune/{base}.cfg"),
        u32::MAX => format!("ENVARC:Zune/{base}.cfg"),
        p => a.mem.read_cstr(p).map_err(Refusal::mem("MUIM_Application_Load/Save name"))?,
    }))
}

fn windows<U: GuestUi>(a: &Amiga<U>, app: u32) -> Vec<u32> { a.objects.children(app).iter().copied().filter(|&w| a.objects.kind(w) == Some(Kind::Window)).collect() }

/// `MUIM_Application_Save(name)`: export the windows' objects and write the file. Returns what the log says.
pub fn save<U: GuestUi>(a: &mut Amiga<U>, app: u32, name: u32) -> Result<String, Refusal> {
    let Some(path) = path(a, app, name)? else { return Ok("no MUIA_Application_Base: nothing saved".into()) };
    let (mut objs, mut skipped) = (Vec::new(), Vec::new());
    for w in windows(a, app) { exporters(a, w, &mut objs, &mut skipped); }
    let mut items: Vec<(u32, Vec<u8>)> = Vec::new();
    for o in objs {
        let (id, data) = (a.objects.num_or(o, MUIA_ObjectID, 0), exported(a, o).unwrap_or_default());
        items.retain(|(i, _)| *i != id); // Dataspace_Add replaces
        items.push((id, data));
    }
    // Open(name, MODE_NEWFILE): the directory must exist (a real system has ENV:Zune); it is made here so a fresh volume can keep settings.
    if let Some((dir, _)) = path.rsplit_once('/') {
        let cwd = crate::fs::cur_dir(a);
        if a.fs.vol.resolve(cwd, dir).is_err() { let _ = a.fs.vol.mkdirs(dir); }
    }
    let n = items.len();
    let wrote = crate::fs::write_file(a, &path, encode(&items)).is_ok();
    Ok(format!("{n} items -> {path}{}{}", if wrote { "" } else { " (could not be opened: nothing saved)" }, skip_note(a, &skipped)))
}

/// `MUIM_Application_Load(name)`: read the file (a missing one imports nothing) and import into the windows' objects.
pub fn load<U: GuestUi>(a: &mut Amiga<U>, app: u32, name: u32) -> Result<String, Refusal> {
    let Some(path) = path(a, app, name)? else { return Ok("no MUIA_Application_Base: nothing loaded".into()) };
    let cwd = crate::fs::cur_dir(a);
    let file = a.fs.vol.resolve(cwd, &path).ok().and_then(|n| a.fs.vol.file_data(n).map(<[u8]>::to_vec)).unwrap_or_default();
    let items = decode(&file);
    let (mut objs, mut skipped) = (Vec::new(), Vec::new());
    for w in windows(a, app) { exporters(a, w, &mut objs, &mut skipped); }
    let mut applied = 0;
    for o in objs {
        let id = a.objects.num_or(o, MUIA_ObjectID, 0);
        if let Some((_, data)) = items.iter().find(|(i, _)| *i == id) { imported(a, o, data)?; applied += 1; }
    }
    Ok(format!("{path}: {} items, {applied} imported{}", items.len(), skip_note(a, &skipped)))
}

fn skip_note<U: GuestUi>(a: &Amiga<U>, skipped: &[u32]) -> String {
    if skipped.is_empty() { return String::new(); }
    let names: Vec<String> = skipped.iter().map(|&o| a.classes.name_of(a.classes.class_of(o).unwrap_or(0))).collect();
    format!("; MUIM_Export/Import NOT sent to {} program-class object(s): {}", skipped.len(), names.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_file_round_trips_and_a_truncated_or_foreign_file_holds_nothing() {
        let items = alloc::vec![(7, alloc::vec![1u8, 2, 3]), (9, alloc::vec![0, 0, 0, 5])];
        let bytes = encode(&items);
        assert_eq!(&bytes[0..4], b"FORM");
        assert_eq!(decode(&bytes), items);
        assert_eq!(bytes.len() % 2, 0, "IFF chunks are even-padded");
        // NEGATIVE CONTROLS
        assert!(decode(&bytes[..bytes.len() - 3]).len() < items.len(), "a truncated MUIO chunk loses the cut item");
        assert!(decode(b"FORM\0\0\0\x04ILBM").is_empty(), "another FORM type");
        assert!(decode(b"junk").is_empty());
    }
}
