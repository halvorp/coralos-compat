//! intuition.library windows: OpenWindow (struct NewWindow), OpenWindowTagList (NewWindow + TagItems), CloseWindow.
//!
//! A window is a REAL guest `struct Window` (136 bytes, intuition/intuition.h; offsets checked by m68k gcc and against
//! the AROS oracle dump tests/oracle/window.bin): programs read LeftEdge/Width, BorderTop, IDCMPFlags and
//! UserPort->mp_SigBit (amimon builds its Wait mask from it). Its UserPort is a real signalling port of the task
//! whenever IDCMP is non-zero. The native window is the GuestUi's; a gadget list passed with WA_Gadgets is attached
//! as native controls (gadtools.rs registers them host-side).
use alloc::string::String;
use alloc::format;
use crate::{idcmp::{self, WD_IDCMP, WD_USERPORT}, machine::Amiga, screen, tags, Refusal};
use guestui_abi::{CallbackToken, GuestRect, GuestUi, WindowModel, WindowRole};
use retro_m68k::Cpu;

pub const LVO_OPEN_WINDOW: i32 = -204;
pub const LVO_CLOSE_WINDOW: i32 = -72;
pub const LVO_OPEN_WINDOW_TAG_LIST: i32 = -606;

pub(crate) const WINDOW_SIZE: u32 = 136;
// struct Window offsets.
const WD_LEFT: u32 = 4;
const WD_TOP: u32 = 6;
const WD_WIDTH: u32 = 8;
const WD_HEIGHT: u32 = 10;
const WD_FLAGS: u32 = 24;
const WD_TITLE: u32 = 32;
const WD_WSCREEN: u32 = 46;
const WD_BORDER_LEFT: u32 = 54;
const WD_GZZ_WIDTH: u32 = 112;
const WD_GZZ_HEIGHT: u32 = 114;
// WFLG_ bits (intuition/intuition.h).
const WFLG_NOCAREREFRESH: u32 = 0x2_0000;
use crate::idcmp::{IDCMP_ACTIVEWINDOW, IDCMP_REFRESHWINDOW};
const WFLG_SIZEGADGET: u32 = 0x1;
const WFLG_DRAGBAR: u32 = 0x2;
const WFLG_DEPTHGADGET: u32 = 0x4;
const WFLG_CLOSEGADGET: u32 = 0x8;
const WFLG_SIMPLE_REFRESH: u32 = 0x40;
const WFLG_BACKDROP: u32 = 0x100;
const WFLG_GIMMEZEROZERO: u32 = 0x400;
const WFLG_BORDERLESS: u32 = 0x800;
const WFLG_ACTIVATE: u32 = 0x1000;
// WA_ tags (intuition/intuition.h, WA_Dummy = TAG_USER + 99).
const WA: u32 = tags::TAG_USER + 99;
const WA_LEFT: u32 = WA + 1;
const WA_TOP: u32 = WA + 2;
const WA_WIDTH: u32 = WA + 3;
const WA_HEIGHT: u32 = WA + 4;
const WA_IDCMP: u32 = WA + 7;
const WA_FLAGS: u32 = WA + 8;
const WA_GADGETS: u32 = WA + 9;
const WA_TITLE: u32 = WA + 11;
const WA_INNER_WIDTH: u32 = WA + 19;
const WA_INNER_HEIGHT: u32 = WA + 20;
/// Boolean tags that each set one WFLG_ bit.
const FLAG_TAGS: [(u32, u32); 9] = [(WA + 30, WFLG_SIZEGADGET), (WA + 31, WFLG_DRAGBAR), (WA + 32, WFLG_DEPTHGADGET),
    (WA + 33, WFLG_CLOSEGADGET), (WA + 34, WFLG_BACKDROP), (WA + 37, WFLG_BORDERLESS), (WA + 38, WFLG_ACTIVATE),
    (WA + 41, WFLG_SIMPLE_REFRESH), (WA + 46, WFLG_GIMMEZEROZERO)];

/// What a window is opened with, from either a NewWindow or tags.
#[derive(Debug, Clone, Default)]
pub struct WindowSpec { pub left: i32, pub top: i32, pub width: i32, pub height: i32, pub inner: (Option<i32>, Option<i32>), pub idcmp: u32, pub flags: u32, pub title: u32, pub gadgets: u32 }

fn read_new_window<U: GuestUi>(a: &Amiga<U>, nw: u32) -> Result<WindowSpec, Refusal> {
    let e = Refusal::mem("NewWindow");
    let w = |o| a.mem.read_i16(nw + o).map(i32::from).map_err(&e);
    Ok(WindowSpec { left: w(0)?, top: w(2)?, width: w(4)?, height: w(6)?, inner: (None, None),
        idcmp: a.mem.read_u32(nw + 10).map_err(&e)?, flags: a.mem.read_u32(nw + 14).map_err(&e)?,
        gadgets: a.mem.read_u32(nw + 18).map_err(&e)?, title: a.mem.read_u32(nw + 26).map_err(&e)? })
}

