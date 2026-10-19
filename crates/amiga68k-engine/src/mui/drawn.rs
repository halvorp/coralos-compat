//! Objects that DRAW THEMSELVES: an instance of a guest class (a disk `.mcc`, or a program's own class) over Area.mui that has no role
//! of its own (ruling 17 piece 4: nothing else claims it, so it is a content PLANE). MUI sends such an object the methods a custom
//! class implements, in Zune's order (window.c open: `MUIM_Setup` for the whole tree, `MUIM_AskMinMax`, layout, `MUIM_Show`, then
//! `MUIM_Draw` with `MADF_DRAWOBJECT`; close: `MUIM_Hide`, `MUIM_Cleanup`; `MUI_Redraw` -> `MUIM_Draw`). They are sent as GUEST calls to the
//! object's class dispatcher, queued like a notification's hook ([`super::hooks`]), so the program sees the window opened (and drawn)
//! before `SetAttrs(win, MUIA_Window_Open, TRUE)` returns. The drawing goes to a real RastPort (the window's own, through the graphics
//! layer's surface: [`surface`]), so the engine records it as a replayed plane in the object's rectangle; nothing is copied as pixels.
//!
//! What the NATIVE Area half answers when the guest's dispatcher passes a method up with `DoSuperMethod` (area.c): AskMinMax = the
//! frame/inner-spacing sums (none are drawn here, so 0), Setup stores `mad_RenderInfo`, Show/Hide the CANDRAW flag, Draw and DrawBackground
//! draw nothing (the plane's surface is the host's own background). NOT done: frames and inner spacing, clipping regions
//! (`MUI_AddClipRegion` is a named refusal), `MUIM_HandleEvent` (input), `MADF_DRAWUPDATE` partial redraws.
//! Facts: Zune's area.c (the methods above, `struct MUIP_AskMinMax { MethodID; struct MUI_MinMax *MinMaxInfo }`, `MUIP_Setup { MethodID;
//! struct MUI_RenderInfo *RenderInfo }`, `MUIP_Draw { MethodID; ULONG flags }`), window.c, classes/window.h `struct MUI_RenderInfo`.
use alloc::boxed::Box;
use alloc::format;
use alloc::vec;
use alloc::vec::Vec;
use super::classes::Kind;
use super::ids_gen::*;
use super::{mirror, roles};
use crate::boopsi_ids::CL_H_ENTRY;
use crate::guest_call::Next;
use crate::{machine::Amiga, Refusal};
use guestui_abi::{GuestRect, GuestUi};
use retro_m68k::Cpu;

/// `MADF_` bits the engine sets (classes/area.h): drawn completely / setup done / may draw / shown.
pub const MADF_DRAWOBJECT: u32 = 1 << 0;
const MADF_SETUP: u32 = 1 << 28;
const MADF_CANDRAW: u32 = 1 << 14;
const MADF_SHOWME: u32 = 1 << 9;
const MUI_MAXMAX: i32 = 10000;
/// `struct MUI_RenderInfo` offsets (window.h): mri_WindowObject 0, mri_Screen 4, mri_DrawInfo 8, mri_Pens 12, mri_Window 16, mri_RastPort 20.
const MRI_WINDOW_OBJECT: u32 = 0;
const MRI_SCREEN: u32 = 4;
const MRI_PENS: u32 = 12;
const MRI_WINDOW: u32 = 16;
const MRI_RASTPORT: u32 = 20;
/// Room for the whole structure (its private tail holds fonts, clip regions and a buffer RastPort; none is used here).
const MRI_SIZE: u32 = 512;
/// mri_PensStorage lives inside the structure; the pens are `MPEN_COUNT` UWORDs.
const MRI_PENS_STORAGE: u32 = 40;
/// `struct MUI_AreaData` offsets used here (mirror.rs has the rest): MinMax 12, Flags 36.
const MAD_RENDER_INFO: u32 = 0;
const MAD_MINMAX: u32 = 12;
const MAD_FLAGS: u32 = 36;
/// intuition DrawInfo pens the MUI pens come from (intuition/screens.h) and the borders the surface window has (window.rs).
const SHINEPEN: usize = 3;
const SHADOWPEN: usize = 4;
const TEXTPEN: usize = 2;
const FILLPEN: usize = 5;
const BACKGROUNDPEN: usize = 7;
const HIGHLIGHTTEXTPEN: usize = 8;

/// What the engine knows of one self-drawing object.
#[derive(Debug, Clone, Default)]
pub struct Drawn {
    /// What its `MUIM_AskMinMax` answered: (min, default, max) width and height.
    pub min: (i32, i32), pub def: (i32, i32), pub max: (i32, i32),
    /// Where layout put it, in window coordinates (the window's own origin, borders included).
    pub rect: Option<GuestRect>,
}

