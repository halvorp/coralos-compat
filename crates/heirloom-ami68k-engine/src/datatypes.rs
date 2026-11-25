//! AmigaOS `datatypes.library` (v43) bridged directly to CoralOS's native `datatype.library`
//! (`heirloom_common::datatype::NativeDatatype`) with built-in IFF-ILBM / BMP fallback.
//!
//! Heavy Amiga viewers & apps (MultiView, PicShow, Visage, AWeb, DOpus 5, MUI DTImg) call
//! `ObtainDataTypeA`, `NewDTObjectA`, `GetDTAttrsA`, `SetDTAttrsA`, `DoDTMethodA`
//! (`DTM_PROCLAYOUT`, `PDTM_READPIXELARRAY`), `AddDTObject`/`RefreshDTObjectA`/`RemoveDTObject`,
//! and `ObtainDTDrawInfoA`/`DrawDTObjectA`/`ReleaseDTDrawInfo`. Every supported image format
//! (PNG, JPEG, GIF, WebP, SVG) decodes natively via CoralOS's out-of-process `datatype.library`
//! workers rather than emulating 68k decoders.
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_common::datatype::{
    decode_with_fallback, identify_magic, DecodedImage, DT_BMP, DT_GIF, DT_IFF_ILBM, DT_JPEG,
    DT_PDF, DT_PNG, DT_SVG, DT_UNKNOWN, DT_WEBP,
};
use heirloom_raster::Rect;
use heirloom_ui_abi::{FontRole, GuestUi};

use crate::cpu::Cpu;
use crate::graphics::layout::*;
use crate::graphics::record::Prim;
use crate::graphics::target;
use crate::libtable::LibraryBuilder;
use crate::machine::Amiga;
use crate::{fs, tags, Refusal};

// Source types (datatypes/datatypes.h)
const DTST_RAM: u32 = 1;
const DTST_FILE: u32 = 2;
const DTST_MEMORY: u32 = 5;

// Group IDs (datatypes/datatypes.h — 4-char ASCII tags)
pub const GID_TEXT: u32 = u32::from_be_bytes(*b"text");
pub const GID_DOCUMENT: u32 = u32::from_be_bytes(*b"docu");
pub const GID_SOUND: u32 = u32::from_be_bytes(*b"soun");
pub const GID_PICTURE: u32 = u32::from_be_bytes(*b"pict");

// DTA_* tags (datatypes/datatypesclass.h, DTA_Dummy = TAG_USER + 0x1000 = 0x8000_1000)
const DTA_DUMMY: u32 = tags::TAG_USER + 0x1000;
const DTA_TEXT_ATTR: u32 = DTA_DUMMY + 10;
const DTA_TOP_VERT: u32 = DTA_DUMMY + 11;
const DTA_VISIBLE_VERT: u32 = DTA_DUMMY + 12;
const DTA_TOTAL_VERT: u32 = DTA_DUMMY + 13;
const DTA_VERT_UNIT: u32 = DTA_DUMMY + 14;
const DTA_TOP_HORIZ: u32 = DTA_DUMMY + 15;
const DTA_VISIBLE_HORIZ: u32 = DTA_DUMMY + 16;
const DTA_TOTAL_HORIZ: u32 = DTA_DUMMY + 17;
const DTA_HORIZ_UNIT: u32 = DTA_DUMMY + 18;
const DTA_NOMINAL_HORIZ: u32 = DTA_DUMMY + 26;
const DTA_NOMINAL_VERT: u32 = DTA_DUMMY + 27;
const DTA_OBJ_NAME: u32 = DTA_DUMMY + 28;
const DTA_DATA_TYPE: u32 = DTA_DUMMY + 41;
const DTA_SOURCE_TYPE: u32 = DTA_DUMMY + 100;
const DTA_HANDLE: u32 = DTA_DUMMY + 101;
const DTA_NAME: u32 = DTA_DUMMY + 103;
const DTA_GROUP_ID: u32 = DTA_DUMMY + 104;
const DTA_SOURCE_ADDRESS: u32 = DTA_DUMMY + 110;
const DTA_SOURCE_SIZE: u32 = DTA_DUMMY + 111;

// PDTA_* tags (datatypes/pictureclass.h, PDTA_ModeID = DTA_Dummy + 200 = 0x8000_10C8)
const PDTA_MODE_ID: u32 = DTA_DUMMY + 200;
const PDTA_BITMAP_HEADER: u32 = DTA_DUMMY + 201;
const PDTA_BITMAP: u32 = DTA_DUMMY + 202;
const PDTA_COLOR_REGISTERS: u32 = DTA_DUMMY + 203;
const PDTA_CREGS: u32 = DTA_DUMMY + 204;
const PDTA_NUM_COLORS: u32 = DTA_DUMMY + 209;
const PDTA_REMAP: u32 = DTA_DUMMY + 211;
const PDTA_SCREEN: u32 = DTA_DUMMY + 212;
const PDTA_DEST_BITMAP: u32 = DTA_DUMMY + 215;
const PDTA_CLASS_BITMAP: u32 = DTA_DUMMY + 216;
const PDTA_DEST_MODE: u32 = DTA_DUMMY + 251;
const PDTA_ALPHA_CHANNEL: u32 = DTA_DUMMY + 258;

// TDTA_* tags (datatypes/textclass.h, TDTA_Dummy = DTA_Dummy + 300)
const TDTA_DUMMY: u32 = DTA_DUMMY + 300;
const TDTA_BUFFER: u32 = TDTA_DUMMY + 1;
const TDTA_BUFFER_LEN: u32 = TDTA_DUMMY + 2;

// Methods (datatypes/datatypesclass.h & pictureclass.h)
const DTM_DUMMY: u32 = 0x600;
const DTM_FRAMEBOX: u32 = DTM_DUMMY + 1;
const DTM_PROCLAYOUT: u32 = DTM_DUMMY + 2;
const PDTM_DUMMY: u32 = DTM_DUMMY + 0x60;
const PDTM_WRITEPIXELARRAY: u32 = PDTM_DUMMY + 0;
const PDTM_READPIXELARRAY: u32 = PDTM_DUMMY + 1;

// Pixel array formats (pictureclass.h)
const PBPAFMT_RGB: u32 = 0;
const PBPAFMT_RGBA: u32 = 1;
const PBPAFMT_ARGB: u32 = 2;
const PBPAFMT_LUT8: u32 = 3;
const PBPAFMT_GREY8: u32 = 4;

// GA_* gadget bounds tags for AddDTObject/SetDTAttrsA
const GA_LEFT: u32 = 0x8003_0001;
const GA_TOP: u32 = 0x8003_0003;
const GA_WIDTH: u32 = 0x8003_0005;
const GA_HEIGHT: u32 = 0x8003_0007;

#[derive(Clone)]
pub struct DtDescriptor {
    pub guest_addr: u32,
    pub header_addr: u32,
    pub name_addr: u32,
    pub name_len: u32,
    pub base_addr: u32,
    pub base_len: u32,
    pub group_id: u32,
    pub format_id: u32,
    pub name: String,
    pub base_name: String,
}

#[derive(Clone)]
pub struct DtObject {
    pub guest_addr: u32,
    pub desc_addr: u32,
    pub name_addr: u32,
    pub name_len: u32,
    pub group_id: u32,
    pub name: String,
    pub image: Option<DecodedImage>,
    pub text_bytes: Vec<u8>,
    pub text_buf_addr: u32,
    pub text_buf_len: u32,
    pub bmhd_addr: u32,
    pub bitmap_addr: u32,
    pub cregs_addr: u32,
    pub color_regs_addr: u32,
    pub num_colors: u32,
    pub rect: (i32, i32, i32, i32), // (left, top, w, h)
    pub window: Option<u32>,
    pub native_decoded: bool,
}

