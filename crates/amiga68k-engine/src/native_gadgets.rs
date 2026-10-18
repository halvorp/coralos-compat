//! Standard Intuition gadgets as NATIVE controls (Gute Form spike, Peter 2026-10-18: "the Dopus looks like an AmigaOS 3.1
//! screenshot, all the retro apps use the style of the CoralOS UI").
//!
//! The engine used to paint buttongclass / propgclass / strgclass and the sysiclass arrows in the AROS look into the window canvas.
//! It now registers a guestui control at the gadget's rectangle instead and paints nothing there; the host draws it with its own kit.
//! The GUEST-VISIBLE state stays real (PropInfo pots and bodies, StringInfo buffer, Gadget.Flags SELECTED, GadgetRender pointers):
//! it is what the control shows and what host events change ([`crate::native_gadgets_input`]).
//!
//! # Which gadgets become native controls
//! | gadget | control |
//! |---|---|
//! | propgclass, or a classic GTYP_PROPGADGET (one axis) | `ScrollBar` (PGA_Total/Visible/Top, else the pots/bodies on a 0xFFFF scale) |
//! | strgclass, or a classic GTYP_STRGADGET | `Field` (the StringInfo buffer; `integer` = GACT_LONGINT) |
//! | buttongclass whose GA_Image is a sysiclass arrow | `ArrowButton` |
//! | buttongclass framed by a frameiclass image, no action flags, no label | `Group` (style from the frame type: raised / recessed, else plain) |
//! | buttongclass / classic GTYP_BOOLGADGET with GACT_TOGGLESELECT | `ToggleButton` (GFLG_SELECTED) |
//! | classic boolean gadget with nothing to draw (no render, no text) | `Hotspot` |
//! | any other buttongclass / classic boolean gadget | `Button` (label = GA_Text or the first IntuiText) |
//!
//! # When
//! A gadget whose class IS a system class (or a classic gadget) is registered when it is added to a window (AddGList / AddGadget /
//! WA_Gadgets). A GUEST-DEFINED class (DOpus's `dopusstrgclass`) keeps its own GM_RENDER, which draws into the canvas; but when it
//! calls DoSuperMethod(GM_RENDER) into the system class it stands on, THAT call registers or updates the native control (the rule:
//! the super render IS the native control, the subclass's own drawing stays guest-drawn). GM_RENDER of a system class, a re-layout
//! and SetGadgetAttrs on a registered gadget re-read the guest state and tell the host what changed (value, rect).
//!
//! Not natively expressible, named in `Amiga::refused`: a two-axis (FREEHORIZ|FREEVERT) prop gadget and a prop with no freedom.
//! GadTools gadgets are bridged by gadtools.rs / gt_kinds.rs and are not touched here. Native gadgets take their pointer input from
//! the host, so they are never offered GM_HITTEST / GM_GOACTIVE (boopsi_input.rs); a guest subclass that edits input itself is not
//! supported on a natively registered gadget.
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use crate::boopsi_draw::{sysi_which, DOWNIMAGE, LEFTIMAGE, RIGHTIMAGE, UPIMAGE};
use crate::boopsi_ids::*;
use crate::iclass::Native;
use crate::{machine::Amiga, Refusal};
use guestui_abi::{ArrowDir, CallbackToken, ControlKind, ControlModel, GuestRect, GuestUi, Projections};

// struct PropInfo (intuition.h): Flags 0, HorizPot 2, VertPot 4, HorizBody 6, VertBody 8; propgclass appends Total/Visible/Top.
pub(crate) const PI_FLAGS: u32 = 0;
pub(crate) const PI_HPOT: u32 = 2;
pub(crate) const PI_VPOT: u32 = 4;
pub(crate) const PI_HBODY: u32 = 6;
pub(crate) const PI_VBODY: u32 = 8;
pub(crate) const PI_TOTAL: u32 = 24;
pub(crate) const PI_VISIBLE: u32 = 28;
pub(crate) const PI_TOP: u32 = 32;
pub(crate) const FREEHORIZ: u32 = 2;
pub(crate) const FREEVERT: u32 = 4;
const MAXPOT: u32 = 0xFFFF;
// struct StringInfo: Buffer 0, MaxChars 10.
const SI_MAX_CHARS: u32 = 10;
// struct IntuiText: IText 12.
const IT_TEXT: u32 = 12;
// struct Window: Width 8, Height 10, BorderLeft 54 (then Top, Right, Bottom as bytes).
const WD_WIDTH: u32 = 8;
const WD_HEIGHT: u32 = 10;
const WD_BORDER_LEFT: u32 = 54;
const GACT_LONGINT: u32 = crate::boopsi_strg::GACT_LONGINT;

