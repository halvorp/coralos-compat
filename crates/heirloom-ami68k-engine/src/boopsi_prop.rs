//! propgclass: a gadgetclass whose instance data is a `struct PropInfo` (22 bytes) followed by the PGA_Total / PGA_Visible /
//! PGA_Top the owner last set; `Gadget.SpecialInfo` points at the PropInfo. Defaults (OM_NEW) PROPNEWLOOK|AUTOKNOB|FREEVERT with
//! both bodies MAXBODY; pots and bodies are recomputed from Total/Visible/Top whenever one of those is set.
//! Facts: AROS rom/intuition propgclass.c (OM_NEW, set_propgclass:306 PGA_Freedom takes the FREEHORIZ/FREEVERT flag values, OM_GET:525; APL), intuition/gadgetclass.h (PGA_*), intuition.h (PropInfo).
//! Not implemented: input (GM_GOACTIVE / GM_HANDLEINPUT stay named refusals in boopsi_native::dispatch), PGA_NewLook is stored but
//! drawn by boopsi_draw::prop_render only as the checkerboard.
use crate::boopsi_ids::*;
use crate::boopsi_native::{dispatch_super, gadget};
use crate::{machine::Amiga, tags, Refusal};
use heirloom_ui_abi::GuestUi;

const PGA: u32 = tags::TAG_USER + 0x31000;
pub const PGA_FREEDOM: u32 = PGA + 1;
pub const PGA_BORDERLESS: u32 = PGA + 2;
pub const PGA_HORIZ_POT: u32 = PGA + 3;
pub const PGA_HORIZ_BODY: u32 = PGA + 4;
pub const PGA_VERT_POT: u32 = PGA + 5;
pub const PGA_VERT_BODY: u32 = PGA + 6;
pub const PGA_TOTAL: u32 = PGA + 7;
pub const PGA_VISIBLE: u32 = PGA + 8;
pub const PGA_TOP: u32 = PGA + 9;
pub const PGA_NEW_LOOK: u32 = PGA + 10;

const AUTOKNOB: u32 = 1;
const FREEHORIZ: u32 = 2;
const FREEVERT: u32 = 4;
const BORDERLESS: u32 = 8;
const NEWLOOK: u32 = 0x10;
const MAXBODY: u32 = 0xFFFF;
const MAXPOT: u32 = 0xFFFF;
const FLAGS: u32 = 0;
const HPOT: u32 = 2;
const VPOT: u32 = 4;
const HBODY: u32 = 6;
const VBODY: u32 = 8;
const TOTAL: u32 = 24;
const VISIBLE: u32 = 28;
const TOP: u32 = 32;

fn data<U: GuestUi>(a: &Amiga<U>, cl: u32, g: u32) -> u32 { g + a.classes.recs.get(&cl).map(|r| r.inst_offset).unwrap_or(0) }
fn r16<U: GuestUi>(a: &Amiga<U>, at: u32) -> Result<u32, Refusal> { a.mem.read_u16(at).map(u32::from).map_err(Refusal::mem("PropInfo")) }
fn r32<U: GuestUi>(a: &Amiga<U>, at: u32) -> Result<u32, Refusal> { a.mem.read_u32(at).map_err(Refusal::mem("PropInfo")) }
fn w16<U: GuestUi>(a: &mut Amiga<U>, at: u32, v: u32) -> Result<(), Refusal> { a.mem.write_u16(at, v as u16).map_err(Refusal::mem("PropInfo")) }
fn w32<U: GuestUi>(a: &mut Amiga<U>, at: u32, v: u32) -> Result<(), Refusal> { a.mem.write_u32(at, v).map_err(Refusal::mem("PropInfo")) }

/// Pot and body for the (total, visible, top) triple of one axis.
pub fn pot_body(total: u32, visible: u32, top: u32) -> (u32, u32) {
    if total <= visible || total == 0 { return (0, MAXBODY); }
    let body = (visible as u64 * MAXBODY as u64 / total as u64) as u32;
    let pot = (top.min(total - visible) as u64 * MAXPOT as u64 / (total - visible) as u64) as u32;
    (pot, body)
}