#[derive(Default)]
pub struct State {
    pub descriptors: BTreeMap<u32, DtDescriptor>,
    pub objects: BTreeMap<u32, DtObject>,
    pub draw_infos: BTreeMap<u32, u32>, // handle -> obj_addr
    pub native_decodes: u64,
}

impl State {
    /// Truecolor decoded image owned by `bitmap_addr` (from `PDTA_BitMap` / `PDTA_DestBitMap`), if any.
    pub fn image_for_bitmap(&self, bm: u32) -> Option<&DecodedImage> {
        if bm == 0 {
            return None;
        }
        self.objects.values().find_map(|o| if o.bitmap_addr == bm { o.image.as_ref() } else { None })
    }
}

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.named("ObtainDataTypeA", obtain_data_type::<U>)
        .named("ReleaseDataType", release_data_type::<U>)
        .named("LockDataType", lock_data_type::<U>)
        .named("NewDTObjectA", new_dt_object::<U>)
        .named("DisposeDTObject", dispose_dt_object::<U>)
        .named("SetDTAttrsA", set_dt_attrs::<U>)
        .named("GetDTAttrsA", get_dt_attrs::<U>)
        .named("AddDTObject", add_dt_object::<U>)
        .named("RefreshDTObjectA", refresh_dt_object::<U>)
        .named("RemoveDTObject", remove_dt_object::<U>)
        .named("DoAsyncLayout", do_async_layout::<U>)
        .named("DoDTMethodA", do_dt_method::<U>)
        .named("GetDTMethods", get_dt_methods::<U>)
        .named("GetDTTriggerMethods", get_dt_trigger_methods::<U>)
        .named("ObtainDTDrawInfoA", obtain_dt_draw_info::<U>)
        .named("DrawDTObjectA", draw_dt_object::<U>)
        .named("ReleaseDTDrawInfo", release_dt_draw_info::<U>)
        .named("GetDTString", get_dt_string::<U>)
        .named("FindMethod", find_method::<U>)
        .named("FindTriggerMethod", find_trigger_method::<U>)
        .named("CopyDTMethods", copy_dt_methods::<U>)
        .named("FreeDTMethods", free_dt_methods::<U>)
        .named("DoDTDomainA", do_dt_domain::<U>)
}

fn format_meta(kind: u32) -> (u32, u32, &'static str, &'static str) {
    match kind {
        DT_PNG => (GID_PICTURE, u32::from_be_bytes(*b"png "), "PNG", "png"),
        DT_JPEG => (GID_PICTURE, u32::from_be_bytes(*b"jpeg"), "JPEG", "jpeg"),
        DT_GIF => (GID_PICTURE, u32::from_be_bytes(*b"gif "), "GIF", "gif"),
        DT_WEBP => (GID_PICTURE, u32::from_be_bytes(*b"webp"), "WebP", "webp"),
        DT_SVG => (GID_PICTURE, u32::from_be_bytes(*b"svg "), "SVG", "svg"),
        DT_BMP => (GID_PICTURE, u32::from_be_bytes(*b"bmp "), "BMP", "bmp"),
        DT_IFF_ILBM => (GID_PICTURE, u32::from_be_bytes(*b"ilbm"), "ILBM", "ilbm"),
        DT_PDF => (GID_DOCUMENT, u32::from_be_bytes(*b"pdf "), "PDF", "pdf"),
        _ => (GID_TEXT, u32::from_be_bytes(*b"asci"), "ASCII", "ascii"),
    }
}

fn identify_bytes<U: GuestUi>(a: &Amiga<U>, bytes: &[u8]) -> u32 {
    if let Some(h) = a.datatype_host.as_deref() {
        let k = h.identify(bytes);
        if k != DT_UNKNOWN {
            return k;
        }
    }
    identify_magic(bytes)
}

fn alloc_cstr<U: GuestUi>(a: &mut Amiga<U>, s: &str) -> Result<(u32, u32), Refusal> {
    let mut bytes = s.as_bytes().to_vec();
    bytes.push(0);
    let len = bytes.len() as u32;
    let p = a.heap.alloc(len).ok_or_else(|| Refusal("datatypes: guest heap full".into()))?;
    a.mem.write_bytes(p, &bytes).map_err(Refusal::mem("datatypes cstr"))?;
    Ok((p, len))
}

/// Allocate a guest `struct DataType` (sizeof ~64) + `struct DataTypeHeader` (sizeof 44).
fn make_descriptor<U: GuestUi>(a: &mut Amiga<U>, kind: u32) -> Result<u32, Refusal> {
    let (group_id, format_id, name, base_name) = format_meta(kind);
    let (name_p, name_len) = alloc_cstr(a, name)?;
    let (base_p, base_len) = alloc_cstr(a, base_name)?;
    let hdr = a.heap.alloc(44).ok_or_else(|| Refusal("datatypes: guest heap full (DataTypeHeader)".into()))?;
    let dt = a.heap.alloc(64).ok_or_else(|| Refusal("datatypes: guest heap full (DataType)".into()))?;
    let m = Refusal::mem("DataTypeHeader");
    a.mem.write_bytes(hdr, &[0u8; 44]).map_err(&m)?;
    // struct DataTypeHeader: 0: dth_Name, 4: dth_BaseName, 8: dth_Pattern, 12: dth_Mask,
    // 16: dth_GroupID, 20: dth_ID, 24: dth_MaskLen, 26: dth_Pad, 28: dth_Flags, 30: dth_Priority
    a.mem.write_u32(hdr + 0, name_p).map_err(&m)?;
    a.mem.write_u32(hdr + 4, base_p).map_err(&m)?;
    a.mem.write_u32(hdr + 16, group_id).map_err(&m)?;
    a.mem.write_u32(hdr + 20, format_id).map_err(&m)?;

    // struct DataType: dtn_Node1 (0..14), dtn_Node2 (14..28), dtn_Header (28..32), dtn_ToolList (32..46)
    a.mem.write_bytes(dt, &[0u8; 64]).map_err(&m)?;
    a.mem.write_u32(dt + 10, name_p).map_err(&m)?; // dtn_Node1.ln_Name
    a.mem.write_u32(dt + 28, hdr).map_err(&m)?; // dtn_Header
    a.datatypes.descriptors.insert(
        dt,
        DtDescriptor {
            guest_addr: dt,
            header_addr: hdr,
            name_addr: name_p,
            name_len,
            base_addr: base_p,
            base_len,
            group_id,
            format_id,
            name: String::from(name),
            base_name: String::from(base_name),
        },
    );
    Ok(dt)
}

fn free_descriptor<U: GuestUi>(a: &mut Amiga<U>, dt: u32) {
    if let Some(rec) = a.datatypes.descriptors.remove(&dt) {
        let _ = a.heap.free(rec.name_addr, rec.name_len);
        let _ = a.heap.free(rec.base_addr, rec.base_len);
        let _ = a.heap.free(rec.header_addr, 44);
        let _ = a.heap.free(rec.guest_addr, 64);
    }
}

fn free_dt_object<U: GuestUi>(a: &mut Amiga<U>, obj_addr: u32) {
    if let Some(obj) = a.datatypes.objects.remove(&obj_addr) {
        if obj.bmhd_addr != 0 { let _ = a.heap.free(obj.bmhd_addr, 24); }
        if obj.cregs_addr != 0 { let _ = a.heap.free(obj.cregs_addr, 256 * 12); }
        if obj.color_regs_addr != 0 { let _ = a.heap.free(obj.color_regs_addr, 256 * 3); }
        if obj.text_buf_addr != 0 && obj.text_buf_len != 0 { let _ = a.heap.free(obj.text_buf_addr, obj.text_buf_len); }
        if obj.name_addr != 0 && obj.name_len != 0 { let _ = a.heap.free(obj.name_addr, obj.name_len); }
        if obj.desc_addr != 0 { free_descriptor(a, obj.desc_addr); }
    }
    if let Some(rec) = a.classes.objs.remove(&obj_addr) {
        let _ = crate::iclass::count_object(a, rec.class, false);
        let _ = a.heap.free(rec.block, rec.size);
    }
}

