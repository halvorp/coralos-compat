//! icon.library: DiskObjects from the real `.info` files on the volume (format: icon_format.rs).
//!
//! A DiskObject handed to the guest is ONE heap block holding the `struct DiskObject`, its DrawerData, Images and
//! strings, so FreeDiskObject is a single checked free; the block's address and size are host-side bookkeeping
//! (`FsState::icons`), not something the guest can corrupt. Guest struct offsets: m68k gcc -m68020 -mno-align-int over
//! the AROS workbench/workbench.h and intuition field lists (2026-10-18): DiskObject 78 bytes (do_Gadget 4, do_Type 48,
//! do_DefaultTool 50, do_ToolTypes 54, do_CurrentX 58, do_CurrentY 62, do_DrawerData 66, do_ToolWindow 70,
//! do_StackSize 74), Gadget 44 (GadgetRender 18, SelectRender 22), Image 20 (ImageData 10), DrawerData 62
//! (dd_CurrentX 48, dd_Flags 56, dd_ViewModes 60).
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use alloc::vec;
use crate::dos_err::*;
use crate::fs::*;
use crate::guest_str::read_bytes_z;
use crate::icon_format::*;
use crate::libtable::LibraryBuilder;
use crate::volume::Kind;
use crate::{machine::Amiga, Refusal};
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

const WBDISK: u8 = 1;
const WBDRAWER: u8 = 2;
const WBTOOL: u8 = 3;
const WBPROJECT: u8 = 4;
const WBGARBAGE: u8 = 5;
const WBDEVICE: u8 = 6;
const WBKICK: u8 = 7;
const WBAPPICON: u8 = 8;
/// Workbench "no position yet" (workbench/workbench.h NO_ICON_POSITION).
const NO_ICON_POSITION: u32 = 0x8000_0000;
const DRAWER_MEM: usize = OLD_DRAWER + NEW_DRAWER;
const MAX_STR: usize = 0x1_0000;
const MAX_TOOL_TYPES: u32 = 4096;
const TRUE: u32 = 1;

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.named("GetDiskObject", get_disk_object::<U>)
        .named("PutDiskObject", put_disk_object::<U>)
        .named("FreeDiskObject", free_disk_object::<U>)
        .named("FindToolType", find_tool_type::<U>)
        .named("MatchToolValue", match_tool_value::<U>)
        .named("GetDefDiskObject", get_def_disk_object::<U>)
        .named("GetDiskObjectNew", get_disk_object_new::<U>)
        .named("DeleteDiskObject", delete_disk_object::<U>)
        .named("DupDiskObjectA", dup_disk_object::<U>)
        .named("NewDiskObject", new_disk_object::<U>)
}

/// The `.info` file name for an object: `Dev:` is described by `Dev:Disk.info`, everything else by `name.info`.
fn info_path(name: &str) -> String { if name.ends_with(':') { format!("{name}Disk.info") } else { format!("{name}.info") } }

/// The engine's own stand-in for Workbench's default icon of `ty`: a 24x16 one-plane outlined box (not Commodore
/// artwork), at no position, with the 4 KiB default stack; drawer-like types carry DrawerData (a 400x150 window).
pub fn default_icon(ty: u8) -> Icon {
    box_icon(ty)
}

/// The default icon the guest gets: the host's artwork for the type's role (icon_host.rs) when it has some, else the
/// placeholder box. Returns the log suffix naming which one, and why the box (FALLBACK-BOX) when it is the box.
fn default_for<U: GuestUi>(a: &mut Amiga<U>, ty: u8) -> (Icon, String) {
    let boxed = box_icon(ty);
    let Some(role) = crate::icon_host::role_of(ty) else { return (boxed, " [placeholder box: no icon role for this type]".into()) };
    let fresh;
    let pal = match a.gfx.palette.as_ref() { Some(p) => p, None => { fresh = crate::graphics::state::Palette::new(); &fresh } };
    match crate::icon_host::host_image(&mut a.ui, role, pal) {
        Ok(img) => (crate::icon_host::with_image(boxed, img), format!(" [host artwork: {role:?}]")),
        Err(why) => (boxed, format!(" [FALLBACK-BOX: {why}]")),
    }
}