/// One queued step of opening, drawing or closing a window that holds self-drawing objects.
#[derive(Debug, Clone, Copy)]
pub enum Step {
    Setup(u32), AskMinMax(u32), Open(u32), Show(u32), Draw(u32, u32), Hide(u32), Cleanup(u32), Close(u32),
    /// `MUIM_HandleEvent` for one event-handler node (handlers.rs); `last` = the last node that will see this message.
    Handle { imsg: u32, ehn: u32, last: bool },
}

/// A window's drawing surface: the `struct Window` the graphics layer draws into, and the `struct MUI_RenderInfo` its objects share.
#[derive(Debug, Clone, Copy)]
pub struct Surface { pub win: u32, pub ri: u32 }

/// Is `obj` an instance of a guest class that nothing else gives a role (so it draws itself)?
pub fn is_drawn_candidate<U: GuestUi>(a: &Amiga<U>, obj: u32) -> bool {
    let Some(cl) = a.classes.class_of(obj) else { return false };
    let guest = a.classes.recs.get(&cl).is_some_and(|r| r.native.is_none());
    guest && matches!(a.objects.kind(obj), Some(Kind::Area | Kind::Rectangle)) && roles::leaf(&a.objects, obj).is_none() && roles::shown(&a.objects, obj)
}

/// The self-drawing objects in the tree of window `w`, in tree order.
pub fn candidates<U: GuestUi>(a: &Amiga<U>, w: u32) -> Vec<u32> {
    fn walk<U: GuestUi>(a: &Amiga<U>, o: u32, out: &mut Vec<u32>) {
        if is_drawn_candidate(a, o) { out.push(o); }
        for &c in a.objects.children(o) { walk(a, c, out); }
    }
    let mut out = Vec::new();
    walk(a, w, &mut out);
    out
}

/// `MUIA_Window_Open` TRUE on `w`: when the window holds self-drawing objects, queue the methods Zune sends and report that the
/// window opens when they have run (`true`); otherwise the caller opens it at once.
pub fn begin_open<U: GuestUi>(a: &mut Amiga<U>, w: u32) -> bool {
    let c = candidates(a, w);
    if c.is_empty() { return false; }
    let steps = c.iter().map(|&o| Step::Setup(o)).chain(c.iter().map(|&o| Step::AskMinMax(o))).chain([Step::Open(w)]);
    a.objects.draw_steps.extend(steps);
    true
}

/// `MUIA_Window_Open` FALSE on `w` (open, with self-drawing objects): Hide and Cleanup them, then close.
pub fn begin_close<U: GuestUi>(a: &mut Amiga<U>, w: u32) -> bool {
    let c = candidates(a, w).into_iter().filter(|o| a.objects.drawn.contains_key(o)).collect::<Vec<_>>();
    if c.is_empty() { return false; }
    a.objects.draw_steps.extend(c.iter().map(|&o| Step::Hide(o)).chain(c.iter().map(|&o| Step::Cleanup(o))).chain([Step::Close(w)]));
    true
}

/// `MUI_Redraw(obj, flags)` / a layout change: draw `obj` again.
pub fn redraw<U: GuestUi>(a: &mut Amiga<U>, obj: u32, flags: u32) { if a.objects.drawn.get(&obj).is_some_and(|d| d.rect.is_some()) { a.objects.draw_steps.push_back(Step::Draw(obj, flags)); } }

/// Run the next queued step (called from [`super::hooks::run`]): a guest call, or `Ok(None)` when the queue is empty.
pub fn run_next<U: GuestUi>(a: &mut Amiga<U>, d0: u32) -> Result<Option<Next<U>>, Refusal> {
    if a.objects.draw_busy { return Ok(None); } // a step is in flight: its own continuation goes on with the queue
    loop {
        let Some(step) = a.objects.draw_steps.pop_front() else { return Ok(None) };
        a.log.push(format!("[muimaster] {step:?}"));
        match step {
            Step::Open(w) => { open(a, w)?; }
            Step::Close(w) => { super::win::close_if_open(a, w)?; }
            Step::Setup(o) => { let ri = ri_of(a, o)?; return method(a, o, &[MUIM_Setup, ri], d0, Some(Step::Setup(o))); }
            Step::AskMinMax(o) => return ask_min_max(a, o, d0).map(Some),
            Step::Show(o) => return method(a, o, &[MUIM_Show], d0, Some(Step::Show(o))),
            Step::Draw(o, flags) => return method(a, o, &[MUIM_Draw, flags], d0, Some(Step::Draw(o, flags))),
            Step::Hide(o) => return method(a, o, &[MUIM_Hide], d0, Some(Step::Hide(o))),
            Step::Cleanup(o) => return method(a, o, &[MUIM_Cleanup], d0, Some(Step::Cleanup(o))),
            Step::Handle { imsg, ehn, last } => { if let Some(n) = super::handlers::handle_step(a, imsg, ehn, last, d0)? { return Ok(Some(n)); } }
        }
    }
}