fn read_node_bytes<U: GuestUi>(a: &Amiga<U>, id: crate::volume::NodeId) -> Option<(String, Vec<u8>)> {
    let buf = a.fs.vol.read_node_all(id, 32 * 1024 * 1024)?;
    Some((a.fs.vol.full_path(id), buf))
}

fn read_source_bytes<U: GuestUi>(a: &mut Amiga<U>, source_type: u32, handle: u32, name: Option<&str>, mem_addr: u32, mem_size: u32) -> Option<(String, Vec<u8>)> {
    if source_type == DTST_MEMORY && mem_addr != 0 && mem_size > 0 {
        let cap = (mem_size as usize).min(32 * 1024 * 1024) as u32;
        if let Ok(slice) = a.mem.bytes(heirloom_mem_flat32::Span { addr: mem_addr, len: cap }) {
            return Some((String::from(name.unwrap_or("memory")), slice.to_vec()));
        }
        return None;
    }
    if source_type == DTST_RAM {
        return Some((String::from(name.unwrap_or("ram")), Vec::new()));
    }
    // DTST_FILE: handle may be a BPTR Lock (handle << 2) or a file path string in `name`.
    if handle != 0 {
        if let Some(lock) = a.fs.locks.get(&(handle << 2)) {
            let node = lock.node;
            if let Some(res) = read_node_bytes(a, node) {
                return Some(res);
            }
        }
        if let Some(lock) = a.fs.locks.get(&handle) {
            let node = lock.node;
            if let Some(res) = read_node_bytes(a, node) {
                return Some(res);
            }
        }
    }
    let path = name?;
    if path.is_empty() {
        return None;
    }
    let cwd = fs::cur_dir(a);
    let node = a.fs.vol.resolve(cwd, path).ok()?;
    read_node_bytes(a, node)
}

/// ObtainDataTypeA(d0 = type, a0 = handle, a1 = attrs) -> struct DataType *
fn obtain_data_type<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let source_type = cpu.dreg(0);
    let handle = cpu.areg(0);
    let tag_ptr = cpu.areg(1);
    let mut mem_addr = 0u32;
    let mut mem_size = 0u32;
    if tag_ptr != 0 {
        for (tag, val) in tags::walk(&a.mem, tag_ptr)? {
            match tag {
                DTA_SOURCE_ADDRESS => mem_addr = val,
                DTA_SOURCE_SIZE => mem_size = val,
                _ => {}
            }
        }
    }
    let bytes = if source_type == DTST_FILE {
        // For ObtainDataTypeA(DTST_FILE, lock_bptr, tags), handle is a BPTR Lock; if it's a string pointer fallback, try it too.
        read_source_bytes(a, DTST_FILE, handle, None, 0, 0).or_else(|| {
            a.mem.read_cstr(handle).ok().and_then(|s| read_source_bytes(a, DTST_FILE, 0, Some(&s), 0, 0))
        })
    } else {
        read_source_bytes(a, source_type, handle, None, mem_addr, mem_size)
    };
    let Some((label, data)) = bytes else {
        cpu.set_d0(0);
        return Ok(format!("[datatypes] ObtainDataTypeA type={source_type} handle={handle:#x} -> NULL (not found)"));
    };
    let kind = identify_bytes(a, &data);
    let dt = make_descriptor(a, kind)?;
    cpu.set_d0(dt);
    let name = a.datatypes.descriptors[&dt].name.clone();
    Ok(format!("[datatypes] ObtainDataTypeA {label:?} -> {dt:#x} ({name})"))
}

fn release_data_type<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let dt = cpu.areg(0);
    free_descriptor(a, dt);
    Ok(format!("[datatypes] ReleaseDataType {dt:#x}"))
}

fn lock_data_type<U: GuestUi>(_a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let dt = cpu.areg(0);
    Ok(format!("[datatypes] LockDataType {dt:#x}"))
}

/// Quantize `0xAARRGGBB` pixel into the current screen's 256-entry palette (for 8-bit BitMap fallback).
fn nearest_pen(rgb: u32, pal: &[u32; 256]) -> u8 {
    let r = ((rgb >> 16) & 0xFF) as i32;
    let g = ((rgb >> 8) & 0xFF) as i32;
    let b = (rgb & 0xFF) as i32;
    let mut best = 0u8;
    let mut best_d = i32::MAX;
    for (i, &c) in pal.iter().enumerate() {
        let dr = r - ((c >> 16) & 0xFF) as i32;
        let dg = g - ((c >> 8) & 0xFF) as i32;
        let db = b - (c & 0xFF) as i32;
        let d = dr * dr + dg * dg + db * db;
        if d < best_d {
            best_d = d;
            best = i as u8;
            if d == 0 {
                break;
            }
        }
    }
    best
}