fn box_icon(ty: u8) -> Icon {
    let mut head = [0u8; HEAD];
    let w16 = |h: &mut [u8], at: usize, v: u16| h[at..at + 2].copy_from_slice(&v.to_be_bytes());
    let w32 = |h: &mut [u8], at: usize, v: u32| h[at..at + 4].copy_from_slice(&v.to_be_bytes());
    w16(&mut head, 0, MAGIC);
    w16(&mut head, 2, 1);
    for (at, v) in [(12, 24), (14, 16), (16, 0x0006), (18, 3), (20, 1)] { w16(&mut head, at, v); } // Gadget Width, Height, Flags (GADGIMAGE|GADGHIMAGE), Activation (RELVERIFY|IMMEDIATE), GadgetType (BOOLGADGET)
    head[O_DO_TYPE] = ty;
    w32(&mut head, 58, NO_ICON_POSITION);
    w32(&mut head, 62, NO_ICON_POSITION);
    w32(&mut head, 74, 4096);
    let mut hdr = [0u8; IMAGE_HDR];
    for (at, v) in [(4, 24u16), (6, 16), (8, 1)] { hdr[at..at + 2].copy_from_slice(&v.to_be_bytes()); }
    hdr[14] = 1; // PlanePick
    let mut data = vec![0u8; 4 * 16];
    for (row, r) in data.chunks_mut(4).enumerate() {
        let edge = row == 0 || row == 15;
        r.copy_from_slice(if edge { &[0xFF, 0xFF, 0xFF, 0x00] } else { &[0x80, 0x00, 0x01, 0x00] });
    }
    let drawerish = matches!(ty, WBDISK | WBDRAWER | WBGARBAGE | WBDEVICE | WBKICK);
    let drawer = drawerish.then(|| { let mut d = [0u8; OLD_DRAWER]; d[4..6].copy_from_slice(&400u16.to_be_bytes()); d[6..8].copy_from_slice(&150u16.to_be_bytes()); d[48..52].copy_from_slice(&NO_ICON_POSITION.to_be_bytes()); d[52..56].copy_from_slice(&NO_ICON_POSITION.to_be_bytes()); d });
    Icon { head, drawer, drawer_new: drawerish.then_some([0; NEW_DRAWER]), images: [Some(Image { hdr, data }), None], default_tool: None, tool_types: None, tool_window: None, trailer: vec![] }
}

fn pad4(b: &mut Vec<u8>) { while b.len() % 4 != 0 { b.push(0); } }

/// The guest image of `icon` as if the block started at `base`: every pointer field is `base + offset`.
fn build(icon: &Icon, base: u32) -> Vec<u8> {
    let mut b = icon.head.to_vec();
    pad4(&mut b);
    let ptr = |b: &mut Vec<u8>, field: usize, at: usize| b[field..field + 4].copy_from_slice(&(base + at as u32).to_be_bytes());
    if let Some(d) = &icon.drawer {
        let at = b.len();
        b.extend_from_slice(d);
        b.extend_from_slice(&icon.drawer_new.unwrap_or([0; NEW_DRAWER]));
        pad4(&mut b);
        ptr(&mut b, O_DRAWER_DATA, at);
    }
    for (i, field) in [O_GADGET_RENDER, O_SELECT_RENDER].into_iter().enumerate() {
        let Some(img) = &icon.images[i] else { continue };
        let at = b.len();
        b.extend_from_slice(&img.hdr);
        if !img.data.is_empty() { b[at + 10..at + 14].copy_from_slice(&(base + (at + IMAGE_HDR) as u32).to_be_bytes()); }
        b.extend_from_slice(&img.data);
        pad4(&mut b);
        ptr(&mut b, field, at);
    }
    for (s, field) in [(&icon.default_tool, O_DEFAULT_TOOL), (&icon.tool_window, O_TOOL_WINDOW)] {
        if let Some(s) = s { let at = b.len(); b.extend_from_slice(s); b.push(0); pad4(&mut b); ptr(&mut b, field, at); }
    }
    if let Some(t) = &icon.tool_types {
        let arr = b.len();
        b.resize(arr + (t.len() + 1) * 4, 0);
        for (i, s) in t.iter().enumerate() {
            let at = b.len();
            b.extend_from_slice(s);
            b.push(0);
            pad4(&mut b);
            b[arr + 4 * i..arr + 4 * i + 4].copy_from_slice(&(base + at as u32).to_be_bytes());
        }
        ptr(&mut b, O_TOOL_TYPES, arr);
    }
    b
}