/// The gadgets the engine registered, in the generic mapper's registry (guestui-abi): token = the gadget's address, the
/// engine's own metadata = its guest window. Where the host holds each and what it was last told live there.
pub type NativeGadgets = Projections<u32>;

fn r16<U: GuestUi>(a: &Amiga<U>, at: u32, what: &'static str) -> Result<u32, Refusal> { a.mem.read_u16(at).map(u32::from).map_err(Refusal::mem(what)) }
fn r32<U: GuestUi>(a: &Amiga<U>, at: u32, what: &'static str) -> Result<u32, Refusal> { a.mem.read_u32(at).map_err(Refusal::mem(what)) }
fn opt_str<U: GuestUi>(a: &Amiga<U>, p: u32, what: &'static str) -> Result<String, Refusal> {
    if p == 0 { Ok(String::new()) } else { a.mem.read_cstr(p).map_err(Refusal::mem(what)) }
}

/// The nearest NATIVE ancestor of `g`'s class (the system class it stands on), `None` for a classic gadget or a class with none.
fn system_class<U: GuestUi>(a: &Amiga<U>, g: u32) -> Option<Native> {
    let mut cl = a.classes.class_of(g)?;
    loop {
        let r = a.classes.recs.get(&cl)?;
        if let Some(n) = r.native { return Some(n); }
        cl = r.sup;
    }
}

/// Is `g` the instance of a system class itself (not of a guest subclass)?
fn direct_system<U: GuestUi>(a: &Amiga<U>, g: u32) -> bool {
    a.classes.class_of(g).and_then(|c| a.classes.recs.get(&c)).is_some_and(|r| matches!(r.native, Some(Native::Button | Native::Prop | Native::Strg)))
}

/// The (total, visible, top) of the one free axis of a PropInfo. A propgclass gadget keeps PGA_Total/Visible/Top beside the PropInfo;
/// when those still agree with the pots and bodies they are used, else (and for a classic prop gadget) the pots and bodies are
/// read on a 0xFFFF scale: total = 0xFFFF, visible = body, top = pot * (total - visible) / 0xFFFF.
pub(crate) fn scroll_of<U: GuestUi>(a: &Amiga<U>, pi: u32, propg: bool, vert: bool) -> Result<(u32, u32, u32), Refusal> {
    let (pot, body) = if vert { (r16(a, pi + PI_VPOT, "PropInfo")?, r16(a, pi + PI_VBODY, "PropInfo")?) } else { (r16(a, pi + PI_HPOT, "PropInfo")?, r16(a, pi + PI_HBODY, "PropInfo")?) };
    if propg {
        let (t, v, p) = (r32(a, pi + PI_TOTAL, "PropInfo")?, r32(a, pi + PI_VISIBLE, "PropInfo")?, r32(a, pi + PI_TOP, "PropInfo")?);
        if t != 0 && crate::boopsi_prop::pot_body(t, v, p) == (pot, body) { return Ok((t, v, p.min(t.saturating_sub(v)))); }
    }
    Ok((MAXPOT, body, (pot as u64 * (MAXPOT - body.min(MAXPOT)) as u64 / MAXPOT as u64) as u32))
}

/// The first line of a classic gadget's GadgetText (an IntuiText), or GA_Text's string.
fn label_of<U: GuestUi>(a: &Amiga<U>, g: u32, fl: u32) -> Result<String, Refusal> {
    let text = r32(a, g + GAD_TEXT, "Gadget.GadgetText")?;
    if text == 0 { return Ok(String::new()); }
    if fl & GFLG_LABELSTRING != 0 { return opt_str(a, text, "GA_Text"); }
    if fl & GFLG_LABELMASK != 0 { return Ok(String::new()); } // a label image: no text to show
    opt_str(a, r32(a, text + IT_TEXT, "IntuiText.IText")?, "IntuiText.IText")
}

/// The gadget's box in window coordinates as a guest rect; `None` when it has no area. A BORDER gadget (GACT_RIGHTBORDER ...) whose
/// box has no extent across its border (DOpus's border scrollbars: width -4 / height 0 with the Rel flags; on AROS the window
/// decoration object gives such a gadget the border's thickness) takes the window border's thickness there: the right / left border
/// across x, the bottom / top border across y. The extent ALONG the border stays the gadget's own.
fn rect_of<U: GuestUi>(a: &Amiga<U>, win: u32, g: u32) -> Result<Option<GuestRect>, Refusal> {
    let (mut left, mut top, mut width, mut height) = crate::boopsi_input::gadget_box(a, win, g)?;
    let act = r16(a, g + GAD_ACTIVATION, "Gadget.Activation")?;
    let (ww, wh) = (r16(a, win + WD_WIDTH, "Window.Width")? as i32, r16(a, win + WD_HEIGHT, "Window.Height")? as i32);
    let border = |i: u32| a.mem.read_u8(win + WD_BORDER_LEFT + i).map(i32::from).map_err(Refusal::mem("Window.BorderLeft"));
    if width <= 0 && act & GACT_RIGHTBORDER != 0 { width = border(2)?; left = ww - width; }
    if width <= 0 && act & GACT_LEFTBORDER != 0 { width = border(0)?; left = 0; }
    if height <= 0 && act & GACT_BOTTOMBORDER != 0 { height = border(3)?; top = wh - height; }
    if height <= 0 && act & GACT_TOPBORDER != 0 { height = border(1)?; top = 0; }
    Ok((width > 0 && height > 0).then_some(GuestRect { left, top, width, height }))
}

/// What a gadget is as a native control, or `None` if it is not one of the system look (a guest-drawn custom gadget, a window
/// dragbar, a GadTools gadget). Errors are guest-memory refusals; a gadget that cannot be expressed is named in `a.refused` and
/// yields `None`.
fn plan<U: GuestUi>(a: &mut Amiga<U>, win: u32, g: u32) -> Result<Option<(ControlKind, String, GuestRect, bool)>, Refusal> {
    if a.gadgets.contains_key(&g) { return Ok(None); }
    let (ty, fl, act) = (r16(a, g + GAD_TYPE, "Gadget.GadgetType")?, r16(a, g + GAD_FLAGS, "Gadget.Flags")?, r16(a, g + GAD_ACTIVATION, "Gadget.Activation")?);
    let boopsi = a.classes.class_of(g).is_some();
    let sys = system_class(a, g);
    let classic = !boopsi && ty & GTYP_SYSGADGET == 0 && ty & GTYP_SYSTYPEMASK == 0;
    let (Some(rect), enabled) = (rect_of(a, win, g)?, fl & GFLG_DISABLED == 0) else { return Ok(None) };
    let special = r32(a, g + GAD_SPECIAL, "Gadget.SpecialInfo")?;
    let kind = if matches!(sys, Some(Native::Prop)) || (classic && ty & GTYP_GTYPEMASK == 3) {
        if special == 0 { return Ok(None); }
        let f = r16(a, special + PI_FLAGS, "PropInfo.Flags")?;
        let vert = match (f & FREEHORIZ != 0, f & FREEVERT != 0) {
            (false, true) => true,
            (true, false) => false,
            (h, v) => {
                a.refused.push(format!("native gadget {g:#x}: a prop gadget with FREEHORIZ={h} FREEVERT={v} has no single-axis scrollbar (no native control)"));
                return Ok(None);
            }
        };
        let (total, visible, top) = scroll_of(a, special, sys == Some(Native::Prop), vert)?;
        ControlKind::ScrollBar { vertical: vert, total, visible, top }
    } else if matches!(sys, Some(Native::Strg)) || (classic && ty & GTYP_GTYPEMASK == 4) {
        if special == 0 { return Ok(None); }
        let buf = r32(a, special, "StringInfo.Buffer")?;
        let max = r16(a, special + SI_MAX_CHARS, "StringInfo.MaxChars")?;
        ControlKind::Field { text: opt_str(a, buf, "StringInfo.Buffer")?, max_len: max.saturating_sub(1), editable: true, integer: act & GACT_LONGINT != 0 }
    } else if matches!(sys, Some(Native::Button)) || (classic && ty & GTYP_GTYPEMASK == 1) {
        let render = if fl & GFLG_GADGIMAGE != 0 { r32(a, g + GAD_RENDER, "Gadget.GadgetRender")? } else { 0 };
        let label = label_of(a, g, fl)?;
        let arrow = sysi_which(a, render).and_then(|w| match w { LEFTIMAGE => Some(ArrowDir::Left), UPIMAGE => Some(ArrowDir::Up), RIGHTIMAGE => Some(ArrowDir::Right), DOWNIMAGE => Some(ArrowDir::Down), _ => None });
        let acts = act & (GACT_RELVERIFY | GACT_IMMEDIATE | GACT_TOGGLESELECT) != 0;
        let kind = if let Some(dir) = arrow { ControlKind::ArrowButton { dir } }
            else if let (Some(b), false, true) = (crate::boopsi_frame::bevel_of(a, render), acts, label.is_empty()) { ControlKind::Group { title: String::new(), style: b } }
            else if act & GACT_TOGGLESELECT != 0 { ControlKind::ToggleButton { on: fl & GFLG_SELECTED != 0 } }
            else if !boopsi && label.is_empty() && r32(a, g + GAD_RENDER, "Gadget.GadgetRender")? == 0 { ControlKind::Hotspot }
            else { ControlKind::Button };
        return Ok(Some((kind, label, rect, enabled)));
    } else { return Ok(None) };
    Ok(Some((kind, String::new(), rect, enabled)))
}

/// Register gadget `g` of guest window `win` as a native control, or bring an already registered one up to date (rect, value).
/// A gadget that is not a system-look gadget is left alone. Returns whether `g` is (now) a native control.
pub fn ensure<U: GuestUi>(a: &mut Amiga<U>, win: u32, g: u32) -> Result<bool, Refusal> {
    let Some(&native) = a.windows.get(&win) else { return Ok(false) };
    let Some((kind, label, rect, enabled)) = plan(a, win, g)? else { return Ok(a.native_gads.get(u64::from(g)).is_some()) };
    let model = ControlModel { kind, label, rect, enabled, token: CallbackToken(u64::from(g)) };
    a.native_gads.sync(&mut a.ui, Some(native), u64::from(g), win, Some(model)).map_err(Refusal::ui)?;
    Ok(true)
}

/// GM_RENDER of a system gadget class (`msg` = gpRender): the host paints, the engine registers or refreshes the native control.
pub fn render<U: GuestUi>(a: &mut Amiga<U>, g: u32, msg: u32) -> Result<u32, Refusal> {
    let gi = r32(a, msg + 4, "gpRender.gpr_GInfo")?;
    if gi == 0 { return Ok(0); }
    let win = r32(a, gi + GI_WINDOW, "GadgetInfo.gi_Window")?;
    if win != 0 { ensure(a, win, g)?; }
    Ok(0)
}

/// The guest changed gadget `g` (OM_SET, SetGadgetAttrs): if it is a native control, tell the host what changed.
pub fn resync<U: GuestUi>(a: &mut Amiga<U>, g: u32) -> Result<(), Refusal> {
    match a.native_gads.get(u64::from(g)).map(|n| n.meta) { Some(win) => ensure(a, win, g).map(|_| ()), None => Ok(()) }
}

/// Register every system-look gadget of the chain (AddGList / AddGadget / WA_Gadgets) that stands directly on a system class or is a
/// classic gadget; guest-defined classes register through their super GM_RENDER.
pub fn sync_chain<U: GuestUi>(a: &mut Amiga<U>, win: u32, gadgets: &[u32]) -> Result<(), Refusal> {
    for &g in gadgets {
        if a.classes.class_of(g).is_none() || direct_system(a, g) { ensure(a, win, g)?; }
    }
    Ok(())
}

/// The gadget left its window (RemoveGList / RemoveGadget): its control goes.
pub fn forget<U: GuestUi>(a: &mut Amiga<U>, gadgets: &[u32]) -> Result<(), Refusal> {
    for g in gadgets {
        a.native_gads.retire(&mut a.ui, u64::from(*g)).map_err(Refusal::ui)?;
    }
    Ok(())
}

/// The window closed: its controls went with it.
pub fn detach<U: GuestUi>(a: &mut Amiga<U>, win: u32) { a.native_gads.forget_where(|n| n.meta == win); }

/// Is `g` a propgclass gadget (its PropInfo is followed by PGA_Total / Visible / Top)?
pub(crate) fn is_propgclass<U: GuestUi>(a: &Amiga<U>, g: u32) -> bool { system_class(a, g) == Some(Native::Prop) }

/// The host just changed the control itself: take the guest state it produced as what the host shows now (no call back to the host).
pub(crate) fn absorb<U: GuestUi>(a: &mut Amiga<U>, g: u32) -> Result<(), Refusal> {
    let Some(win) = a.native_gads.get(u64::from(g)).map(|n| n.meta) else { return Ok(()) };
    if let Some((kind, ..)) = plan(a, win, g)? { if let Some(n) = a.native_gads.get_mut(u64::from(g)) { n.model.kind = kind; } }
    Ok(())
}

/// Is `g` a native control (so the guest dispatcher never sees its pointer input)?
pub fn is_native<U: GuestUi>(a: &Amiga<U>, g: u32) -> bool { a.native_gads.get(u64::from(g)).is_some() }

/// The registered gadgets of `win` with the control each is, in gadget address order (tests, diagnostics).
pub fn of_window<U: GuestUi>(a: &Amiga<U>, win: u32) -> Vec<(u32, ControlModel)> {
    a.native_gads.iter().filter(|(_, n)| n.meta == win).map(|(g, n)| (g as u32, n.model.clone())).collect()
}
