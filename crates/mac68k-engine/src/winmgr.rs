//! More of the Window Manager (Inside Macintosh I, "The Window Manager"), over the same [`crate::machine::MacWindow`] the
//! basics in [`crate::toolbox`] keep: `SetWTitle`/`GetWTitle`, `SetWRefCon`/`GetWRefCon`, `MoveWindow`, `SizeWindow`, and the
//! Color Window Manager's `NewCWindow`/`GetNewCWindow`.
//!
//! * A title change goes to the native window (`set_window_title`); the WindowRecord's `refCon` (offset 152) is real guest memory.
//! * `SizeWindow`/`MoveWindow` change the content rectangle the guest sees (`portRect`, `portBits.bounds`, FindWindow); the
//!   canvas is rebuilt and the program gets an `updateEvt` (`SizeWindow(.., fUpdate = TRUE)` and any growth). The seam has no
//!   call to resize or move a native window, so the native window keeps the size the user gave it: a named limit, logged.
//! * A colour window (`NewCWindow`/`GetNewCWindow`) takes the SAME native window path as `NewWindow`: the engine's QuickDraw is
//!   1 bit deep, so the window's port is a plain `GrafPort` (`SysEnvirons` says `hasColorQD` FALSE); a `WIND` resource is read
//!   exactly as `GetNewWindow` does. A program that treats the port as a `CGrafPort` is not served by this.
use crate::events::UPDATE_EVT;
use crate::machine::{Call, Done, Mac};
use crate::pascal::{read_pstring, read_rect, write_pstring, Arg, Rect};
use crate::toolbox::{WindowSpec, WR_REF_CON};
use crate::Refusal;
use alloc::format;
use alloc::string::String;
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    m.register(0xA91A, &[Arg::P, Arg::P], None, set_w_title::<U>)?;
    m.register(0xA919, &[Arg::P, Arg::P], None, get_w_title::<U>)?;
    m.register(0xA918, &[Arg::P, Arg::L], None, set_w_ref_con::<U>)?;
    m.register(0xA917, &[Arg::P], Some(Arg::L), get_w_ref_con::<U>)?;
    m.register(0xA91B, &[Arg::P, Arg::W, Arg::W, Arg::B], None, move_window::<U>)?;
    m.register(0xA91D, &[Arg::P, Arg::W, Arg::W, Arg::B], None, size_window::<U>)?;
    m.register(0xAA45, &[Arg::L, Arg::L, Arg::L, Arg::B, Arg::W, Arg::L, Arg::B, Arg::L], Some(Arg::L), new_c_window::<U>)?;
    m.register(0xAA46, &[Arg::W, Arg::L, Arg::L], Some(Arg::L), get_new_c_window::<U>)?;
    m.register(0xA920, &[Arg::P], None, bring_to_front::<U>)?;
    m.register(0xA921, &[Arg::P, Arg::P], None, send_behind::<U>)?;
    m.register(0xA904, &[Arg::P], None, draw_grow_icon::<U>)?;
    m.register(0xA91C, &[Arg::P, Arg::B], None, hilite_window::<U>)?;
    m.register(0xA92D, &[Arg::P], None, close_window::<U>)
}

/// `BringToFront(window)`: front of the stacking order, with no activate events (that is `SelectWindow`'s job).
fn bring_to_front<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let win = c.args.get(0);
    m.open_win("BringToFront", win)?;
    m.z.retain(|&z| z != win);
    m.z.insert(0, win);
    m.relink_windows()?;
    Ok(Done { result: 0, log: format!("[toolbox] BringToFront {win:#x}") })
}

/// `SendBehind(window, behindWindow)`: `window` goes just behind `behindWindow` (NIL = the very back). The host's own stacking is
/// the user's, so only the program's view (`WindowList`, `FrontWindow`) changes.
fn send_behind<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (win, behind) = (c.args.get(0), c.args.get(1));
    m.open_win("SendBehind", win)?;
    m.z.retain(|&z| z != win);
    let at = if behind == 0 { m.z.len() } else { m.z.iter().position(|&z| z == behind).map(|i| i + 1).ok_or_else(|| Refusal(format!("SendBehind: {behind:#x} is not an open window")))? };
    m.z.insert(at, win);
    m.relink_windows()?;
    Ok(Done { result: 0, log: format!("[toolbox] SendBehind {win:#x} behind {behind:#x}") })
}

/// `DrawGrowIcon(window)`: the grow box and scroll-bar corner lines are the host window's own chrome, not drawn into the port.
fn draw_grow_icon<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let win = c.args.get(0);
    m.open_win("DrawGrowIcon", win)?;
    Ok(Done { result: 0, log: format!("[toolbox] DrawGrowIcon {win:#x} (the host window draws its own grow affordance)") })
}

/// `HiliteWindow(window, fHilite)`: records `hilited` in the WindowRecord; the host shows the active window itself.
fn hilite_window<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (win, on) = (c.args.get(0), c.args.get(1) as u8 != 0);
    m.open_win("HiliteWindow", win)?;
    m.mem.write_u8(win + crate::toolbox::WR_HILITED, u8::from(on)).map_err(Refusal::mem("WindowRecord.hilited"))?;
    Ok(Done { result: 0, log: format!("[toolbox] HiliteWindow {win:#x} {on}") })
}