/// NewDTObjectA(d0 = name, a0 = attrs) -> Object *
fn new_dt_object<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let name_arg = cpu.dreg(0);
    let tag_ptr = cpu.areg(0);
    let mut source_type = DTST_FILE;
    let mut handle = 0u32;
    let mut tag_name: Option<String> = None;
    let mut mem_addr = 0u32;
    let mut mem_size = 0u32;
    let mut want_group = 0u32;
    let mut rect = (0i32, 0i32, 0i32, 0i32);

    if tag_ptr != 0 {
        for (tag, val) in tags::walk(&a.mem, tag_ptr)? {
            match tag {
                DTA_SOURCE_TYPE => source_type = val,
                DTA_HANDLE => handle = val,
                DTA_NAME if val != 0 => tag_name = a.mem.read_cstr(val).ok(),
                DTA_SOURCE_ADDRESS => mem_addr = val,
                DTA_SOURCE_SIZE => mem_size = val,
                DTA_GROUP_ID => want_group = val,
                GA_LEFT => rect.0 = val as i32,
                GA_TOP => rect.1 = val as i32,
                GA_WIDTH => rect.2 = val as i32,
                GA_HEIGHT => rect.3 = val as i32,
                _ => {}
            }
        }
    }
    let name = if name_arg != 0 {
        a.mem.read_cstr(name_arg).ok().or(tag_name)
    } else {
        tag_name
    };
    let Some((label, raw)) = read_source_bytes(a, source_type, handle, name.as_deref(), mem_addr, mem_size) else {
        cpu.set_d0(0);
        return Ok(format!("[datatypes] NewDTObjectA {:?} -> NULL (file not found)", name));
    };

    let kind = identify_bytes(a, &raw);
    let (group_id, _, fmt_name, _) = format_meta(kind);
    if want_group != 0 && want_group != group_id && !(want_group == GID_PICTURE && group_id == GID_PICTURE) {
        cpu.set_d0(0);
        return Ok(format!("[datatypes] NewDTObjectA {label:?} -> NULL (group mismatch: got {fmt_name})"));
    }

    let mut decoded_img: Option<DecodedImage> = None;
    let mut native_decoded = false;
    if group_id == GID_PICTURE {
        if let Some(h) = a.datatype_host.as_deref() {
            if let Some(img) = h.decode_image(&raw, 0, 0) {
                a.datatypes.native_decodes += 1;
                native_decoded = true;
                decoded_img = Some(img);
            }
        }
        if decoded_img.is_none() {
            decoded_img = decode_with_fallback(None, &raw, 0, 0);
        }
        if decoded_img.is_none() {
            cpu.set_d0(0);
            return Ok(format!("[datatypes] NewDTObjectA {label:?} ({fmt_name}) -> NULL (decode failed)"));
        }
    }

    let desc_addr = make_descriptor(a, kind)?;
    let (name_addr, name_len) = alloc_cstr(a, &label)?;
    // Allocate as a real BOOPSI `datatypeclass` object so inline `DoMethod(obj, ...)` / `GetAttr` /
    // `SetAttrs` / `DisposeObject` find `OCLASS(obj)` at `obj - 4` and dispatch cleanly into `dispatch_boopsi`.
    let dt_class = crate::iclass::native_addr(crate::iclass::Native::Datatype);
    let obj_size = a.classes.recs.get(&dt_class).map(|r| r.object_size()).unwrap_or(80).max(80);
    let block = crate::memory::alloc_zeroed(a, obj_size, "DTObject")?;
    a.mem.write_u32(block + crate::boopsi_ids::OBJ_CLASS, dt_class).map_err(Refusal::mem("DTObject.o_Class"))?;
    let _ = crate::iclass::count_object(a, dt_class, true);
    let obj_addr = block + crate::boopsi_ids::OBJ_HEADER;
    a.classes.objs.insert(obj_addr, crate::iclass::ObjRec { class: dt_class, block, size: obj_size });

    let mut bmhd_addr = 0u32;
    let mut bitmap_addr = 0u32;
    let mut cregs_addr = 0u32;
    let mut color_regs_addr = 0u32;
    let mut num_colors = 0u32;
    let mut text_buf_addr = 0u32;
    let mut text_buf_len = 0u32;

    if let Some(ref img) = decoded_img {
        let (w, h) = (img.width as i32, img.height as i32);
        if rect.2 == 0 {
            rect.2 = w;
        }
        if rect.3 == 0 {
            rect.3 = h;
        }
        // struct BitMapHeader (datatypes/pictureclass.h, 20 bytes):
        // 0: Width (u16), 2: Height (u16), 4: Left (i16), 6: Top (i16), 8: Depth (u8),
        // 9: Masking (u8), 10: Compression (u8), 11: Pad (u8), 12: Transparent (u16),
        // 14: XAspect (u8), 15: YAspect (u8), 16: PageWidth (i16), 18: PageHeight (i16)
        bmhd_addr = a.heap.alloc(24).ok_or_else(|| Refusal("datatypes: guest heap full (BitMapHeader)".into()))?;
        let mut bmhd = [0u8; 24];
        bmhd[0..2].copy_from_slice(&(img.width.min(u16::MAX as u32) as u16).to_be_bytes());
        bmhd[2..4].copy_from_slice(&(img.height.min(u16::MAX as u32) as u16).to_be_bytes());
        bmhd[8] = 24; // truecolor depth
        bmhd[14] = 1;
        bmhd[15] = 1;
        bmhd[16..18].copy_from_slice(&(img.width.min(i16::MAX as u32) as i16).to_be_bytes());
        bmhd[18..20].copy_from_slice(&(img.height.min(i16::MAX as u32) as i16).to_be_bytes());
        a.mem.write_bytes(bmhd_addr, &bmhd).map_err(Refusal::mem("BitMapHeader"))?;

        // Build a matching Amiga BitMap (8-bit quantized to screen palette) so legacy BltBitMap callers work too!
        bitmap_addr = crate::graphics::bitmaps::create(a, w, h, 8, None)?;
        let pal = a.gfx.palette().rgb;
        if let Some(gb) = a.gfx.bitmaps.get_mut(&bitmap_addr) {
            for y in 0..h {
                for x in 0..w {
                    let idx = (y as usize) * (img.width as usize) + (x as usize);
                    if let Some(&px) = img.pixels.get(idx) {
                        gb.bm.set(x, y, u32::from(nearest_pen(px, &pal)));
                    }
                }
            }
        }
        let _ = crate::graphics::bitmaps::export_planes(a, bitmap_addr);

        // Provide a 256-entry CRegs (3 * u32 per color) + ColorRegister (3 * u8 per color) table
        num_colors = 256;
        cregs_addr = a.heap.alloc(256 * 12).ok_or_else(|| Refusal("datatypes: guest heap full (CRegs)".into()))?;
        color_regs_addr = a.heap.alloc(256 * 3).ok_or_else(|| Refusal("datatypes: guest heap full (ColorRegs)".into()))?;
        let mut cregs = Vec::with_capacity(256 * 12);
        let mut colregs = Vec::with_capacity(256 * 3);
        for &c in &pal {
            let r = ((c >> 16) & 0xFF) as u8;
            let g = ((c >> 8) & 0xFF) as u8;
            let b = (c & 0xFF) as u8;
            cregs.extend_from_slice(&((r as u32) * 0x0101_0101).to_be_bytes());
            cregs.extend_from_slice(&((g as u32) * 0x0101_0101).to_be_bytes());
            cregs.extend_from_slice(&((b as u32) * 0x0101_0101).to_be_bytes());
            colregs.extend_from_slice(&[r, g, b]);
        }
        a.mem.write_bytes(cregs_addr, &cregs).map_err(Refusal::mem("CRegs"))?;
        a.mem.write_bytes(color_regs_addr, &colregs).map_err(Refusal::mem("ColorRegs"))?;
    } else if group_id == GID_TEXT {
        let len = raw.len().min(1024 * 1024);
        text_buf_len = (len + 1) as u32;
        text_buf_addr = a.heap.alloc(text_buf_len).ok_or_else(|| Refusal("datatypes: guest heap full (text)".into()))?;
        a.mem.write_bytes(text_buf_addr, &raw[..len]).map_err(Refusal::mem("TDTA_Buffer"))?;
        a.mem.write_u8(text_buf_addr + len as u32, 0).map_err(Refusal::mem("TDTA_Buffer"))?;
        let lines = raw[..len].iter().filter(|&&b| b == b'\n').count().max(1) as i32;
        if rect.2 == 0 {
            rect.2 = 600;
        }
        if rect.3 == 0 {
            rect.3 = (lines * 10).clamp(160, 400);
        }
    }

    // Write Gadget bounds into the guest object header (Gadget.LeftEdge=4, TopEdge=6, Width=8, Height=10)
    a.mem.write_u16(obj_addr + 4, rect.0 as i16 as u16).map_err(Refusal::mem("DTObject"))?;
    a.mem.write_u16(obj_addr + 6, rect.1 as i16 as u16).map_err(Refusal::mem("DTObject"))?;
    a.mem.write_u16(obj_addr + 8, rect.2 as i16 as u16).map_err(Refusal::mem("DTObject"))?;
    a.mem.write_u16(obj_addr + 10, rect.3 as i16 as u16).map_err(Refusal::mem("DTObject"))?;

    let dims = decoded_img.as_ref().map(|i| (i.width, i.height));
    a.datatypes.objects.insert(
        obj_addr,
        DtObject {
            guest_addr: obj_addr,
            desc_addr,
            name_addr,
            name_len,
            group_id,
            name: label.clone(),
            image: decoded_img,
            text_bytes: if group_id == GID_TEXT { raw } else { Vec::new() },
            text_buf_addr,
            text_buf_len,
            bmhd_addr,
            bitmap_addr,
            cregs_addr,
            color_regs_addr,
            num_colors,
            rect,
            window: None,
            native_decoded,
        },
    );
    cpu.set_d0(obj_addr);
    let via = if native_decoded { "native-coral-datatype" } else { "builtin-fallback" };
    Ok(format!("[datatypes] NewDTObjectA {label:?} ({fmt_name}, {via}, dims={dims:?}) -> {obj_addr:#x}"))
}

fn dispose_dt_object<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let obj = cpu.areg(0);
    free_dt_object(a, obj);
    Ok(format!("[datatypes] DisposeDTObject {obj:#x}"))
}

fn set_dt_attrs<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let obj_addr = cpu.areg(0);
    let tag_ptr = cpu.areg(3);
    if tag_ptr != 0 {
        let pairs = tags::walk(&a.mem, tag_ptr)?;
        if let Some(obj) = a.datatypes.objects.get_mut(&obj_addr) {
            for (tag, val) in pairs {
                match tag {
                    GA_LEFT => obj.rect.0 = val as i32,
                    GA_TOP => obj.rect.1 = val as i32,
                    GA_WIDTH => obj.rect.2 = val as i32,
                    GA_HEIGHT => obj.rect.3 = val as i32,
                    _ => {}
                }
            }
        }
    }
    cpu.set_d0(0);
    Ok(format!("[datatypes] SetDTAttrsA {obj_addr:#x}"))
}

