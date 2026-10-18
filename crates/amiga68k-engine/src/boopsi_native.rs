//! The host implementations of the built-in classes: rootclass (object memory and list membership), imageclass and
//! gadgetclass (the guest structs `struct Image` / `struct ExtGadget` ARE the instance data, written field by field).
//! One entry, [`dispatch`], runs a method on a native class; a native subclass reaches its superclass with
//! [`dispatch_super`] directly (native classes only ever have native supers). Guest classes reach these through the
//! trap slot with the ordinary DoSuperMethod. Facts: AROS rom/intuition rootclass.c, imageclass.c, gadgetclass.c.
use alloc::format;
use crate::boopsi_ids::*;
use crate::iclass::{count_object, Native, ObjRec};
use crate::{exec_wait, machine::Amiga, memory, tags, Refusal};
use guestui_abi::GuestUi;

fn rd<U: GuestUi>(a: &Amiga<U>, at: u32, what: &'static str) -> Result<u32, Refusal> { a.mem.read_u32(at).map_err(Refusal::mem(what)) }
fn w16<U: GuestUi>(a: &mut Amiga<U>, at: u32, v: u32, what: &'static str) -> Result<(), Refusal> { a.mem.write_u16(at, v as u16).map_err(Refusal::mem(what)) }
fn r16<U: GuestUi>(a: &Amiga<U>, at: u32, what: &'static str) -> Result<u32, Refusal> { a.mem.read_u16(at).map(u32::from).map_err(Refusal::mem(what)) }
/// Set / clear `bit` of the UWORD at `at` according to `on`.
fn bit<U: GuestUi>(a: &mut Amiga<U>, at: u32, bit: u32, on: bool) -> Result<(), Refusal> {
    let v = r16(a, at, "gadget word")?;
    w16(a, at, if on { v | bit } else { v & !bit }, "gadget word")
}

/// Run `method` of native class `cl` on `obj` (for OM_NEW, `obj` is the class the object is being made from).
pub fn dispatch<U: GuestUi>(a: &mut Amiga<U>, native: Native, cl: u32, obj: u32, msg: u32) -> Result<u32, Refusal> {
    // GM_RENDER of a system gadget class puts / updates the NATIVE control (native_gadgets.rs; it paints nothing); sysiclass IM_DRAW paints
    // nothing either (boopsi_draw.rs). Their input methods are the host's: GM_GOACTIVE / GM_HANDLEINPUT stay named refusals.
    if matches!(native, Native::Button | Native::Prop | Native::Sysi) {
        let id = rd(a, msg, "method id")?;
        match (native, id) {
            (Native::Button | Native::Prop, GM_RENDER) => return crate::native_gadgets::render(a, obj, msg),
            (Native::Sysi, IM_DRAW) => {
                let (rp, x, y, st) = (rd(a, msg + 4, "impDraw")?, r16(a, msg + 8, "impDraw")? as i16 as i32, r16(a, msg + 10, "impDraw")? as i16 as i32, rd(a, msg + 12, "impDraw")?);
                crate::boopsi_draw::draw_native_image(a, obj, rp, x, y, st)?;
                return Ok(1);
            }
            (_, GM_RENDER | GM_GOACTIVE | GM_HANDLEINPUT | IM_DRAW) => {
                return Err(Refusal(format!("{}: method {id:#x} (its own drawing / input) is not implemented yet; only the gadgetclass/imageclass struct and attributes are", a.classes.name_of(cl))));
            }
            _ => {}
        }
    }
    match native {
        Native::Root => root(a, cl, obj, msg),
        Native::Image | Native::Sysi => image(a, cl, obj, msg),
        Native::Frame => crate::boopsi_frame::frame(a, cl, obj, msg),
        Native::Gadget | Native::Button => gadget(a, cl, obj, msg),
        Native::Prop => crate::boopsi_prop::prop(a, cl, obj, msg),
        Native::Strg => crate::boopsi_strg::strg(a, cl, obj, msg),
        Native::Mui(_) => Err(Refusal(format!("{}: a MUI class is reached through its trap slot (boopsi.rs dispatcher), not dispatch_super", a.classes.name_of(cl)))),
    }
}

/// DoSuperMethod from a native class into its (native) superclass.
pub fn dispatch_super<U: GuestUi>(a: &mut Amiga<U>, cl: u32, obj: u32, msg: u32) -> Result<u32, Refusal> {
    let sup = a.classes.recs.get(&cl).map(|r| r.sup).unwrap_or(0);
    let native = a.classes.recs.get(&sup).and_then(|r| r.native).ok_or_else(|| Refusal(format!("{}: no native superclass", a.classes.name_of(cl))))?;
    dispatch(a, native, sup, obj, msg)
}

fn root<U: GuestUi>(a: &mut Amiga<U>, _cl: u32, obj: u32, msg: u32) -> Result<u32, Refusal> {
    match rd(a, msg, "method id")? {
        OM_NEW => {
            // `obj` is the class (rootclass.c: "NOTE: The object argument is actually the class!").
            let (size, fresh) = match a.classes.recs.get(&obj) { Some(r) => (r.object_size(), true), None => (0, false) };
            if !fresh { return Err(Refusal(format!("rootclass OM_NEW: {obj:#x} is not a class"))); }
            let block = memory::alloc_zeroed(a, size, "OM_NEW instance")?;
            a.mem.write_u32(block + OBJ_CLASS, obj).map_err(Refusal::mem("_Object.o_Class"))?;
            count_object(a, obj, true)?;
            a.classes.objs.insert(block + OBJ_HEADER, ObjRec { class: obj, block, size });
            Ok(block + OBJ_HEADER)
        }
        OM_DISPOSE => {
            let rec = a.classes.objs.remove(&obj).ok_or_else(|| Refusal(format!("rootclass OM_DISPOSE: {obj:#x} is not a live object")))?;
            count_object(a, rec.class, false)?;
            a.heap.free(rec.block, rec.size).map_err(|e| Refusal(format!("OM_DISPOSE: {e}")))?;
            Ok(0)
        }
        OM_ADDTAIL => { let list = rd(a, msg + 4, "opat_List")?; exec_wait::add_tail(a, list, obj - OBJ_HEADER)?; Ok(1) }
        OM_REMOVE => { exec_wait::remove(a, obj - OBJ_HEADER)?; Ok(1) }
        // OM_SET/GET/UPDATE/NOTIFY/ADDMEMBER/REMMEMBER and any method rootclass does not know are ignored (rootclass.c).
        _ => Ok(0),
    }
}

pub(crate) fn image<U: GuestUi>(a: &mut Amiga<U>, cl: u32, o: u32, msg: u32) -> Result<u32, Refusal> {
    match rd(a, msg, "method id")? {
        OM_NEW => {
            let im = dispatch_super(a, cl, o, msg)?;
            if im == 0 { return Ok(0); }
            w16(a, im + IMG_WIDTH, 80, "Image.Width")?;
            w16(a, im + IMG_HEIGHT, 40, "Image.Height")?;
            w16(a, im + IMG_DEPTH, 0xFFFF, "Image.Depth")?; // CUSTOMIMAGEDEPTH: how intuition knows a boopsi image
            let t = rd(a, msg + 4, "ops_AttrList")?;
            image_set(a, im, t)?;
            if a.classes.recs.get(&cl).and_then(|r| r.native) == Some(Native::Sysi) { crate::boopsi_draw::sysi_new(a, im, t)?; }
            Ok(im)
        }
        OM_SET | OM_UPDATE => { let t = rd(a, msg + 4, "ops_AttrList")?; image_set(a, o, t) }
        OM_GET => {
            let (attr, store) = (rd(a, msg + 4, "opg_AttrID")?, rd(a, msg + OPSET_GINFO, "opg_Storage")?);
            let v = match attr {
                IA_LEFT => r16(a, o + IMG_LEFT, "Image")? as i16 as i32 as u32, IA_TOP => r16(a, o + IMG_TOP, "Image")? as i16 as i32 as u32,
                IA_WIDTH => r16(a, o + IMG_WIDTH, "Image")? as i16 as i32 as u32, IA_HEIGHT => r16(a, o + IMG_HEIGHT, "Image")? as i16 as i32 as u32,
                IA_FGPEN => a.mem.read_u8(o + IMG_PLANE_PICK).map_err(Refusal::mem("Image"))? as u32,
                IA_BGPEN => a.mem.read_u8(o + IMG_PLANE_ONOFF).map_err(Refusal::mem("Image"))? as u32,
                IA_DATA => rd(a, o + IMG_DATA, "Image")?,
                _ => return Ok(0),
            };
            a.mem.write_u32(store, v).map_err(Refusal::mem("opg_Storage"))?;
            Ok(1)
        }
        IM_HITTEST => {
            let (x, y) = (r16(a, msg + 4, "impHitTest")? as i16 as i32, r16(a, msg + 6, "impHitTest")? as i16 as i32);
            let (l, t) = (r16(a, o + IMG_LEFT, "Image")? as i16 as i32, r16(a, o + IMG_TOP, "Image")? as i16 as i32);
            let (w, h) = (r16(a, o + IMG_WIDTH, "Image")? as i16 as i32, r16(a, o + IMG_HEIGHT, "Image")? as i16 as i32);
            Ok((x >= l && x < l + w && y >= t && y < t + h) as u32)
        }
        _ => dispatch_super(a, cl, o, msg),
    }
}

/// imageclass `_om_set`: 1 when every tag was understood, else 0.
fn image_set<U: GuestUi>(a: &mut Amiga<U>, im: u32, tag_list: u32) -> Result<u32, Refusal> {
    let mut all = true;
    for (tag, d) in tags::walk(&a.mem, tag_list)? {
        match tag {
            IA_LEFT => w16(a, im + IMG_LEFT, d, "Image")?, IA_TOP => w16(a, im + IMG_TOP, d, "Image")?,
            IA_WIDTH => w16(a, im + IMG_WIDTH, d, "Image")?, IA_HEIGHT => w16(a, im + IMG_HEIGHT, d, "Image")?,
            IA_FGPEN => a.mem.write_u8(im + IMG_PLANE_PICK, d as u8).map_err(Refusal::mem("Image"))?,
            IA_BGPEN => a.mem.write_u8(im + IMG_PLANE_ONOFF, d as u8).map_err(Refusal::mem("Image"))?,
            IA_DATA => a.mem.write_u32(im + IMG_DATA, d).map_err(Refusal::mem("Image"))?,
            _ => all = false,
        }
    }
    Ok(all as u32)
}

pub(crate) fn gadget<U: GuestUi>(a: &mut Amiga<U>, cl: u32, o: u32, msg: u32) -> Result<u32, Refusal> {
    let id = rd(a, msg, "method id")?;
    match id {
        OM_NEW => {
            let g = dispatch_super(a, cl, o, msg)?;
            if g == 0 { return Ok(0); }
            w16(a, g + GAD_FLAGS, GFLG_EXTENDED, "Gadget.Flags")?;
            w16(a, g + GAD_TYPE, GTYP_CUSTOMGADGET, "Gadget.GadgetType")?;
            a.mem.write_u32(g + EGAD_MORE_FLAGS, GMORE_BOOPSIGADGET).map_err(Refusal::mem("Gadget.MoreFlags"))?;
            a.mem.write_u32(g + GAD_MUTUAL, o).map_err(Refusal::mem("Gadget.MutualExclude"))?; // &class->cl_Dispatcher == the class
            let t = rd(a, msg + 4, "ops_AttrList")?;
            gadget_set(a, g, t, id)?;
            Ok(g)
        }
        OM_SET | OM_UPDATE => {
            let t = rd(a, msg + 4, "ops_AttrList")?;
            let sup = dispatch_super(a, cl, o, msg)?;
            Ok(sup.wrapping_add(gadget_set(a, o, t, id)?))
        }
        OM_GET => gadget_get(a, o, rd(a, msg + 4, "opg_AttrID")?, rd(a, msg + OPSET_GINFO, "opg_Storage")?),
        OM_DISPOSE => dispatch_super(a, cl, o, msg),
        GM_HITTEST => Ok(GMR_GADGETHIT),
        GM_RENDER | GM_LAYOUT => Ok(1),
        GM_GOACTIVE | GM_HANDLEINPUT => Ok(GMR_NOREUSE),
        GM_HELPTEST => Ok(0xFFFF_FFFF),
        _ => dispatch_super(a, cl, o, msg),
    }
}

/// gadgetclass `set_gadgetclass`: apply the GA_/ICA_ tags to the ExtGadget; non-zero when something visual changed.
fn gadget_set<U: GuestUi>(a: &mut Amiga<U>, g: u32, tag_list: u32, method: u32) -> Result<u32, Refusal> {
    let mut visual = 0;
    let w32 = |a: &mut Amiga<U>, off: u32, v: u32| a.mem.write_u32(g + off, v).map_err(Refusal::mem("Gadget"));
    for (tag, d) in tags::walk(&a.mem, tag_list)? {
        let on = d != 0;
        match tag {
            GA_LEFT => { w16(a, g + GAD_LEFT, d, "Gadget")?; visual = 1 }
            GA_TOP => { w16(a, g + GAD_TOP, d, "Gadget")?; visual = 1 }
            GA_WIDTH => { w16(a, g + GAD_WIDTH, d, "Gadget")?; bit(a, g + GAD_FLAGS, GFLG_RELWIDTH, false)?; visual = 1 }
            GA_HEIGHT => { w16(a, g + GAD_HEIGHT, d, "Gadget")?; bit(a, g + GAD_FLAGS, GFLG_RELHEIGHT, false)?; visual = 1 }
            GA_REL_RIGHT => { w16(a, g + GAD_LEFT, d, "Gadget")?; bit(a, g + GAD_FLAGS, GFLG_RELRIGHT, true)?; visual = 1 }
            GA_REL_BOTTOM => { w16(a, g + GAD_TOP, d, "Gadget")?; bit(a, g + GAD_FLAGS, GFLG_RELBOTTOM, true)?; visual = 1 }
            GA_REL_WIDTH => { w16(a, g + GAD_WIDTH, d, "Gadget")?; bit(a, g + GAD_FLAGS, GFLG_RELWIDTH, true)?; visual = 1 }
            GA_REL_HEIGHT => { w16(a, g + GAD_HEIGHT, d, "Gadget")?; bit(a, g + GAD_FLAGS, GFLG_RELHEIGHT, true)?; visual = 1 }
            GA_REL_SPECIAL => { bit(a, g + GAD_FLAGS, GFLG_RELSPECIAL, on)?; visual = 1 }
            GA_BOUNDS => if on {
                for i in 0..4 { let v = r16(a, d + 2 * i, "IBox")?; w16(a, g + EGAD_BOUNDS + 2 * i, v, "Gadget bounds")?; }
                let f = rd(a, g + EGAD_MORE_FLAGS, "Gadget")? | GMORE_BOUNDS;
                w32(a, EGAD_MORE_FLAGS, f)?; visual = 1;
            },
            GA_GADGET_HELP => {
                let f = rd(a, g + EGAD_MORE_FLAGS, "Gadget")?;
                w32(a, EGAD_MORE_FLAGS, if on { f | GMORE_GADGETHELP } else { f & !GMORE_GADGETHELP })?; visual = 1;
            }
            GA_NEXT => w32(a, GAD_NEXT, d)?,
            GA_PREVIOUS => if on && method == OM_NEW {
                // Link after `previous` once: the tag list reaches gadgetclass again when a subclass re-applies it, and a second
                // pass would make the gadget its own successor.
                let n = rd(a, d + GAD_NEXT, "previous gadget")?;
                if n != g {
                    w32(a, GAD_NEXT, n)?;
                    a.mem.write_u32(d + GAD_NEXT, g).map_err(Refusal::mem("previous gadget"))?;
                }
            },
            GA_INTUI_TEXT | GA_TEXT | GA_LABEL_IMAGE => {
                w32(a, GAD_TEXT, d)?;
                if on {
                    let f = r16(a, g + GAD_FLAGS, "Gadget")? & !GFLG_LABELMASK;
                    let l = match tag { GA_TEXT => GFLG_LABELSTRING, GA_LABEL_IMAGE => GFLG_LABELIMAGE, _ => 0 };
                    w16(a, g + GAD_FLAGS, f | l, "Gadget")?;
                }
                visual = 1;
            }
            GA_IMAGE => { w32(a, GAD_RENDER, d)?; if on { bit(a, g + GAD_FLAGS, GFLG_GADGIMAGE, true)?; } visual = 1 }
            GA_BORDER => { w32(a, GAD_RENDER, d)?; if on { bit(a, g + GAD_FLAGS, GFLG_GADGIMAGE, false)?; } visual = 1 }
            GA_SELECT_RENDER => {
                w32(a, GAD_SELECT, d)?;
                let f = r16(a, g + GAD_FLAGS, "Gadget")? & !GFLG_GADGHIGHBITS;
                w16(a, g + GAD_FLAGS, f | GFLG_GADGIMAGE | GFLG_GADGHIMAGE, "Gadget")?; visual = 1;
            }
            GA_SPECIAL_INFO => w32(a, GAD_SPECIAL, d)?,
            GA_GZZ_GADGET => bit(a, g + GAD_TYPE, GTYP_GZZGADGET, on)?,
            GA_SYS_GADGET => bit(a, g + GAD_TYPE, GTYP_SYSGADGET, on)?,
            GA_SELECTED => { bit(a, g + GAD_FLAGS, GFLG_SELECTED, on)?; visual = 1 }
            GA_DISABLED => { bit(a, g + GAD_FLAGS, GFLG_DISABLED, on)?; visual = 1 }
            GA_END_GADGET => bit(a, g + GAD_ACTIVATION, GACT_ENDGADGET, on)?,
            GA_IMMEDIATE => bit(a, g + GAD_ACTIVATION, GACT_IMMEDIATE, on)?,
            GA_REL_VERIFY => bit(a, g + GAD_ACTIVATION, GACT_RELVERIFY, on)?,
            GA_FOLLOW_MOUSE => bit(a, g + GAD_ACTIVATION, GACT_FOLLOWMOUSE, on)?,
            GA_RIGHT_BORDER => bit(a, g + GAD_ACTIVATION, GACT_RIGHTBORDER, on)?,
            GA_LEFT_BORDER => bit(a, g + GAD_ACTIVATION, GACT_LEFTBORDER, on)?,
            GA_TOP_BORDER => bit(a, g + GAD_ACTIVATION, GACT_TOPBORDER, on)?,
            GA_BOTTOM_BORDER => bit(a, g + GAD_ACTIVATION, GACT_BOTTOMBORDER, on)?,
            GA_TOGGLE_SELECT => bit(a, g + GAD_ACTIVATION, GACT_TOGGLESELECT, on)?,
            GA_TAB_CYCLE => bit(a, g + GAD_FLAGS, GFLG_TABCYCLE, on)?,
            GA_HIGHLIGHT => { let f = r16(a, g + GAD_FLAGS, "Gadget")? & !GFLG_GADGHIGHBITS; w16(a, g + GAD_FLAGS, f | (d & GFLG_GADGHIGHBITS), "Gadget")? }
            GA_SYS_GTYPE => { let f = r16(a, g + GAD_TYPE, "Gadget")? & !GTYP_SYSTYPEMASK; w16(a, g + GAD_TYPE, f | (d & GTYP_SYSTYPEMASK), "Gadget")? }
            GA_ID => if method != OM_UPDATE { w16(a, g + GAD_ID, d, "Gadget")? },
            GA_USER_DATA => w32(a, GAD_USER, d)?,
            ICA_TARGET => w32(a, GDATA_IC + IC_TARGET, d)?,
            ICA_MAP => w32(a, GDATA_IC + IC_MAPPING, d)?,
            _ => {}
        }
    }
    Ok(visual)
}

/// gadgetclass OM_GET: 1 and the value stored for the attributes it knows, else 0.
fn gadget_get<U: GuestUi>(a: &mut Amiga<U>, g: u32, attr: u32, store: u32) -> Result<u32, Refusal> {
    let s16 = |a: &Amiga<U>, off: u32| r16(a, g + off, "Gadget").map(|v| v as i16 as i32 as u32);
    let flag = |a: &Amiga<U>, off: u32, m: u32| r16(a, g + off, "Gadget").map(|v| (v & m != 0) as u32);
    let v = match attr {
        GA_LEFT | GA_REL_RIGHT => s16(a, GAD_LEFT)?, GA_TOP | GA_REL_BOTTOM => s16(a, GAD_TOP)?,
        GA_WIDTH | GA_REL_WIDTH => s16(a, GAD_WIDTH)?, GA_HEIGHT | GA_REL_HEIGHT => s16(a, GAD_HEIGHT)?,
        GA_SELECTED => flag(a, GAD_FLAGS, GFLG_SELECTED)?, GA_DISABLED => flag(a, GAD_FLAGS, GFLG_DISABLED)?,
        GA_ID => r16(a, g + GAD_ID, "Gadget")?, GA_USER_DATA => rd(a, g + GAD_USER, "Gadget")?,
        GA_REL_SPECIAL => flag(a, GAD_FLAGS, GFLG_RELSPECIAL)?, GA_NEXT => rd(a, g + GAD_NEXT, "Gadget")?,
        GA_IMAGE => if r16(a, g + GAD_FLAGS, "Gadget")? & GFLG_GADGIMAGE != 0 { rd(a, g + GAD_RENDER, "Gadget")? } else { 0 },
        GA_BORDER => if r16(a, g + GAD_FLAGS, "Gadget")? & GFLG_GADGIMAGE == 0 { rd(a, g + GAD_RENDER, "Gadget")? } else { 0 },
        GA_SPECIAL_INFO => rd(a, g + GAD_SPECIAL, "Gadget")?,
        GA_END_GADGET => flag(a, GAD_ACTIVATION, GACT_ENDGADGET)?, GA_IMMEDIATE => flag(a, GAD_ACTIVATION, GACT_IMMEDIATE)?,
        GA_REL_VERIFY => flag(a, GAD_ACTIVATION, GACT_RELVERIFY)?, GA_FOLLOW_MOUSE => flag(a, GAD_ACTIVATION, GACT_FOLLOWMOUSE)?,
        GA_TOGGLE_SELECT => flag(a, GAD_ACTIVATION, GACT_TOGGLESELECT)?, GA_TAB_CYCLE => flag(a, GAD_FLAGS, GFLG_TABCYCLE)?,
        GA_TEXT => if r16(a, g + GAD_FLAGS, "Gadget")? & GFLG_LABELSTRING != 0 { rd(a, g + GAD_TEXT, "Gadget")? } else { 0 },
        ICA_TARGET => rd(a, g + GDATA_IC + IC_TARGET, "Gadget")?, ICA_MAP => rd(a, g + GDATA_IC + IC_MAPPING, "Gadget")?,
        _ => return Ok(0),
    };
    a.mem.write_u32(store, v).map_err(Refusal::mem("opg_Storage"))?;
    Ok(1)
}