/// Put `icon` into guest memory; None = out of memory.
fn materialize<U: GuestUi>(a: &mut Amiga<U>, icon: &Icon) -> Result<Option<u32>, Refusal> {
    let size = build(icon, 0).len() as u32;
    let Some(p) = a.heap.alloc(size) else { return Ok(None) };
    let bytes = build(icon, p);
    a.mem.write_bytes(p, &bytes).map_err(Refusal::mem("DiskObject"))?;
    a.fs.icons.insert(p, size);
    Ok(Some(p))
}

/// Read a guest DiskObject back into the file model (PutDiskObject, DupDiskObject).
fn extract<U: GuestUi>(a: &Amiga<U>, p: u32) -> Result<Icon, Refusal> {
    let m = Refusal::mem("DiskObject");
    let mut head = [0u8; HEAD];
    for (i, h) in head.iter_mut().enumerate() { *h = a.mem.read_u8(p + i as u32).map_err(&m)?; }
    if u16::from_be_bytes([head[0], head[1]]) != MAGIC { return Err(Refusal(format!("DiskObject {p:#x}: do_Magic is not {MAGIC:#x}"))); }
    let at = |h: &[u8; HEAD], o: usize| u32::from_be_bytes([h[o], h[o + 1], h[o + 2], h[o + 3]]);
    let (g, s, d, tt, tw, dr) = (at(&head, O_GADGET_RENDER), at(&head, O_SELECT_RENDER), at(&head, O_DEFAULT_TOOL), at(&head, O_TOOL_TYPES), at(&head, O_TOOL_WINDOW), at(&head, O_DRAWER_DATA));
    for o in [O_GADGET_RENDER, O_SELECT_RENDER, 4 + 26, 4 + 34, O_DEFAULT_TOOL, O_TOOL_TYPES, O_DRAWER_DATA, O_TOOL_WINDOW, 4] { head[o..o + 4].fill(0); }
    let bytes = |a: &Amiga<U>, from: u32, n: usize| -> Result<Vec<u8>, Refusal> { (0..n as u32).map(|i| a.mem.read_u8(from + i).map_err(Refusal::mem("DiskObject"))).collect() };
    let image = |ptr: u32| -> Result<Option<Image>, Refusal> {
        if ptr == 0 { return Ok(None); }
        let mut hdr: [u8; IMAGE_HDR] = bytes(a, ptr, IMAGE_HDR)?.try_into().expect("20 bytes");
        let data_at = u32::from_be_bytes([hdr[10], hdr[11], hdr[12], hdr[13]]);
        hdr[10..14].fill(0);
        hdr[16..20].fill(0);
        let n = image_bytes(&hdr);
        Ok(Some(Image { data: if data_at == 0 { vec![0; n] } else { bytes(a, data_at, n)? }, hdr }))
    };
    let string = |ptr: u32| -> Result<Option<Vec<u8>>, Refusal> { if ptr == 0 { Ok(None) } else { read_bytes_z(&a.mem, ptr, MAX_STR).map(Some).map_err(Refusal::mem("DiskObject string")) } };
    let tool_types = if tt == 0 { None } else {
        let mut v = Vec::new();
        for i in 0..MAX_TOOL_TYPES {
            let sp = a.mem.read_u32(tt + 4 * i).map_err(Refusal::mem("do_ToolTypes"))?;
            if sp == 0 { break; }
            v.push(read_bytes_z(&a.mem, sp, MAX_STR).map_err(Refusal::mem("tool type"))?);
        }
        Some(v)
    };
    let (drawer, drawer_new) = if dr == 0 { (None, None) } else {
        let all = bytes(a, dr, DRAWER_MEM)?;
        (Some(all[..OLD_DRAWER].try_into().expect("56")), Some(all[OLD_DRAWER..].try_into().expect("6")))
    };
    Ok(Icon { head, drawer, drawer_new, images: [image(g)?, image(s)?], default_tool: string(d)?, tool_types, tool_window: string(tw)?, trailer: vec![] })
}

