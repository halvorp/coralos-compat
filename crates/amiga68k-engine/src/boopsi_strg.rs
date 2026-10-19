//! strgclass: a gadgetclass whose instance data is a `StrGData` = `struct StringInfo` (36) + `struct StringExtend` (36)
//! + flags, with `Gadget.SpecialInfo` pointing at the StringInfo (GFLG_STRINGEXTEND set). OM_NEW/OM_SET/OM_GET/OM_DISPOSE
//! are real (buffers allocated when the tags give none, STRINGA_TextVal / LongVal, ...). GM_RENDER puts / updates the NATIVE
//! field (native_gadgets.rs); the EDITING (GM_GOACTIVE, GM_HANDLEINPUT, GM_GOINACTIVE) is the host's, so those methods are a
//! named refusal, not a silent 0. Facts: AROS rom/intuition strgclass.c strg_set / OM_NEW / OM_GET / OM_DISPOSE,
//! intuition/intuition.h StringInfo / StringExtend.
use alloc::format;
use alloc::string::String;
use crate::boopsi_ids::*;
use crate::boopsi_native::{dispatch_super};
use crate::{machine::Amiga, memory, tags, Refusal};
use guestui_abi::GuestUi;

const SI_BUFFER: u32 = 0;
const SI_UNDO: u32 = 4;
const SI_BUFFER_POS: u32 = 8;
const SI_DISP_POS: u32 = 12;
const SI_NUM_CHARS: u32 = 16;
const SI_EXTENSION: u32 = 24;
const SI_LONG_INT: u32 = 28;
const EXT: u32 = 36; // StringExtend inside the instance
const EXT_PENS: u32 = 4;
const EXT_ACTIVE_PENS: u32 = 6;
const EXT_WORK_BUFFER: u32 = 16;
const GFLG_STRINGEXTEND: u32 = 1 << 10;
pub(crate) const GACT_LONGINT: u32 = 1 << 11;
const STRINGA: u32 = tags::TAG_USER + 0x32000;
pub const STRINGA_MAX_CHARS: u32 = STRINGA + 1;
pub const STRINGA_BUFFER: u32 = STRINGA + 2;
pub const STRINGA_UNDO_BUFFER: u32 = STRINGA + 3;
pub const STRINGA_WORK_BUFFER: u32 = STRINGA + 4;
pub const STRINGA_BUFFER_POS: u32 = STRINGA + 5;
pub const STRINGA_DISP_POS: u32 = STRINGA + 6;
pub const STRINGA_LONG_VAL: u32 = STRINGA + 17;
pub const STRINGA_TEXT_VAL: u32 = STRINGA + 18;

fn w16<U: GuestUi>(a: &mut Amiga<U>, at: u32, v: u32) -> Result<(), Refusal> { a.mem.write_u16(at, v as u16).map_err(Refusal::mem("StringInfo")) }
fn w32<U: GuestUi>(a: &mut Amiga<U>, at: u32, v: u32) -> Result<(), Refusal> { a.mem.write_u32(at, v).map_err(Refusal::mem("StringInfo")) }
fn r32<U: GuestUi>(a: &Amiga<U>, at: u32) -> Result<u32, Refusal> { a.mem.read_u32(at).map_err(Refusal::mem("StringInfo")) }

fn data<U: GuestUi>(a: &Amiga<U>, cl: u32, g: u32) -> u32 { g + a.classes.recs.get(&cl).map(|r| r.inst_offset).unwrap_or(0) }

/// Write `s` plus a NUL into the buffer at `buf`, bounded by MaxChars (a longer string is cut, as strcpy into a sized buffer cannot be).
fn put_str<U: GuestUi>(a: &mut Amiga<U>, si: u32, s: &str) -> Result<(), Refusal> {
    let buf = r32(a, si + SI_BUFFER)?;
    let max = a.mem.read_u16(si + SI_MAX_CHARS).map_err(Refusal::mem("StringInfo"))? as usize;
    let n = s.len().min(max.saturating_sub(1));
    a.mem.write_bytes(buf, &s.as_bytes()[..n]).map_err(Refusal::mem("string buffer"))?;
    a.mem.write_u8(buf + n as u32, 0).map_err(Refusal::mem("string buffer"))
}

/// Store `text` as the gadget's string the way Intuition's UpdateStrGadget does: into the buffer (cut to MaxChars), NumChars, the
/// cursor at the end, and LongInt when the gadget is a number field.
pub(crate) fn store_text<U: GuestUi>(a: &mut Amiga<U>, si: u32, text: &str, long: bool) -> Result<(), Refusal> {
    put_str(a, si, text)?;
    let buf = r32(a, si + SI_BUFFER)?;
    let n = a.mem.read_cstr(buf).map_err(Refusal::mem("string buffer"))?.len() as u32;
    w16(a, si + SI_NUM_CHARS, n)?;
    w16(a, si + SI_BUFFER_POS, n)?;
    if long { w32(a, si + SI_LONG_INT, text.trim().parse::<i32>().unwrap_or(0) as u32)?; }
    Ok(())
}

