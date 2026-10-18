//! gadtools.library onto native controls. A GadTools gadget is a real guest `struct Gadget` (44 bytes,
//! intuition/intuition.h: NextGadget 0, LeftEdge 4, TopEdge 6, Width 8, Height 10, GadgetID 38, UserData 40), chained
//! the way GadTools chains them (CreateContext's dummy first, each CreateGadgetA linked after the previous), plus a
//! HOST-side record of what it is. When a window opens with WA_Gadgets the chain becomes native controls through
//! GuestUi; GT_SetGadgetAttrsA updates them. Kinds bridged: TEXT_KIND (a native label), BUTTON_KIND and (gt_kinds.rs)
//! CHECKBOX / STRING / INTEGER / SLIDER; any other kind is a named refusal and CreateGadgetA returns NULL (which
//! GadTools programs already handle).
//!
//! NewGadget (libraries/gadtools.h): ng_LeftEdge 0, ng_TopEdge 2, ng_Width 4, ng_Height 6, ng_GadgetText 8,
//! ng_TextAttr 12, ng_GadgetID 16, ng_Flags 18, ng_VisualInfo 22, ng_UserData 26.
use alloc::string::String;
use alloc::format;
use crate::{gt_kinds, machine::Amiga, tags, Refusal};
use guestui_abi::{CallbackToken, ControlModel, GuestRect, GuestUi, UiObjectRef};
use retro_m68k::Cpu;

pub const LVO_CREATE_GADGET_A: i32 = -30;
pub const LVO_FREE_GADGETS: i32 = -36;
pub const LVO_GT_SET_GADGET_ATTRS_A: i32 = -42;
pub const LVO_GT_GET_IMSG: i32 = -72;
pub const LVO_GT_REPLY_IMSG: i32 = -78;
pub const LVO_GT_REFRESH_WINDOW: i32 = -84;
pub const LVO_GT_BEGIN_REFRESH: i32 = -90;
pub const LVO_GT_END_REFRESH: i32 = -96;
pub const LVO_CREATE_CONTEXT: i32 = -114;
pub const LVO_GET_VISUAL_INFO_A: i32 = -126;
pub const LVO_FREE_VISUAL_INFO: i32 = -132;

pub(crate) const GADGET_SIZE: u32 = 44;
const VISUAL_INFO_SIZE: u32 = 16;
pub const BUTTON_KIND: u32 = 1;
pub const TEXT_KIND: u32 = 13;
/// The context gadget CreateContext makes: chained, never shown.
pub(crate) const CONTEXT: u32 = u32::MAX;
const GTTX_BORDER: u32 = tags::TAG_USER + 0x0008_0000 + 57;

/// What the host knows about one gadget.
#[derive(Debug, Clone)]
pub struct GadgetInfo { pub kind: u32, pub rect: GuestRect, pub label: String, pub text: String, pub framed: bool, pub attached: Option<(u32, UiObjectRef, UiObjectRef)>, pub state: gt_kinds::KindState }

fn opt_str<U: GuestUi>(a: &Amiga<U>, p: u32, what: &str) -> Result<String, Refusal> {
    if p == 0 { Ok(String::new()) } else { a.mem.read_cstr(p).map_err(Refusal::mem(what)) }
}

pub fn get_visual_info<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let vi = a.heap.alloc(VISUAL_INFO_SIZE).unwrap_or(0);
    cpu.set_d0(vi);
    Ok(format!("[gadtools] GetVisualInfoA screen={:#x} -> {vi:#x}", cpu.areg(0)))
}

pub fn free_visual_info<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let vi = cpu.areg(0);
    if vi != 0 { a.heap.free(vi, VISUAL_INFO_SIZE).map_err(|e| Refusal(format!("FreeVisualInfo: {e}")))?; }
    Ok(format!("[gadtools] FreeVisualInfo {vi:#x}"))
}

fn new_gadget<U: GuestUi>(a: &mut Amiga<U>, info: GadgetInfo) -> Result<u32, Refusal> {
    let g = a.heap.alloc(GADGET_SIZE).ok_or_else(|| Refusal("no memory for a Gadget (NOMEM)".into()))?;
    for i in 0..GADGET_SIZE { a.mem.write_u8(g + i, 0).map_err(Refusal::mem("Gadget"))?; }
    let r = info.rect;
    for (off, v) in [(4, r.left), (6, r.top), (8, r.width), (10, r.height)] { a.mem.write_u16(g + off, v as u16).map_err(Refusal::mem("Gadget geometry"))?; }
    a.gadgets.insert(g, info);
    Ok(g)
}

/// CreateContext(a0 = &glist) -> the context gadget, also stored in *glist.
pub fn create_context<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let slot = cpu.areg(0);
    let g = new_gadget(a, GadgetInfo { kind: CONTEXT, rect: GuestRect { left: 0, top: 0, width: 0, height: 0 }, label: String::new(), text: String::new(), framed: false, attached: None, state: Default::default() })?;
    a.mem.write_u32(slot, g).map_err(Refusal::mem("CreateContext glistptr"))?;
    cpu.set_d0(g);
    Ok(format!("[gadtools] CreateContext -> {g:#x}"))
}