/// Load and parse `name.info`. Err = (IoErr, why) for the caller to report.
fn load<U: GuestUi>(a: &Amiga<U>, name: &str) -> Result<Icon, (DosErr, String)> {
    let path = info_path(name);
    let node = a.fs.vol.resolve(cur_dir(a), &path).map_err(|e| (e, format!("{path}: not found")))?;
    let data = a.fs.vol.file_data(node).ok_or((ERROR_OBJECT_WRONG_TYPE, format!("{path}: a directory")))?;
    Icon::parse(data).map_err(|e| (ERROR_OBJECT_WRONG_TYPE, format!("{path}: {e:?}")))
}

fn deliver<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, icon: &Icon, what: String) -> Result<String, Refusal> {
    let note = if icon.trailer.is_empty() { String::new() } else { format!(" ({} trailing bytes of GlowIcons/NewIcons data ignored)", icon.trailer.len()) };
    match materialize(a, icon)? {
        Some(p) => ok(a, cpu, p, format!("{what} -> {p:#x} type {}{note}", icon.do_type())),
        None => fail(a, cpu, ERROR_NO_FREE_STORE, 0, what),
    }
}

/// GetDiskObject(a0 = name) -> DiskObject or NULL.
fn get_disk_object<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let name = str_arg(a, cpu.areg(0), "GetDiskObject.name")?;
    let what = format!("[icon] GetDiskObject \"{name}\"");
    match load(a, &name) { Ok(i) => deliver(a, cpu, &i, what), Err((e, why)) => fail(a, cpu, e, 0, format!("{what} ({why})")) }
}

/// GetDiskObjectNew(a0 = name): like GetDiskObject, but a missing `.info` gives the default icon for what `name` is.
fn get_disk_object_new<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let name = str_arg(a, cpu.areg(0), "GetDiskObjectNew.name")?;
    let what = format!("[icon] GetDiskObjectNew \"{name}\"");
    match load(a, &name) {
        Ok(i) => deliver(a, cpu, &i, what),
        Err((e, why)) if e == ERROR_OBJECT_NOT_FOUND => {
            let Ok(node) = a.fs.vol.resolve(cur_dir(a), &name) else { return fail(a, cpu, e, 0, format!("{what} ({why}; and {name} itself does not exist)")) };
            let ty = match &a.fs.vol.get(node).kind {
                Kind::Dir(_) if a.fs.vol.is_device(node) => WBDISK,
                Kind::Dir(_) => WBDRAWER,
                Kind::File(d) if d.starts_with(&[0, 0, 3, 0xF3]) => WBTOOL, // HUNK_HEADER: an executable
                Kind::File(_) => WBPROJECT,
            };
            { let (i, note) = default_for(a, ty); deliver(a, cpu, &i, format!("{what} (no .info: default){note}")) }
        }
        Err((e, why)) => fail(a, cpu, e, 0, format!("{what} ({why})")),
    }
}

/// GetDefDiskObject(d0 = type) -> the engine's default icon of that type, or NULL for a type Workbench has none of.
fn get_def_disk_object<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let ty = cpu.dreg(0);
    let what = format!("[icon] GetDefDiskObject {ty}");
    if !(u32::from(WBDISK)..=u32::from(WBKICK)).contains(&ty) { return fail(a, cpu, ERROR_OBJECT_WRONG_TYPE, 0, what); }
    { let (i, note) = default_for(a, ty as u8); deliver(a, cpu, &i, format!("{what}{note}")) }
}

/// NewDiskObject(d0 = type) -> a fresh icon of that type (the default one: the engine has no blank-icon variant).
fn new_disk_object<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let ty = cpu.dreg(0);
    let what = format!("[icon] NewDiskObject {ty}");
    if !(u32::from(WBDISK)..=u32::from(WBAPPICON)).contains(&ty) { return fail(a, cpu, ERROR_OBJECT_WRONG_TYPE, 0, what); }
    { let (i, note) = default_for(a, ty as u8); deliver(a, cpu, &i, format!("{what}{note}")) }
}