/// Borders (left, top, right, bottom): the oracle's 4/15/4/4 for a window with a title bar (the screen's WBorTop 6 +
/// font height 8 + 1), the screen's plain 4 otherwise, none when borderless.
fn borders(flags: u32, has_title: bool) -> (i32, i32, i32, i32) {
    if flags & WFLG_BORDERLESS != 0 { return (0, 0, 0, 0); }
    let top = if has_title || flags & (WFLG_DRAGBAR | WFLG_DEPTHGADGET | WFLG_CLOSEGADGET) != 0 { 15 } else { 4 };
    (4, top, 4, 4)
}

/// The one open path: a real struct Window, its UserPort, the native window, its gadgets as native controls.
pub fn open<U: GuestUi>(a: &mut Amiga<U>, s: WindowSpec) -> Result<u32, Refusal> {
    let m = Refusal::mem;
    let title = if s.title == 0 { a.gfx.program_name.clone() } else { a.mem.read_cstr(s.title).map_err(m("Window title"))? };
    let (bl, bt, br, bb) = borders(s.flags, s.title != 0);
    let width = s.inner.0.map(|w| w + bl + br).unwrap_or(s.width);
    let height = s.inner.1.map(|h| h + bt + bb).unwrap_or(s.height);
    if width <= 0 || height <= 0 { return Err(Refusal(format!("OpenWindow: size {width}x{height}"))); }
    let win = a.heap.alloc(WINDOW_SIZE).ok_or_else(|| Refusal("OpenWindow: no memory for struct Window (NOMEM)".into()))?;
    for i in 0..WINDOW_SIZE { a.mem.write_u8(win + i, 0).map_err(m("Window"))?; }
    for (off, v) in [(WD_LEFT, s.left), (WD_TOP, s.top), (WD_WIDTH, width), (WD_HEIGHT, height)] { a.mem.write_u16(win + off, v as u16).map_err(m("Window geometry"))?; }
    a.mem.write_u32(win + WD_FLAGS, s.flags).map_err(m("Window.Flags"))?;
    a.mem.write_u32(win + WD_TITLE, s.title).map_err(m("Window.Title"))?;
    a.mem.write_u32(win + WD_WSCREEN, screen::SCREEN).map_err(m("Window.WScreen"))?;
    for (i, b) in [bl, bt, br, bb].iter().enumerate() { a.mem.write_u8(win + WD_BORDER_LEFT + i as u32, *b as u8).map_err(m("Window borders"))?; }
    // GZZWidth / GZZHeight: the inner size, for every window (intuition.h; AROS sets them in OpenWindow). DOpus sizes its file list from GZZWidth.
    for (off, v) in [(WD_GZZ_WIDTH, width - bl - br), (WD_GZZ_HEIGHT, height - bt - bb)] { a.mem.write_u16(win + off, v as u16).map_err(m("Window GZZ size"))?; }
    a.mem.write_u32(win + WD_IDCMP, s.idcmp).map_err(m("Window.IDCMPFlags"))?;
    idcmp::create_window_port(a, win)?;
    if s.idcmp != 0 { idcmp::create_user_port(a, win)?; }
    let model = WindowModel {
        title: title.clone(),
        rect: GuestRect { left: s.left, top: s.top, width, height },
        role: WindowRole::Document,
        closable: s.flags & WFLG_CLOSEGADGET != 0,
        resizable: s.flags & WFLG_SIZEGADGET != 0,
        token: CallbackToken(win as u64),
    };
    let native = a.ui.open_window(model).map_err(Refusal::ui)?;
    a.windows.insert(win, native);
    crate::graphics::open_window_surface(a, win, (s.left, s.top), (width - bl - br, height - bt - bb), (bl, bt), s.flags & WFLG_GIMMEZEROZERO != 0)?;
    if s.gadgets != 0 {
        crate::gadtools::attach(a, win, native, s.gadgets)?;
        crate::native_gadgets::sync_chain(a, win, &crate::boopsi_glist::chain(a, s.gadgets, -1)?)?;
    }
    // Intuition's first messages to a new window (AROS inputhandler_support.c WindowNeedsRefresh, the activation): IDCMP_ACTIVEWINDOW
    // when it opens active, IDCMP_REFRESHWINDOW when it is a simple-refresh window that cares about refresh. Unsubscribed classes post nothing.
    if s.flags & WFLG_ACTIVATE != 0 { idcmp::post(a, win, IDCMP_ACTIVEWINDOW, 0, win)?; }
    if s.flags & WFLG_SIMPLE_REFRESH != 0 && s.flags & WFLG_NOCAREREFRESH == 0 { crate::graphics::mark_damaged(a, win)?; idcmp::post(a, win, IDCMP_REFRESHWINDOW, 0, win)?; }
    Ok(win)
}