/// GetDTAttrsA(a0 = obj, a2 = attrs) -> count of tags filled
fn get_dt_attrs<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let obj_addr = cpu.areg(0);
    let tag_ptr = cpu.areg(2);
    let Some(obj) = a.datatypes.objects.get(&obj_addr).cloned() else {
        cpu.set_d0(0);
        return Ok(format!("[datatypes] GetDTAttrsA {obj_addr:#x} -> 0 (unknown object)"));
    };
    let mut filled = 0u32;
    if tag_ptr != 0 {
        for (tag, storage_ptr) in tags::walk(&a.mem, tag_ptr)? {
            if storage_ptr == 0 {
                continue;
            }
            let val = match tag {
                DTA_OBJ_NAME | DTA_NAME => Some(obj.name_addr),
                DTA_DATA_TYPE => Some(obj.desc_addr),
                DTA_GROUP_ID => Some(obj.group_id),
                DTA_NOMINAL_HORIZ | DTA_TOTAL_HORIZ | DTA_VISIBLE_HORIZ => Some(obj.rect.2.max(1) as u32),
                DTA_NOMINAL_VERT | DTA_TOTAL_VERT | DTA_VISIBLE_VERT => Some(obj.rect.3.max(1) as u32),
                DTA_TOP_HORIZ | DTA_TOP_VERT => Some(0),
                DTA_HORIZ_UNIT | DTA_VERT_UNIT => Some(1),
                PDTA_BITMAP_HEADER => Some(obj.bmhd_addr),
                PDTA_BITMAP | PDTA_DEST_BITMAP | PDTA_CLASS_BITMAP => Some(obj.bitmap_addr),
                PDTA_CREGS => Some(obj.cregs_addr),
                PDTA_COLOR_REGISTERS => Some(obj.color_regs_addr),
                PDTA_NUM_COLORS => Some(obj.num_colors),
                PDTA_MODE_ID => Some(crate::asl::DEFAULT_RTG_DISPLAY_ID),
                PDTA_ALPHA_CHANNEL => Some(0),
                TDTA_BUFFER => Some(obj.text_buf_addr),
                TDTA_BUFFER_LEN => Some(obj.text_bytes.len() as u32),
                PDTA_REMAP | PDTA_DEST_MODE | PDTA_SCREEN | DTA_TEXT_ATTR => Some(0),
                _ => None,
            };
            if let Some(v) = val {
                a.mem.write_u32(storage_ptr, v).map_err(Refusal::mem("GetDTAttrsA"))?;
                filled += 1;
            }
        }
    }
    cpu.set_d0(filled);
    Ok(format!("[datatypes] GetDTAttrsA {obj_addr:#x} -> {filled} attrs"))
}

/// Blit a picture or text DTObject onto a RastPort at `(dst_x, dst_y, w, h)` starting from `(src_x, src_y)`.
fn render_dt_to_rp<U: GuestUi>(a: &mut Amiga<U>, obj: &DtObject, rp: u32, dst_x: i32, dst_y: i32, w: i32, h: i32, src_x: i32, src_y: i32) -> Result<(), Refusal> {
    if let Some(ref img) = obj.image {
        let cw = w.min(img.width as i32 - src_x).max(0) as usize;
        let ch = h.min(img.height as i32 - src_y).max(0) as usize;
        if cw == 0 || ch == 0 {
            return Ok(());
        }
        let mut rgb = Vec::with_capacity(cw * ch);
        let pal = a.gfx.palette().rgb;
        let mut pens = Vec::with_capacity(cw * ch);
        for row in 0..ch {
            let sy = (src_y as usize) + row;
            for col in 0..cw {
                let sx = (src_x as usize) + col;
                let px = img.pixels[sy * (img.width as usize) + sx];
                rgb.push(px & 0x00FF_FFFF);
                pens.push(nearest_pen(px, &pal));
            }
        }
        let draw_rect = Rect::from_size(dst_x, dst_y, cw as i32, ch as i32);
        let prim = Prim::Pixels { rect: draw_rect, rgb };
        let _ = target::paint_as(a, rp, "DrawDTObjectA", Some(prim), |bm, clip, (ox, oy)| {
            let mut dirty = Rect::default();
            for row in 0..ch {
                for col in 0..cw {
                    let bx = dst_x + col as i32 - ox;
                    let by = dst_y + row as i32 - oy;
                    if clip.allows(bx, by) {
                        bm.set(bx, by, u32::from(pens[row * cw + col]));
                        dirty = dirty.union(&Rect::from_size(bx, by, 1, 1));
                    }
                }
            }
            dirty
        })?;
    } else if obj.group_id == GID_TEXT && !obj.text_bytes.is_empty() {
        let text = String::from_utf8_lossy(&obj.text_bytes);
        let mut y = dst_y + 2;
        for line in text.lines().take(((h.max(16)) / 10) as usize) {
            let clipped: String = line.chars().take((w.max(80) / 8) as usize).collect();
            let line_w = (clipped.len() as i32) * 8;
            let prim = Prim::Text {
                x: dst_x + 4,
                top: y,
                w: line_w,
                h: 8,
                text: clipped,
                fg: 1,
                bg: None,
                font: FontRole::Mono,
            };
            let _ = target::paint_as(a, rp, "DrawDTObjectA(text)", Some(prim), |_bm, _clip, (ox, oy)| {
                Rect::from_size(dst_x + 4 - ox, y - oy, line_w.max(1), 8)
            })?;
            y += 10;
        }
    }
    Ok(())
}

/// AddDTObject(a0 = win, a1 = req, a2 = obj, d0 = pos) -> pos
fn add_dt_object<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let win = cpu.areg(0);
    let obj_addr = cpu.areg(2);
    let pos = cpu.dreg(0);
    if let Some(obj) = a.datatypes.objects.get_mut(&obj_addr) {
        obj.window = Some(win);
    }
    cpu.set_d0(pos);
    Ok(format!("[datatypes] AddDTObject win={win:#x} obj={obj_addr:#x}"))
}

/// RefreshDTObjectA(a0 = obj, a1 = win, a2 = req, a3 = attrs)
fn refresh_dt_object<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let obj_addr = cpu.areg(0);
    let win = cpu.areg(1);
    let Some(obj) = a.datatypes.objects.get(&obj_addr).cloned() else {
        return Ok(format!("[datatypes] RefreshDTObjectA {obj_addr:#x} (unknown)"));
    };
    let target_win = if win != 0 { win } else { obj.window.unwrap_or(0) };
    if target_win != 0 {
        let rp = a.mem.read_u32(target_win + WD_RPORT).unwrap_or(0);
        if rp != 0 {
            let bl = a.mem.read_u8(target_win + 54).unwrap_or(4) as i32;
            let bt = a.mem.read_u8(target_win + 55).unwrap_or(14) as i32;
            let x = if obj.rect.0 != 0 { obj.rect.0 } else { bl };
            let y = if obj.rect.1 != 0 { obj.rect.1 } else { bt };
            render_dt_to_rp(a, &obj, rp, x, y, obj.rect.2, obj.rect.3, 0, 0)?;
        }
    }
    Ok(format!("[datatypes] RefreshDTObjectA {obj_addr:#x} win={target_win:#x}"))
}

fn remove_dt_object<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let obj_addr = cpu.areg(1);
    if let Some(obj) = a.datatypes.objects.get_mut(&obj_addr) {
        obj.window = None;
    }
    cpu.set_d0(0);
    Ok(format!("[datatypes] RemoveDTObject {obj_addr:#x}"))
}