/// CreateGadgetA(d0 = kind, a0 = previous, a1 = NewGadget, a2 = tags) -> the gadget, linked after `previous`; NULL
/// when `previous` is NULL (GadTools' chained-failure rule) or the kind is not bridged.
pub fn create_gadget<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (kind, prev, ng) = (cpu.dreg(0), cpu.areg(0), cpu.areg(1));
    if prev == 0 { cpu.set_d0(0); return Ok(format!("[gadtools] CreateGadgetA kind={kind} previous=NULL -> NULL")); }
    if !gt_kinds::bridged(kind) { cpu.set_d0(0); return Err(gt_kinds::refusal(kind)); }
    let e = Refusal::mem("NewGadget");
    let w = |o| a.mem.read_i16(ng + o).map(i32::from).map_err(&e);
    let rect = GuestRect { left: w(0)?, top: w(2)?, width: w(4)?, height: w(6)? };
    let label = opt_str(a, a.mem.read_u32(ng + 8).map_err(&e)?, "ng_GadgetText")?;
    let id = a.mem.read_u16(ng + 16).map_err(&e)?;
    let user = a.mem.read_u32(ng + 26).map_err(&e)?;
    let t = tags::walk(&a.mem, cpu.areg(2))?;
    let (state, text) = gt_kinds::state_from_tags(a, kind, &t)?;
    let framed = tags::get(&t, GTTX_BORDER).unwrap_or(0) != 0;
    let g = new_gadget(a, GadgetInfo { kind, rect, label: label.clone(), text: text.clone(), framed, attached: None, state })?;
    let made = (|| {
        a.mem.write_u16(g + 38, id).map_err(Refusal::mem("Gadget.GadgetID"))?;
        a.mem.write_u32(g + 40, user).map_err(Refusal::mem("Gadget.UserData"))?;
        gt_kinds::init_guest(a, g)?;
        a.mem.write_u32(prev, g).map_err(Refusal::mem("previous.NextGadget"))
    })();
    if let Err(e) = made { let _ = drop_gadget(a, g); cpu.set_d0(0); return Err(e); }
    cpu.set_d0(g);
    Ok(format!("[gadtools] CreateGadgetA kind={kind} id={id} rect={rect:?} label={label:?} text={text:?} -> {g:#x}"))
}

/// Forget gadget `g` and give back its blocks (the Gadget, a StringInfo, its buffer).
fn drop_gadget<U: GuestUi>(a: &mut Amiga<U>, g: u32) -> Result<(), Refusal> {
    let Some(info) = a.gadgets.remove(&g) else { return Ok(()) };
    for (at, size) in info.state.allocs { a.heap.free(at, size).map_err(|e| Refusal(format!("gadget block: {e}")))?; }
    a.heap.free(g, GADGET_SIZE).map_err(|e| Refusal(format!("gadget: {e}")))
}

/// Attach the chain starting at `first` to native window `native` of guest window `win` (OpenWindow's WA_Gadgets).
pub fn attach<U: GuestUi>(a: &mut Amiga<U>, win: u32, native: UiObjectRef, first: u32) -> Result<(), Refusal> {
    let mut g = first;
    let mut n = 0;
    while g != 0 {
        n += 1;
        if n > 4096 { return Err(Refusal("gadget list longer than 4096 (cyclic NextGadget?)".into())); }
        if let Some(info) = a.gadgets.get(&g).cloned() {
            if info.kind != CONTEXT {
                let model = ControlModel { kind: gt_kinds::control_kind(&info), label: info.label.clone(), rect: info.rect, enabled: !info.state.disabled, token: CallbackToken(g as u64) };
                let ctl = a.ui.add_control(native, model).map_err(Refusal::ui)?;
                if let Some(i) = a.gadgets.get_mut(&g) { i.attached = Some((win, native, ctl)); }
            }
        }
        g = a.mem.read_u32(g).map_err(Refusal::mem("Gadget.NextGadget"))?;
    }
    Ok(())
}

/// The window closed: its controls went with it.
pub fn detach<U: GuestUi>(a: &mut Amiga<U>, win: u32) {
    for i in a.gadgets.values_mut() { if matches!(i.attached, Some((w, _, _)) if w == win) { i.attached = None; } }
}

/// FreeGadgets(a0 = glist): the chain goes back. Freeing gadgets still attached to an open window is refused (on
/// AmigaOS it is a crash: the window would draw freed memory).
pub fn free_gadgets<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let mut g = cpu.areg(0);
    let mut n = 0;
    while g != 0 {
        let next = a.mem.read_u32(g).map_err(Refusal::mem("Gadget.NextGadget"))?;
        match a.gadgets.get(&g) {
            Some(i) if i.attached.is_some() => return Err(Refusal(format!("FreeGadgets: {g:#x} is still in an open window"))),
            Some(_) => { drop_gadget(a, g)?; n += 1; }
            None => {}
        }
        g = next;
    }
    Ok(format!("[gadtools] FreeGadgets ({n} freed)"))
}

/// GT_GetIMsg(a0 = UserPort) / GT_ReplyIMsg(a1 = message): exactly exec's GetMsg / ReplyMsg for the gadgets
/// bridged so far (GadTools only filters messages for kinds that need it: sliders, listviews).
pub fn get_imsg<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> { crate::exec_wait::get_msg(a, cpu) }
pub fn reply_imsg<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> { crate::exec_wait::reply_msg(a, cpu) }

/// GT_RefreshWindow / GT_BeginRefresh / GT_EndRefresh: the native window draws itself; nothing to do.
pub fn refresh<U: GuestUi>(_: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    Ok(format!("[gadtools] refresh {:#x} (native windows draw themselves)", cpu.areg(0)))
}