/// A `struct Window` for a window the MUI host opened itself (mui/surface.rs): the same 4/15/4/4 borders, geometry and screen as
/// [`open`], no IDCMP port and no gadgets, registered under `native`, with the graphics layer's surface (canvas, layer, RPort). Its
/// RPort draws in window coordinates, which is what a MUI object's `_left(obj)` / `_rp(obj)` mean.
pub(crate) fn make_surface_window<U: GuestUi>(a: &mut Amiga<U>, native: guestui_abi::UiObjectRef, (iw, ih): (i32, i32)) -> Result<u32, Refusal> {
    let m = Refusal::mem;
    let (bl, bt, br, bb) = borders(0, true);
    let win = a.heap.alloc(WINDOW_SIZE).ok_or_else(|| Refusal("MUI window surface: no memory for struct Window (NOMEM)".into()))?;
    for i in 0..WINDOW_SIZE { a.mem.write_u8(win + i, 0).map_err(m("Window"))?; }
    for (off, v) in [(WD_WIDTH, iw + bl + br), (WD_HEIGHT, ih + bt + bb)] { a.mem.write_u16(win + off, v as u16).map_err(m("Window geometry"))?; }
    a.mem.write_u32(win + WD_WSCREEN, screen::SCREEN).map_err(m("Window.WScreen"))?;
    for (i, b) in [bl, bt, br, bb].iter().enumerate() { a.mem.write_u8(win + WD_BORDER_LEFT + i as u32, *b as u8).map_err(m("Window borders"))?; }
    for (off, v) in [(WD_GZZ_WIDTH, iw), (WD_GZZ_HEIGHT, ih)] { a.mem.write_u16(win + off, v as u16).map_err(m("Window GZZ size"))?; }
    a.windows.insert(win, native);
    crate::graphics::open_window_surface(a, win, (0, 0), (iw, ih), (bl, bt), false)?;
    Ok(win)
}

/// Give back what [`make_surface_window`] made.
pub(crate) fn drop_surface_window<U: GuestUi>(a: &mut Amiga<U>, win: u32) -> Result<(), Refusal> {
    crate::graphics::close_window_surface(a, win)?;
    a.windows.remove(&win);
    a.heap.free(win, WINDOW_SIZE).map_err(|e| Refusal(format!("MUI window surface: {e}")))
}

/// OpenWindow(a0 = NewWindow).
pub fn open_window<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let s = read_new_window(a, cpu.areg(0))?;
    let (l, t, w, h) = (s.left, s.top, s.width, s.height);
    let title = if s.title == 0 { String::new() } else { a.mem.read_cstr(s.title).map_err(Refusal::mem("NewWindow.Title"))? };
    let win = open(a, s)?;
    cpu.set_d0(win);
    Ok(format!("[intuition] OpenWindow left={l} top={t} w={w} h={h} title=\"{title}\" -> {win:#x}"))
}

/// OpenWindowTagList(a0 = NewWindow or NULL, a1 = tags): the tags override the NewWindow.
pub fn open_window_tag_list<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let mut s = if cpu.areg(0) == 0 { WindowSpec { width: 640, height: 256, ..Default::default() } } else { read_new_window(a, cpu.areg(0))? };
    let t = tags::walk(&a.mem, cpu.areg(1))?;
    for &(tag, d) in &t {
        match tag {
            WA_LEFT => s.left = d as i32, WA_TOP => s.top = d as i32, WA_WIDTH => s.width = d as i32, WA_HEIGHT => s.height = d as i32,
            WA_INNER_WIDTH => s.inner.0 = Some(d as i32), WA_INNER_HEIGHT => s.inner.1 = Some(d as i32),
            WA_IDCMP => s.idcmp = d, WA_FLAGS => s.flags |= d, WA_GADGETS => s.gadgets = d, WA_TITLE => s.title = d,
            _ => if let Some(&(_, bit)) = FLAG_TAGS.iter().find(|(f, _)| *f == tag) { if d != 0 { s.flags |= bit } else { s.flags &= !bit } },
        }
    }
    let shown = format!("left={} top={} inner={:?} idcmp={:#x} flags={:#x}", s.left, s.top, s.inner, s.idcmp, s.flags);
    let win = open(a, s)?;
    cpu.set_d0(win);
    Ok(format!("[intuition] OpenWindowTagList {shown} -> {win:#x}"))
}

/// CloseWindow(a0 = window): the native window closes; the UserPort, its signal and the struct go back.
pub fn close_window<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let win = cpu.areg(0);
    let native = *a.windows.get(&win).ok_or_else(|| Refusal(format!("CloseWindow: {win:#x} is not an open window")))?;
    let port = a.mem.read_u32(win + WD_USERPORT).map_err(Refusal::mem("Window.UserPort"))?;
    if port != 0 { a.drop_window_msgs(win, port)?; }
    a.windows.remove(&win);
    crate::graphics::release_damage(a, win)?; // a DamageList left by mark_damaged that the program never refreshed
    crate::graphics::close_window_surface(a, win)?;
    a.ui.close_window(native).map_err(Refusal::ui)?;
    crate::gadtools::detach(a, win);
    crate::native_gadgets::detach(a, win);
    idcmp::free_user_port(a, win)?;
    idcmp::free_window_port(a, win)?;
    a.menu_strips.remove(&win);
    a.heap.free(win, WINDOW_SIZE).map_err(|e| Refusal(format!("CloseWindow: {e}")))?;
    Ok(format!("[intuition] CloseWindow {win:#x}"))
}