fn window_of<U: GuestUi>(a: &Amiga<U>, obj: u32) -> Option<u32> { super::object::ancestor(&a.objects, obj, Kind::Window) }

/// The window's surface (the `struct Window` + RenderInfo), made on first need.
fn ri_of<U: GuestUi>(a: &mut Amiga<U>, obj: u32) -> Result<u32, Refusal> {
    let w = window_of(a, obj).ok_or_else(|| Refusal(format!("MUIM_Setup: {obj:#x} is not in a window")))?;
    Ok(super::surface::render_info(a, w)?)
}

/// Send `words` (a method id and its parameters) to `obj`'s own class dispatcher as a guest call; afterwards the step's effect on the
/// mirror is applied and the queue goes on.
fn method<U: GuestUi>(a: &mut Amiga<U>, obj: u32, words: &[u32], d0: u32, step: Option<Step>) -> Result<Option<Next<U>>, Refusal> {
    let cl = a.classes.class_of(obj).ok_or_else(|| Refusal(format!("{obj:#x} has no guest class to send {} to", super::attrs::method_name(words[0]))))?;
    if let Some(s) = step { before(a, obj, s)?; }
    call(a, cl, obj, words, d0, Box::new(move |a: &mut Amiga<U>, r: u32| match step { Some(s) => after(a, obj, s, r), None => Ok(()) }))
}

/// A guest call of class `cl`'s dispatcher with `words` as the message and `obj` as the object (`CoerceMethod`, when `cl` is not the
/// object's own class); `done` sees the dispatcher's D0, then the queue goes on.
pub(super) fn call<U: GuestUi>(a: &mut Amiga<U>, cl: u32, obj: u32, words: &[u32], d0: u32, done: Box<dyn FnOnce(&mut Amiga<U>, u32) -> Result<(), Refusal>>) -> Result<Option<Next<U>>, Refusal> {
    let entry = a.mem.read_u32(cl + CL_H_ENTRY).map_err(Refusal::mem("cl_Dispatcher.h_Entry"))?;
    let msg = crate::boopsi_send::make_msg(a, words)?;
    a.objects.draw_busy = true;
    Ok(Some(Next::Call { target: entry, d0: 0, aregs: vec![(0, cl), (1, msg), (2, obj)], then: Box::new(move |a: &mut Amiga<U>, _cpu: &mut Cpu, r: u32| {
        a.objects.draw_busy = false;
        a.heap.free(msg, crate::boopsi_send::MSG_BLOCK).map_err(|e| Refusal(format!("BOOPSI message: {e}")))?;
        done(a, r)?;
        super::hooks::run(a, d0)
    }) }))
}

/// May `obj` draw (`MADF_CANDRAW`) and is it shown (`MADF_SHOWME`)?
pub(super) fn can_draw<U: GuestUi>(a: &Amiga<U>, obj: u32) -> bool {
    mirror::area_at(a, obj, MAD_FLAGS).and_then(|at| a.mem.read_u32(at).ok()).is_some_and(|f| f & MADF_CANDRAW != 0 && f & MADF_SHOWME != 0)
}

/// The mirror before a method is sent.
fn before<U: GuestUi>(a: &mut Amiga<U>, obj: u32, step: Step) -> Result<(), Refusal> {
    match step {
        Step::Show(_) | Step::Draw(..) => mirror::area_geometry(a, obj),
        _ => Ok(()),
    }
}

fn after<U: GuestUi>(a: &mut Amiga<U>, obj: u32, step: Step, r: u32) -> Result<(), Refusal> {
    match step {
        // A Setup that fails (FALSE) leaves the object unusable: Zune closes the window; here the object is not drawn and the refusal names it.
        Step::Setup(_) if r == 0 => Err(Refusal(format!("MUIM_Setup of {obj:#x} ({}) returned FALSE", a.classes.name_of(a.classes.class_of(obj).unwrap_or(0))))),
        _ => Ok(()),
    }
}