/// `CloseWindow(window)`: take the window away; the record stays the program's unless the engine allocated it.
fn close_window<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let win = c.args.get(0);
    m.dispose_window(win)?;
    Ok(Done { result: 0, log: format!("[toolbox] CloseWindow {win:#x}") })
}

impl<U: GuestUi> Mac<U> {
    fn open_win(&self, what: &str, win: u32) -> Result<(), Refusal> {
        if self.windows.contains_key(&win) { Ok(()) } else { Err(Refusal(format!("{what}: {win:#x} is not an open window"))) }
    }
}

fn set_w_title<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (win, title) = (c.args.get(0), read_pstring(&m.mem, c.args.get(1))?);
    m.open_win("SetWTitle", win)?;
    let native = {
        let w = m.windows.get_mut(&win).ok_or_else(|| Refusal(format!("SetWTitle: {win:#x} vanished")))?;
        w.title = title.clone();
        w.native
    };
    if let Some(n) = native { m.ui.set_window_title(n, title.clone()).map_err(Refusal::ui)?; }
    Ok(Done { result: 0, log: format!("[toolbox] SetWTitle {win:#x} \"{title}\"") })
}

fn get_w_title<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (win, at) = (c.args.get(0), c.args.get(1));
    m.open_win("GetWTitle", win)?;
    let title: String = m.windows[&win].title.clone();
    let bytes: alloc::vec::Vec<u8> = title.chars().map(|ch| ch as u32 as u8).take(255).collect();
    write_pstring(&mut m.mem, at, &bytes)?;
    Ok(Done { result: 0, log: format!("[toolbox] GetWTitle {win:#x} -> \"{title}\"") })
}

fn set_w_ref_con<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (win, v) = (c.args.get(0), c.args.get(1));
    m.open_win("SetWRefCon", win)?;
    m.mem.write_u32(win + WR_REF_CON, v).map_err(Refusal::mem("WindowRecord.refCon"))?;
    Ok(Done { result: 0, log: format!("[toolbox] SetWRefCon {win:#x} {v:#x}") })
}

fn get_w_ref_con<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let win = c.args.get(0);
    m.open_win("GetWRefCon", win)?;
    let v = m.mem.read_u32(win + WR_REF_CON).map_err(Refusal::mem("WindowRecord.refCon"))?;
    Ok(Done { result: v, log: format!("[toolbox] GetWRefCon {win:#x} -> {v:#x}") })
}

/// `MoveWindow(window, h, v, front)`: the content's top-left becomes the global (h, v); `front` also selects the window.
fn move_window<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (win, h, v, front) = (c.args.get(0), c.args.get(1) as i16, c.args.get(2) as i16, c.args.get(3) as u8 != 0);
    m.open_win("MoveWindow", win)?;
    let r = m.windows[&win].rect;
    m.set_window_rect(win, Rect { top: v, left: h, bottom: v + (r.bottom - r.top), right: h + (r.right - r.left) })?;
    if front { m.select_window_front(win)?; }
    Ok(Done { result: 0, log: format!("[toolbox] MoveWindow {win:#x} to ({h},{v}){} (the native window stays where the user put it)", if front { ", selected" } else { "" }) })
}

/// `SizeWindow(window, w, h, fUpdate)`.
fn size_window<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (win, w, h, update) = (c.args.get(0), c.args.get(1) as i16, c.args.get(2) as i16, c.args.get(3) as u8 != 0);
    m.open_win("SizeWindow", win)?;
    if w <= 0 || h <= 0 { return Err(Refusal(format!("SizeWindow {win:#x}: {w}x{h} is an empty window"))); }
    let old = m.windows[&win].rect;
    m.set_window_rect(win, Rect { top: old.top, left: old.left, bottom: old.top + h, right: old.left + w })?;
    crate::qd::lifecycle::window_resized(m, win)?;
    let grew = w > old.right - old.left || h > old.bottom - old.top;
    if update || grew { m.post(UPDATE_EVT, win, 0, 0); }
    Ok(Done { result: 0, log: format!("[toolbox] SizeWindow {win:#x} {w}x{h} (was {}x{}){} (the native window keeps the user's size)", old.right - old.left, old.bottom - old.top, if update || grew { ", update posted" } else { "" }) })
}

fn new_c_window<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let a = &c.args;
    let spec = WindowSpec {
        storage: a.get(0), rect: read_rect(&m.mem, a.get(1))?, title: read_pstring(&m.mem, a.get(2))?, visible: a.get(3) as u8 != 0,
        proc_id: a.get(4) as u16, behind: a.get(5), go_away: a.get(6) as u8 != 0, ref_con: a.get(7),
    };
    crate::toolbox::created(m, "NewCWindow (a monochrome port)", spec)
}

fn get_new_c_window<U: GuestUi>(m: &mut Mac<U>, cpu: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    crate::toolbox::get_new_window_named(m, cpu, c, "GetNewCWindow (a monochrome port)")
}