fn set<U: GuestUi>(a: &mut Amiga<U>, pi: u32, tag_list: u32) -> Result<u32, Refusal> {
    let mut visual = 0;
    let mut geometry = false;
    for (tag, d) in tags::walk(&a.mem, tag_list)? {
        let flag = |a: &mut Amiga<U>, m: u32, on: bool| -> Result<(), Refusal> { let f = r16(a, pi + FLAGS)?; w16(a, pi + FLAGS, if on { f | m } else { f & !m }) };
        match tag {
            PGA_FREEDOM => { let f = r16(a, pi + FLAGS)? & !(FREEHORIZ | FREEVERT); w16(a, pi + FLAGS, f | (d & (FREEHORIZ | FREEVERT)))?; visual = 1 }
            PGA_BORDERLESS => { flag(a, BORDERLESS, d != 0)?; visual = 1 }
            PGA_NEW_LOOK => { flag(a, NEWLOOK, d != 0)?; visual = 1 }
            PGA_HORIZ_POT => { w16(a, pi + HPOT, d)?; visual = 1 }
            PGA_HORIZ_BODY => { w16(a, pi + HBODY, d)?; visual = 1 }
            PGA_VERT_POT => { w16(a, pi + VPOT, d)?; visual = 1 }
            PGA_VERT_BODY => { w16(a, pi + VBODY, d)?; visual = 1 }
            PGA_TOTAL => { w32(a, pi + TOTAL, d)?; geometry = true }
            PGA_VISIBLE => { w32(a, pi + VISIBLE, d)?; geometry = true }
            PGA_TOP => { w32(a, pi + TOP, d)?; geometry = true }
            _ => {}
        }
    }
    if geometry {
        let (t, v, p) = (r32(a, pi + TOTAL)?, r32(a, pi + VISIBLE)?, r32(a, pi + TOP)?);
        let (pot, body) = pot_body(t, v, p);
        let vert = r16(a, pi + FLAGS)? & FREEVERT != 0;
        let (po, bo) = if vert { (VPOT, VBODY) } else { (HPOT, HBODY) };
        w16(a, pi + po, pot)?; w16(a, pi + bo, body)?;
        visual = 1;
    }
    Ok(visual)
}

pub fn prop<U: GuestUi>(a: &mut Amiga<U>, cl: u32, o: u32, msg: u32) -> Result<u32, Refusal> {
    let id = a.mem.read_u32(msg).map_err(Refusal::mem("method id"))?;
    match id {
        OM_NEW => {
            // The superclass builds the gadget (and applies the GA_ tags); then the PropInfo goes in with its defaults and the PGA_ tags.
            let g = dispatch_super(a, cl, o, msg)?;
            if g == 0 { return Ok(0); }
            let pi = data(a, cl, g);
            for off in (0..PROP_DATA).step_by(2) { w16(a, pi + off, 0)?; }
            w16(a, pi + FLAGS, AUTOKNOB | FREEVERT | NEWLOOK)?;
            w16(a, pi + HBODY, MAXBODY)?; w16(a, pi + VBODY, MAXBODY)?;
            w32(a, g + GAD_SPECIAL, pi)?;
            let t = r32(a, msg + 4)?;
            set(a, pi, t)?;
            Ok(g)
        }
        OM_SET | OM_UPDATE => {
            let sup = gadget(a, cl, o, msg)?;
            let t = r32(a, msg + 4)?;
            let pi = data(a, cl, o);
            Ok(sup.wrapping_add(set(a, pi, t)?))
        }
        OM_GET => {
            let (attr, store, pi) = (r32(a, msg + 4)?, r32(a, msg + OPSET_GINFO)?, data(a, cl, o));
            let f = r16(a, pi + FLAGS)?;
            let v = match attr {
                PGA_FREEDOM => f & (FREEHORIZ | FREEVERT),
                PGA_BORDERLESS => (f & BORDERLESS != 0) as u32,
                PGA_NEW_LOOK => (f & NEWLOOK != 0) as u32,
                PGA_HORIZ_POT => r16(a, pi + HPOT)?, PGA_HORIZ_BODY => r16(a, pi + HBODY)?,
                PGA_VERT_POT => r16(a, pi + VPOT)?, PGA_VERT_BODY => r16(a, pi + VBODY)?,
                PGA_TOTAL => r32(a, pi + TOTAL)?, PGA_VISIBLE => r32(a, pi + VISIBLE)?, PGA_TOP => r32(a, pi + TOP)?,
                _ => return gadget(a, cl, o, msg),
            };
            w32(a, store, v)?;
            Ok(1)
        }
        _ => gadget(a, cl, o, msg),
    }
}