fn ask_min_max<U: GuestUi>(a: &mut Amiga<U>, obj: u32, d0: u32) -> Result<Next<U>, Refusal> {
    let mm = obj + mirror::MAD + MAD_MINMAX;
    a.mem.zero(mm, 12).map_err(Refusal::mem("mad_MinMax"))?;
    let next = method(a, obj, &[MUIM_AskMinMax, mm], d0, None)?.expect("a method call");
    // The answer is read when the guest returned: wrap the continuation.
    let Next::Call { target, d0: dd, aregs, then } = next else { unreachable!("method() always starts a call") };
    Ok(Next::Call { target, d0: dd, aregs, then: Box::new(move |a: &mut Amiga<U>, cpu: &mut Cpu, r: u32| {
        let w = |i: u32| a.mem.read_i16(mm + 2 * i).map(i32::from).map_err(Refusal::mem("mad_MinMax"));
        let v = [w(0)?, w(1)?, w(2)?, w(3)?, w(4)?, w(5)?];
        let (min, max, def) = ((v[0], v[1]), ((v[2]).min(MUI_MAXMAX), v[3].min(MUI_MAXMAX)), (v[4], v[5]));
        a.objects.drawn.insert(obj, Drawn { min, def: (def.0.max(min.0), def.1.max(min.1)), max, rect: None });
        then(a, cpu, r)
    }) })
}

/// Open the native window with the sizes the objects answered, give the objects their boxes, and queue Show and Draw.
fn open<U: GuestUi>(a: &mut Amiga<U>, w: u32) -> Result<(), Refusal> {
    super::win::open_window(a, w)?;
    let c: Vec<u32> = candidates(a, w).into_iter().filter(|o| a.objects.drawn.get(o).is_some_and(|d| d.rect.is_some())).collect();
    let front = c.iter().map(|&o| Step::Show(o)).chain(c.iter().map(|&o| Step::Draw(o, MADF_DRAWOBJECT))).collect::<Vec<_>>();
    for s in front.into_iter().rev() { a.objects.draw_steps.push_front(s); }
    Ok(())
}

/// The native Area half of a method the guest's dispatcher passed up with `DoSuperMethod` (area.c). `None` = not one of these.
pub fn area_method<U: GuestUi>(a: &mut Amiga<U>, obj: u32, id: u32, msg: u32) -> Result<Option<u32>, Refusal> {
    if !a.objects.kind(obj).is_some_and(|k| k.is_area()) { return Ok(None); }
    let rd = |a: &Amiga<U>, off: u32| a.mem.read_u32(msg + off).map_err(Refusal::mem("method message"));
    Ok(Some(match id {
        MUIM_AskMinMax => {
            // Area__MUIM_AskMinMax: Min = Max = Def = the frame and inner-spacing sums, none here.
            let mm = rd(a, 4)?;
            a.mem.zero(mm, 12).map_err(Refusal::mem("MUI_MinMax"))?;
            1
        }
        MUIM_Setup => { let ri = rd(a, 4)?; mirror::set_area(a, obj, MAD_RENDER_INFO, ri)?; flag(a, obj, MADF_SETUP | MADF_SHOWME, true)?; 1 }
        MUIM_Show => { flag(a, obj, MADF_CANDRAW, true)?; 1 }
        MUIM_Hide => { flag(a, obj, MADF_CANDRAW, false)?; 0 }
        MUIM_Cleanup => { mirror::set_area(a, obj, MAD_RENDER_INFO, 0)?; flag(a, obj, MADF_SETUP, false)?; 0 }
        // The plane's surface is the host's own background; Zune's Area draws the background and frame here.
        MUIM_Draw | MUIM_DrawBackground => 0,
        _ => return Ok(None),
    }))
}

fn flag<U: GuestUi>(a: &mut Amiga<U>, obj: u32, bits: u32, on: bool) -> Result<(), Refusal> {
    let Some(at) = mirror::area_at(a, obj, MAD_FLAGS) else { return Ok(()) };
    let f = a.mem.read_u32(at).map_err(Refusal::mem("mad_Flags"))?;
    a.mem.write_u32(at, if on { f | bits } else { f & !bits }).map_err(Refusal::mem("mad_Flags"))
}

/// The MUI pens of the window's surface (classes/window.h MPEN_*: SHINE, HALFSHINE, BACKGROUND, HALFSHADOW, SHADOW, TEXT, FILL, MARK),
/// as palette pens through the DrawInfo table the engine installs.
pub fn pens() -> [u16; 8] {
    use retro_theme::amiga::DRI_PENS as D;
    [D[SHINEPEN], D[SHINEPEN], D[BACKGROUNDPEN], D[SHADOWPEN], D[SHADOWPEN], D[TEXTPEN], D[FILLPEN], D[HIGHLIGHTTEXTPEN]]
}

pub(super) const RI: (u32, u32, u32, u32, u32, u32, u32) = (MRI_WINDOW_OBJECT, MRI_SCREEN, MRI_PENS, MRI_WINDOW, MRI_RASTPORT, MRI_SIZE, MRI_PENS_STORAGE);