/// DoAsyncLayout(a0 = obj, a1 = gpl) -> TRUE
fn do_async_layout<U: GuestUi>(_a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    cpu.set_d0(1);
    Ok(format!("[datatypes] DoAsyncLayout obj={:#x} -> TRUE", cpu.areg(0)))
}

/// DoDTMethodA(a0 = obj, a1 = win, a2 = req, a3 = msg) -> result
fn do_dt_method<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let obj_addr = cpu.areg(0);
    let msg_ptr = cpu.areg(3);
    if msg_ptr == 0 {
        cpu.set_d0(0);
        return Ok(format!("[datatypes] DoDTMethodA {obj_addr:#x} msg=NULL -> 0"));
    }
    let method_id = a.mem.read_u32(msg_ptr).map_err(Refusal::mem("DoDTMethodA msg"))?;
    let Some(obj) = a.datatypes.objects.get(&obj_addr).cloned() else {
        cpu.set_d0(0);
        return Ok(format!("[datatypes] DoDTMethodA {obj_addr:#x} method={method_id:#x} -> 0 (unknown obj)"));
    };
    match method_id {
        DTM_PROCLAYOUT | DTM_FRAMEBOX => {
            cpu.set_d0(1);
            Ok(format!("[datatypes] DoDTMethodA {obj_addr:#x} method={method_id:#x} -> 1"))
        }
        PDTM_READPIXELARRAY => {
            // struct pdtBlitPixelArray (datatypes/pictureclass.h):
            // 0: MethodID (u32), 4: pbpa_PixelData (u32), 8: pbpa_PixelFormat (u32),
            // 12: pbpa_PixelArrayMod (u32), 16: pbpa_Left (u32), 20: pbpa_Top (u32),
            // 24: pbpa_Width (u32), 28: pbpa_Height (u32)
            let m = Refusal::mem("pdtBlitPixelArray");
            let dst_ptr = a.mem.read_u32(msg_ptr + 4).map_err(&m)?;
            let fmt = a.mem.read_u32(msg_ptr + 8).map_err(&m)?;
            let stride = a.mem.read_u32(msg_ptr + 12).map_err(&m)? as usize;
            let sx = a.mem.read_u32(msg_ptr + 16).map_err(&m)? as usize;
            let sy = a.mem.read_u32(msg_ptr + 20).map_err(&m)? as usize;
            let w = a.mem.read_u32(msg_ptr + 24).map_err(&m)? as usize;
            let h = a.mem.read_u32(msg_ptr + 28).map_err(&m)? as usize;
            let Some(ref img) = obj.image else {
                cpu.set_d0(0);
                return Ok(format!("[datatypes] PDTM_READPIXELARRAY {obj_addr:#x} -> 0 (no image)"));
            };
            let pal = a.gfx.palette().rgb;
            let cw = w.min((img.width as usize).saturating_sub(sx));
            let ch = h.min((img.height as usize).saturating_sub(sy));
            for row in 0..ch {
                let row_addr = dst_ptr.wrapping_add((row * stride) as u32);
                let mut line = Vec::with_capacity(cw * 4);
                for col in 0..cw {
                    let px = img.pixels[(sy + row) * (img.width as usize) + (sx + col)];
                    let a_ch = ((px >> 24) & 0xFF) as u8;
                    let r = ((px >> 16) & 0xFF) as u8;
                    let g = ((px >> 8) & 0xFF) as u8;
                    let b = (px & 0xFF) as u8;
                    match fmt {
                        PBPAFMT_RGB => line.extend_from_slice(&[r, g, b]),
                        PBPAFMT_RGBA => line.extend_from_slice(&[r, g, b, a_ch]),
                        PBPAFMT_ARGB => line.extend_from_slice(&[a_ch, r, g, b]),
                        PBPAFMT_GREY8 => {
                            let y = ((299u32 * r as u32 + 587u32 * g as u32 + 114u32 * b as u32) / 1000) as u8;
                            line.push(y);
                        }
                        PBPAFMT_LUT8 | _ => line.push(nearest_pen(px, &pal)),
                    }
                }
                a.mem.write_bytes(row_addr, &line).map_err(&m)?;
            }
            cpu.set_d0(1);
            Ok(format!("[datatypes] PDTM_READPIXELARRAY {obj_addr:#x} fmt={fmt} {cw}x{ch} -> 1"))
        }
        PDTM_WRITEPIXELARRAY => {
            cpu.set_d0(1);
            Ok(format!("[datatypes] PDTM_WRITEPIXELARRAY {obj_addr:#x} -> 1"))
        }
        _ => {
            cpu.set_d0(1);
            Ok(format!("[datatypes] DoDTMethodA {obj_addr:#x} method={method_id:#x} -> 1"))
        }
    }
}

fn get_dt_methods<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let list = a.heap.alloc(16).ok_or_else(|| Refusal("datatypes: guest heap full".into()))?;
    let m = Refusal::mem("GetDTMethods");
    a.mem.write_u32(list, DTM_PROCLAYOUT).map_err(&m)?;
    a.mem.write_u32(list + 4, PDTM_READPIXELARRAY).map_err(&m)?;
    a.mem.write_u32(list + 8, 0).map_err(&m)?;
    cpu.set_d0(list);
    Ok(format!("[datatypes] GetDTMethods -> {list:#x}"))
}

fn get_dt_trigger_methods<U: GuestUi>(_a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    cpu.set_d0(0);
    Ok(String::from("[datatypes] GetDTTriggerMethods -> NULL"))
}

/// ObtainDTDrawInfoA(a0 = obj, a1 = attrs) -> APTR handle
fn obtain_dt_draw_info<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let obj = cpu.areg(0);
    let handle = obj;
    a.datatypes.draw_infos.insert(handle, obj);
    cpu.set_d0(handle);
    Ok(format!("[datatypes] ObtainDTDrawInfoA {obj:#x} -> {handle:#x}"))
}

/// DrawDTObjectA(a0 = rp, a1 = obj, d0 = x, d1 = y, d2 = w, d3 = h, d4 = th, d5 = tv, a2 = attrs) -> TRUE
fn draw_dt_object<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let rp = cpu.areg(0);
    let obj_addr = cpu.areg(1);
    let x = cpu.dreg(0) as i32;
    let y = cpu.dreg(1) as i32;
    let w = cpu.dreg(2) as i32;
    let h = cpu.dreg(3) as i32;
    let th = cpu.dreg(4) as i32;
    let tv = cpu.dreg(5) as i32;
    let Some(obj) = a.datatypes.objects.get(&obj_addr).cloned() else {
        cpu.set_d0(0);
        return Ok(format!("[datatypes] DrawDTObjectA {obj_addr:#x} -> FALSE (unknown)"));
    };
    render_dt_to_rp(a, &obj, rp, x, y, w, h, th, tv)?;
    cpu.set_d0(1);
    Ok(format!("[datatypes] DrawDTObjectA {obj_addr:#x} rp={rp:#x} ({x},{y} {w}x{h}) -> TRUE"))
}

fn release_dt_draw_info<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let handle = cpu.areg(1);
    a.datatypes.draw_infos.remove(&handle);
    Ok(format!("[datatypes] ReleaseDTDrawInfo {handle:#x}"))
}