/// DupDiskObjectA(a0 = icon, a1 = tags): an independent copy (the tag list is not interpreted; any tag is logged).
fn dup_disk_object<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let icon = extract(a, cpu.areg(0))?;
    if cpu.areg(1) != 0 { return Err(Refusal("DupDiskObjectA: tags (ICONDUPA_*) are not modelled".into())); }
    deliver(a, cpu, &icon, format!("[icon] DupDiskObjectA {:#x}", cpu.areg(0)))
}

/// FreeDiskObject(a0 = icon).
fn free_disk_object<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let p = cpu.areg(0);
    if p == 0 { return Ok("[icon] FreeDiskObject NULL".into()); }
    let Some(size) = a.fs.icons.remove(&p) else { return Err(Refusal(format!("FreeDiskObject {p:#x}: not a DiskObject icon.library made (or already freed)"))) };
    a.heap.free(p, size).map_err(|e| Refusal(format!("FreeDiskObject: {e}")))?;
    Ok(format!("[icon] FreeDiskObject {p:#x}"))
}

/// PutDiskObject(a0 = name, a1 = icon) -> TRUE: writes `name.info` on a writable device.
fn put_disk_object<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let name = str_arg(a, cpu.areg(0), "PutDiskObject.name")?;
    let path = info_path(&name);
    let what = format!("[icon] PutDiskObject \"{path}\"");
    let bytes = extract(a, cpu.areg(1))?.to_bytes();
    match write_file(a, &path, bytes) { Err(e) => fail(a, cpu, e, 0, what), Ok(_) => ok(a, cpu, TRUE, what) }
}

/// DeleteDiskObject(a0 = name) -> TRUE: removes `name.info`.
fn delete_disk_object<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let path = info_path(&str_arg(a, cpu.areg(0), "DeleteDiskObject.name")?);
    let what = format!("[icon] DeleteDiskObject \"{path}\"");
    let r = a.fs.vol.resolve(cur_dir(a), &path).and_then(|n| if a.fs.node_in_use(n) { Err(ERROR_OBJECT_IN_USE) } else { a.fs.vol.remove(n) });
    match r { Ok(()) => ok(a, cpu, TRUE, what), Err(e) => fail(a, cpu, e, 0, what) }
}

/// FindToolType(a0 = tool type array, a1 = name) -> the text after `name=` (or the empty tail of a bare `name`), NULL
/// if no entry is `name` (case-insensitively; `(name=x)`, the disabled form, is not `name`).
fn find_tool_type<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (arr, name) = (cpu.areg(0), read_bytes_z(&a.mem, cpu.areg(1), MAX_STR).map_err(Refusal::mem("FindToolType.name"))?);
    let mut found = 0;
    if arr != 0 {
        for i in 0..MAX_TOOL_TYPES {
            let sp = a.mem.read_u32(arr + 4 * i).map_err(Refusal::mem("tool type array"))?;
            if sp == 0 { break; }
            let e = read_bytes_z(&a.mem, sp, MAX_STR).map_err(Refusal::mem("tool type"))?;
            let key = e.split(|&c| c == b'=').next().unwrap_or(&e);
            if key.eq_ignore_ascii_case(&name) { found = sp + if e.len() > name.len() { name.len() as u32 + 1 } else { name.len() as u32 }; break; }
        }
    }
    cpu.set_d0(found);
    Ok(format!("[icon] FindToolType \"{}\" -> {found:#x}", String::from_utf8_lossy(&name)))
}

/// MatchToolValue(a0 = "A|B|C", a1 = value) -> TRUE if value is one of the alternatives (case-insensitive, whole word);
/// a value containing `|` never matches.
fn match_tool_value<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let list = read_bytes_z(&a.mem, cpu.areg(0), MAX_STR).map_err(Refusal::mem("MatchToolValue.typeString"))?;
    let value = read_bytes_z(&a.mem, cpu.areg(1), MAX_STR).map_err(Refusal::mem("MatchToolValue.value"))?;
    let hit = !value.contains(&b'|') && list.split(|&c| c == b'|').any(|alt| alt.eq_ignore_ascii_case(&value));
    cpu.set_d0(if hit { TRUE } else { 0 });
    Ok(format!("[icon] MatchToolValue \"{}\" in \"{}\" -> {hit}", String::from_utf8_lossy(&value), String::from_utf8_lossy(&list)))
}