/// strg_set: the STRINGA_ tags. Non-zero when something visible changed.
fn set<U: GuestUi>(a: &mut Amiga<U>, g: u32, si: u32, tag_list: u32, is_new: bool) -> Result<u32, Refusal> {
    let mut r = 0;
    for (tag, d) in tags::walk(&a.mem, tag_list)? {
        match tag {
            STRINGA_LONG_VAL if !is_new => {
                w32(a, si + SI_LONG_INT, d)?;
                put_str(a, si, &format!("{}", d as i32))?;
                let act = a.mem.read_u16(g + GAD_ACTIVATION).map_err(Refusal::mem("Gadget"))? as u32 | GACT_LONGINT;
                a.mem.write_u16(g + GAD_ACTIVATION, act as u16).map_err(Refusal::mem("Gadget"))?;
                r = 1;
            }
            STRINGA_TEXT_VAL if !is_new => {
                let s: String = if d == 0 { String::new() } else { a.mem.read_cstr(d).map_err(Refusal::mem("STRINGA_TextVal"))? };
                put_str(a, si, &s)?;
                let act = a.mem.read_u16(g + GAD_ACTIVATION).map_err(Refusal::mem("Gadget"))? as u32 & !GACT_LONGINT;
                a.mem.write_u16(g + GAD_ACTIVATION, act as u16).map_err(Refusal::mem("Gadget"))?;
                r = 1;
            }
            STRINGA_MAX_CHARS => w16(a, si + SI_MAX_CHARS, d)?,
            STRINGA_BUFFER => w32(a, si + SI_BUFFER, d)?,
            STRINGA_UNDO_BUFFER => w32(a, si + SI_UNDO, d)?,
            STRINGA_WORK_BUFFER => w32(a, si + EXT + EXT_WORK_BUFFER, d)?,
            STRINGA_BUFFER_POS => { w16(a, si + SI_BUFFER_POS, d)?; r = 1; }
            STRINGA_DISP_POS => { w16(a, si + SI_DISP_POS, d)?; r = 1; }
            _ => {}
        }
    }
    Ok(r)
}

pub fn strg<U: GuestUi>(a: &mut Amiga<U>, cl: u32, o: u32, msg: u32) -> Result<u32, Refusal> {
    let id = r32(a, msg)?;
    match id {
        OM_NEW => {
            let g = dispatch_super(a, cl, o, msg)?;
            if g == 0 { return Ok(0); }
            let si = data(a, cl, g);
            w16(a, si + SI_MAX_CHARS, 80)?;
            a.mem.write_u8(si + EXT + EXT_PENS, 1).map_err(Refusal::mem("StringExtend"))?;
            a.mem.write_u8(si + EXT + EXT_ACTIVE_PENS, 1).map_err(Refusal::mem("StringExtend"))?;
            let list = r32(a, msg + 4)?;
            set(a, g, si, list, true)?;
            let max = a.mem.read_u16(si + SI_MAX_CHARS).map_err(Refusal::mem("StringInfo"))? as u32;
            for off in [SI_BUFFER, SI_UNDO, EXT + EXT_WORK_BUFFER] {
                if r32(a, si + off)? == 0 {
                    let b = memory::alloc_zeroed(a, max.max(1), "strgclass buffer")?;
                    w32(a, si + off, b)?;
                    a.classes.strg_bufs.entry(g).or_default().push((b, max.max(1)));
                }
            }
            let t = tags::walk(&a.mem, list)?;
            if let Some(tv) = tags::get(&t, STRINGA_TEXT_VAL) {
                let s = if tv == 0 { String::new() } else { a.mem.read_cstr(tv).map_err(Refusal::mem("STRINGA_TextVal"))? };
                put_str(a, si, &s)?;
            }
            if let Some(lv) = tags::get(&t, STRINGA_LONG_VAL) {
                w32(a, si + SI_LONG_INT, lv)?;
                put_str(a, si, &format!("{}", lv as i32))?;
                let act = a.mem.read_u16(g + GAD_ACTIVATION).map_err(Refusal::mem("Gadget"))? | GACT_LONGINT as u16;
                a.mem.write_u16(g + GAD_ACTIVATION, act).map_err(Refusal::mem("Gadget"))?;
            }
            w32(a, g + GAD_SPECIAL, si)?;
            let f = a.mem.read_u16(g + GAD_FLAGS).map_err(Refusal::mem("Gadget"))? | GFLG_STRINGEXTEND as u16;
            a.mem.write_u16(g + GAD_FLAGS, f).map_err(Refusal::mem("Gadget"))?;
            w32(a, si + SI_EXTENSION, si + EXT)?;
            Ok(g)
        }
        OM_SET | OM_UPDATE => {
            let sup = dispatch_super(a, cl, o, msg)?;
            let si = data(a, cl, o);
            let list = r32(a, msg + 4)?;
            Ok(sup.wrapping_add(set(a, o, si, list, false)?))
        }
        OM_GET => {
            let (attr, store) = (r32(a, msg + 4)?, r32(a, msg + OPSET_GINFO)?);
            let si = data(a, cl, o);
            let long = a.mem.read_u16(o + GAD_ACTIVATION).map_err(Refusal::mem("Gadget"))? as u32 & GACT_LONGINT != 0;
            let v = match attr {
                STRINGA_LONG_VAL => if long { r32(a, si + SI_LONG_INT)? } else { 0 },
                STRINGA_TEXT_VAL => if long { 0 } else { r32(a, si + SI_BUFFER)? },
                _ => return dispatch_super(a, cl, o, msg),
            };
            a.mem.write_u32(store, v).map_err(Refusal::mem("opg_Storage"))?;
            Ok(1)
        }
        OM_DISPOSE => {
            for (b, n) in a.classes.strg_bufs.remove(&o).unwrap_or_default() { a.heap.free(b, n).map_err(|e| Refusal(format!("strgclass buffer: {e}")))?; }
            dispatch_super(a, cl, o, msg)
        }
        GM_RENDER => crate::native_gadgets::render(a, o, msg),
        GM_GOACTIVE | GM_HANDLEINPUT | GM_GOINACTIVE => Err(Refusal(format!("strgclass method {id}: the string field is a native control, its editing is the host's (native_gadgets.rs)"))),
        _ => dispatch_super(a, cl, o, msg),
    }
}