/// Handle BOOPSI `DoMethod(obj, ...)` / `GetAttr` / `SetAttrs` / `DisposeObject` on a `datatypeclass` object.
pub fn dispatch_boopsi<U: GuestUi>(a: &mut Amiga<U>, cl: u32, obj_addr: u32, msg_ptr: u32) -> Result<u32, Refusal> {
    let method_id = a.mem.read_u32(msg_ptr).map_err(Refusal::mem("datatypeclass msg"))?;
    match method_id {
        crate::boopsi_ids::OM_DISPOSE => {
            free_dt_object(a, obj_addr);
            Ok(0)
        }
        crate::boopsi_ids::OM_GET => {
            let attr = a.mem.read_u32(msg_ptr + 4).map_err(Refusal::mem("opGet.opg_AttrID"))?;
            let storage = a.mem.read_u32(msg_ptr + 8).map_err(Refusal::mem("opGet.opg_Storage"))?;
            if let Some(obj) = a.datatypes.objects.get(&obj_addr).cloned() {
                if let Some(v) = lookup_dt_attr(&obj, attr) {
                    if storage != 0 {
                        a.mem.write_u32(storage, v).map_err(Refusal::mem("opGet storage"))?;
                    }
                    return Ok(1);
                }
            }
            crate::boopsi_native::dispatch_super(a, cl, obj_addr, msg_ptr)
        }
        crate::boopsi_ids::OM_SET | crate::boopsi_ids::OM_UPDATE => {
            let tag_ptr = a.mem.read_u32(msg_ptr + 4).map_err(Refusal::mem("opSet.ops_AttrList"))?;
            if tag_ptr != 0 {
                let pairs = tags::walk(&a.mem, tag_ptr)?;
                if let Some(obj) = a.datatypes.objects.get_mut(&obj_addr) {
                    for (tag, val) in pairs {
                        match tag {
                            GA_LEFT => obj.rect.0 = val as i32,
                            GA_TOP => obj.rect.1 = val as i32,
                            GA_WIDTH => obj.rect.2 = val as i32,
                            GA_HEIGHT => obj.rect.3 = val as i32,
                            _ => {}
                        }
                    }
                }
            }
            Ok(0)
        }
        DTM_FRAMEBOX => {
            if let Some(obj) = a.datatypes.objects.get(&obj_addr).cloned() {
                fill_frame_box(a, &obj, msg_ptr)?;
            }
            Ok(1)
        }
        DTM_PROCLAYOUT | PDTM_WRITEPIXELARRAY => Ok(1),
        PDTM_READPIXELARRAY => {
            let Some(obj) = a.datatypes.objects.get(&obj_addr).cloned() else { return Ok(0); };
            read_pixel_array_into_guest(a, &obj, msg_ptr)
        }
        crate::boopsi_ids::GM_RENDER => {
            let gp_rp = a.mem.read_u32(msg_ptr + 8).map_err(Refusal::mem("gpRender.gpr_RPort"))?;
            if gp_rp != 0 {
                if let Some(obj) = a.datatypes.objects.get(&obj_addr).cloned() {
                    render_dt_to_rp(a, &obj, gp_rp, obj.rect.0, obj.rect.1, obj.rect.2, obj.rect.3, 0, 0)?;
                }
            }
            Ok(1)
        }
        crate::boopsi_ids::GM_LAYOUT | crate::boopsi_ids::GM_DOMAIN => Ok(1),
        _ => crate::boopsi_native::dispatch_super(a, cl, obj_addr, msg_ptr),
    }
}

fn lookup_dt_attr(obj: &DtObject, tag: u32) -> Option<u32> {
    match tag {
        DTA_OBJ_NAME | DTA_NAME => Some(obj.name_addr),
        DTA_DATA_TYPE => Some(obj.desc_addr),
        DTA_GROUP_ID => Some(obj.group_id),
        DTA_NOMINAL_HORIZ | DTA_TOTAL_HORIZ | DTA_VISIBLE_HORIZ => Some(obj.rect.2.max(1) as u32),
        DTA_NOMINAL_VERT | DTA_TOTAL_VERT | DTA_VISIBLE_VERT => Some(obj.rect.3.max(1) as u32),
        DTA_TOP_HORIZ | DTA_TOP_VERT => Some(0),
        DTA_HORIZ_UNIT | DTA_VERT_UNIT => Some(1),
        PDTA_BITMAP_HEADER => Some(obj.bmhd_addr),
        PDTA_BITMAP | PDTA_DEST_BITMAP | PDTA_CLASS_BITMAP => Some(obj.bitmap_addr),
        PDTA_CREGS => Some(obj.cregs_addr),
        PDTA_COLOR_REGISTERS => Some(obj.color_regs_addr),
        PDTA_NUM_COLORS => Some(obj.num_colors),
        PDTA_MODE_ID => Some(crate::asl::DEFAULT_RTG_DISPLAY_ID),
        PDTA_ALPHA_CHANNEL => Some(0),
        TDTA_BUFFER => Some(obj.text_buf_addr),
        TDTA_BUFFER_LEN => Some(obj.text_bytes.len() as u32),
        PDTA_REMAP | PDTA_DEST_MODE | PDTA_SCREEN | DTA_TEXT_ATTR => Some(0),
        _ => None,
    }
}

fn read_pixel_array_into_guest<U: GuestUi>(a: &mut Amiga<U>, obj: &DtObject, msg_ptr: u32) -> Result<u32, Refusal> {
    let m = Refusal::mem("pdtBlitPixelArray");
    let dst_ptr = a.mem.read_u32(msg_ptr + 4).map_err(&m)?;
    let fmt = a.mem.read_u32(msg_ptr + 8).map_err(&m)?;
    let stride = a.mem.read_u32(msg_ptr + 12).map_err(&m)? as usize;
    let sx = a.mem.read_u32(msg_ptr + 16).map_err(&m)? as usize;
    let sy = a.mem.read_u32(msg_ptr + 20).map_err(&m)? as usize;
    let w = a.mem.read_u32(msg_ptr + 24).map_err(&m)? as usize;
    let h = a.mem.read_u32(msg_ptr + 28).map_err(&m)? as usize;
    let Some(ref img) = obj.image else { return Ok(0); };
    let pal = a.gfx.palette().rgb;
    let cw = w.min((img.width as usize).saturating_sub(sx));
    let ch = h.min((img.height as usize).saturating_sub(sy));
    for row in 0..ch {
        let row_addr = dst_ptr.wrapping_add((row * stride) as u32);
        let mut line = Vec::with_capacity(cw * 4);
        for col in 0..cw {
            let px = img.pixels[(sy + row) * (img.width as usize) + (sx + col)];
            let a_ch = ((px >> 24) & 0xFF) as u8;
            let r = ((px >> 16) & 0xFF) as u8;
            let g = ((px >> 8) & 0xFF) as u8;
            let b = (px & 0xFF) as u8;
            match fmt {
                PBPAFMT_RGB => line.extend_from_slice(&[r, g, b]),
                PBPAFMT_RGBA => line.extend_from_slice(&[r, g, b, a_ch]),
                PBPAFMT_ARGB => line.extend_from_slice(&[a_ch, r, g, b]),
                PBPAFMT_GREY8 => {
                    let y = ((299u32 * r as u32 + 587u32 * g as u32 + 114u32 * b as u32) / 1000) as u8;
                    line.push(y);
                }
                PBPAFMT_LUT8 | _ => line.push(nearest_pen(px, &pal)),
            }
        }
        a.mem.write_bytes(row_addr, &line).map_err(&m)?;
    }
    Ok(1)
}

fn get_dt_string<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let id = cpu.dreg(0);
    let (p, _) = alloc_cstr(a, "Datatype error")?;
    cpu.set_d0(p);
    Ok(format!("[datatypes] GetDTString({id}) -> {p:#x}"))
}

fn fill_frame_box<U: GuestUi>(a: &mut Amiga<U>, obj: &DtObject, msg_ptr: u32) -> Result<(), Refusal> {
    let m = Refusal::mem("dtFrameBox");
    let frame_info = a.mem.read_u32(msg_ptr + 12).map_err(&m)?;
    if frame_info != 0 {
        let (w, h, depth) = match obj.image {
            Some(ref img) => (img.width, img.height, 24u32),
            None => (obj.rect.2.max(1) as u32, obj.rect.3.max(1) as u32, 8u32),
        };
        a.mem.write_u32(frame_info + 0, 0).map_err(&m)?; // fri_PropertyFlags
        a.mem.write_u16(frame_info + 4, 1).map_err(&m)?; // fri_Resolution.x
        a.mem.write_u16(frame_info + 6, 1).map_err(&m)?; // fri_Resolution.y
        a.mem.write_u8(frame_info + 8, 8).map_err(&m)?;  // fri_RedBits
        a.mem.write_u8(frame_info + 9, 8).map_err(&m)?;  // fri_GreenBits
        a.mem.write_u8(frame_info + 10, 8).map_err(&m)?; // fri_BlueBits
        a.mem.write_u32(frame_info + 12, w).map_err(&m)?;
        a.mem.write_u32(frame_info + 16, h).map_err(&m)?;
        a.mem.write_u32(frame_info + 20, depth).map_err(&m)?;
    }
    Ok(())
}

/// FindMethod(a0 = methods, a1 = search_id) -> ULONG * pointing to matching entry or NULL.
fn find_method<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let mut ptr = cpu.areg(0);
    let want = cpu.areg(1);
    let mut found = 0u32;
    if ptr != 0 {
        for _ in 0..256 {
            let id = a.mem.read_u32(ptr).map_err(Refusal::mem("FindMethod"))?;
            if id == 0 {
                break;
            }
            if id == want {
                found = ptr;
                break;
            }
            ptr = ptr.wrapping_add(4);
        }
    }
    cpu.set_d0(found);
    Ok(format!("[datatypes] FindMethod want={want:#x} -> {found:#x}"))
}

/// FindTriggerMethod(a0 = dtm, a1 = command, d0 = method) -> struct DTMethod * or NULL.
fn find_trigger_method<U: GuestUi>(_a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    cpu.set_d0(0);
    Ok(String::from("[datatypes] FindTriggerMethod -> NULL"))
}

/// CopyDTMethods(a0 = methods, a1 = include, a2 = exclude) -> ULONG *
fn copy_dt_methods<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let mut src = cpu.areg(0);
    let mut vals = Vec::new();
    if src != 0 {
        for _ in 0..128 {
            let v = a.mem.read_u32(src).map_err(Refusal::mem("CopyDTMethods"))?;
            if v == 0 {
                break;
            }
            vals.push(v);
            src = src.wrapping_add(4);
        }
    }
    vals.push(0);
    let bytes = (vals.len() * 4) as u32;
    let dst = crate::memory::alloc_zeroed(a, bytes, "CopyDTMethods")?;
    for (i, v) in vals.iter().enumerate() {
        a.mem.write_u32(dst + (i * 4) as u32, *v).map_err(Refusal::mem("CopyDTMethods"))?;
    }
    cpu.set_d0(dst);
    Ok(format!("[datatypes] CopyDTMethods -> {dst:#x} ({} entries)", vals.len() - 1))
}

fn free_dt_methods<U: GuestUi>(_a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    Ok(format!("[datatypes] FreeDTMethods {:#x}", cpu.areg(0)))
}

/// DoDTDomainA(a0=obj, a1=win, a2=req, a3=rport, d0=which, a4=domain, a5=attrs) -> TRUE
fn do_dt_domain<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let obj_addr = cpu.areg(0);
    let domain = cpu.areg(4);
    if domain != 0 {
        let (w, h) = a.datatypes.objects.get(&obj_addr).map(|o| (o.rect.2, o.rect.3)).unwrap_or((320, 200));
        let m = Refusal::mem("DoDTDomainA IBox");
        a.mem.write_u16(domain + 0, 0).map_err(&m)?;
        a.mem.write_u16(domain + 2, 0).map_err(&m)?;
        a.mem.write_u16(domain + 4, w.clamp(1, i16::MAX as i32) as u16).map_err(&m)?;
        a.mem.write_u16(domain + 6, h.clamp(1, i16::MAX as i32) as u16).map_err(&m)?;
    }
    cpu.set_d0(1);
    Ok(format!("[datatypes] DoDTDomainA {obj_addr:#x} -> TRUE"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use heirloom_ui_abi::RecordingGuestUi;

    fn make_hunk(code: &[u8]) -> Vec<u8> {
        let mut h = Vec::new();
        let longs = (code.len() as u32 + 3) / 4;
        for w in [0x0000_03F3, 0, 1, 0, 0, longs, 0x0000_03E9, longs] {
            h.extend_from_slice(&w.to_be_bytes());
        }
        h.extend_from_slice(code);
        while h.len() % 4 != 0 { h.push(0); }
        h.extend_from_slice(&0x0000_03F2u32.to_be_bytes());
        h
    }

    #[test]
    fn creating_and_disposing_80_picture_dtobjects_does_not_exhaust_sys_heap() {
        let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
        // Minimal 2x2 24-bit uncompressed BMP so builtin picture decoder succeeds without external host.
        let mut bmp = vec![0u8; 54 + 16];
        let bmp_len = bmp.len() as u32;
        bmp[0..2].copy_from_slice(b"BM");
        bmp[2..6].copy_from_slice(&bmp_len.to_le_bytes());
        bmp[10..14].copy_from_slice(&54u32.to_le_bytes());
        bmp[14..18].copy_from_slice(&40u32.to_le_bytes());
        bmp[18..22].copy_from_slice(&2u32.to_le_bytes());
        bmp[22..26].copy_from_slice(&2u32.to_le_bytes());
        bmp[26..28].copy_from_slice(&1u16.to_le_bytes());
        bmp[28..30].copy_from_slice(&24u16.to_le_bytes());
        a.fs.vol.put_file("SYS:icon.bmp", bmp).unwrap();

        // 68k loop: OpenLibrary("datatypes.library", 0), then 80x NewDTObjectA("SYS:icon.bmp", NULL) + DisposeDTObject.
        let mut c = vec![0u8; 0x200];
        let base = crate::machine::LOAD_BASE;
        let lib_str = base + 0x100;
        let pic_str = base + 0x120;
        let mut p = 0usize;
        let emit = |buf: &mut [u8], pos: &mut usize, w: &[u16]| {
            for &x in w {
                buf[*pos..*pos + 2].copy_from_slice(&x.to_be_bytes());
                *pos += 2;
            }
        };
        // movea.l 4.w,a6; lea lib_str,a1; moveq #0,d0; jsr -552(a6); movea.l d0,a6
        emit(&mut c, &mut p, &[0x2C78, 0x0004]);
        emit(&mut c, &mut p, &[0x227C, (lib_str >> 16) as u16, lib_str as u16]);
        emit(&mut c, &mut p, &[0x7000, 0x4EAE, (-552i16) as u16, 0x2C40]);
        // moveq #79,d7
        emit(&mut c, &mut p, &[0x7E4F]);
        let loop_off = p;
        // move.l #pic_str,d0; suba.l a0,a0; jsr -48(a6) (NewDTObjectA)
        emit(&mut c, &mut p, &[0x203C, (pic_str >> 16) as u16, pic_str as u16, 0x91C8, 0x4EAE, (-48i16) as u16]);
        // tst.l d0; beq.s fail; movea.l d0,a0; jsr -54(a6) (DisposeDTObject)
        emit(&mut c, &mut p, &[0x4A80, 0x6708, 0x2040, 0x4EAE, (-54i16) as u16]);
        let disp = (loop_off as i16) - (p as i16 + 2);
        // dbra d7,loop_off; moveq #0,d0; rts
        emit(&mut c, &mut p, &[0x51CF, disp as u16, 0x7000, 0x4E75]);
        // fail: moveq #1,d0; rts
        emit(&mut c, &mut p, &[0x7001, 0x4E75]);
        c[0x100..0x100 + 18].copy_from_slice(b"datatypes.library\0");
        c[0x120..0x120 + 13].copy_from_slice(b"SYS:icon.bmp\0");

        let r = a.run_hunk(&make_hunk(&c), 500_000).unwrap();
        assert_eq!(r.stats.bus_faults, 0);
        assert!(r.stats.halted);
        assert!(a.refused.is_empty(), "creating & disposing 80 picture DTObjects must not refuse: {:?}", a.refused);
    }
}
