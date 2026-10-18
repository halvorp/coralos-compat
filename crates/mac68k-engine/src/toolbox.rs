//! The Window Manager bridged onto GuestUi: `NewWindow`, `GetNewWindow` (WIND), `DisposeWindow`, `ShowWindow`,
//! `SelectWindow`, `FrontWindow`, `FindWindow`, `TrackGoAway`, `BeginUpdate`/`EndUpdate`, `InitWindows`, `ExitToShell`.
//!
//! The WindowRecord is real guest memory (156 bytes: an embedded 108-byte GrafPort, then the window fields), so a
//! program that reads `window->portRect`, `window->visible` or walks `nextWindow` sees the right values. The record's
//! offsets are Inside Macintosh; the GrafPort offsets used here (`portBits` 2, `portRect` 16, `pnSize` 52, `pnMode` 56,
//! `txSize` 74) and `nextWindow` 144 agree with the Window Manager fields metamage_1's `FindWindow` walks.
//! A window created invisible opens no native window until `ShowWindow`.
//!
//! `FindWindow` is geometric over the host-tracked window rectangles: menu bar (rows above `MBarHeight`), then each window
//! front to back: content, the 18-row title bar above it (`inDrag`), and the go-away box inside the title bar
//! (`inGoAway`). A native close request arrives as a click on the *centre* of that box ([`go_away_point`]); that exact
//! point answers `inGoAway` even where the geometry would say menu bar (a window high on the screen).
use crate::events::{go_away_point, ACTIVATE_EVT, UPDATE_EVT};
use crate::lowmem::{MBAR_HEIGHT, SCRN_BASE, SCREEN_ROW, WINDOW_LIST};
use crate::machine::{Call, Done, Mac, MacWindow};
use crate::pascal::{read_pstring, read_rect, Arg, Rect};
use crate::quickdraw::set_the_port;
use crate::Refusal;
use alloc::format;
use alloc::string::String;
use guestui_abi::{CallbackToken, GuestRect, GuestUi, WindowModel, WindowRole};
use retro_m68k::Cpu;

/// sizeof(WindowRecord) (a GrafPort's 108 bytes + the window fields), Inside Macintosh.
pub const WINDOW_RECORD_SIZE: u32 = 156;
const WR_PORT_BITS: u32 = 2;
const WR_PORT_RECT: u32 = 16;
const WR_PN_SIZE: u32 = 52;
const WR_PN_MODE: u32 = 56;
const WR_TX_SIZE: u32 = 74;
const WR_KIND: u32 = 108;
const WR_VISIBLE: u32 = 110;
const WR_HILITED: u32 = 111;
const WR_GO_AWAY: u32 = 112;
const WR_NEXT: u32 = 144;
const WR_REF_CON: u32 = 152;
/// userKind: a window the application owns (system windows have a negative kind).
const USER_KIND: u16 = 8;
const PAT_COPY: u16 = 8;

// FindWindow results.
pub const IN_DESK: u16 = 0;
pub const IN_MENU_BAR: u16 = 1;
pub const IN_CONTENT: u16 = 3;
pub const IN_DRAG: u16 = 4;
pub const IN_GO_AWAY: u16 = 6;

/// What `NewWindow` / `GetNewWindow` ask for.
pub struct WindowSpec { pub storage: u32, pub rect: Rect, pub title: String, pub visible: bool, pub proc_id: u16, pub behind: u32, pub go_away: bool, pub ref_con: u32 }

fn done(log: String) -> Result<Done, Refusal> { Ok(Done { result: 0, log }) }

/// documentProc (0) and zoomDocProc (8) have a grow box; noGrowDocProc (4) / zoomNoGrow (12) are documents without
/// one; every other proc id (dBoxProc 1, plainDBox 2, altDBoxProc 3, movableDBoxProc 5, ...) is a dialog frame.
fn role_of(proc_id: u16) -> (WindowRole, bool) {
    match proc_id & 7 { 0 => (WindowRole::Document, true), 4 => (WindowRole::Document, false), _ => (WindowRole::Dialog, false) }
}

impl<U: GuestUi> Mac<U> {
    /// Write the geometry fields of a WindowRecord for content rectangle `r` (global coordinates; local origin 0,0).
    pub(crate) fn set_window_rect(&mut self, win: u32, r: Rect) -> Result<(), Refusal> {
        let e = Refusal::mem("WindowRecord");
        let w = self.windows.get_mut(&win).ok_or_else(|| Refusal(format!("{win:#x} is not an open window")))?;
        w.rect = r;
        let (h, wd) = (r.bottom - r.top, r.right - r.left);
        for (i, v) in [0, 0, h, wd].into_iter().enumerate() { self.mem.write_u16(win + WR_PORT_RECT + 2 * i as u32, v as u16).map_err(&e)?; }
        // portBits.bounds maps local coordinates to the screen: its top-left is minus the global origin of the content.
        for (i, v) in [-r.top, -r.left, -r.top + h, -r.left + wd].into_iter().enumerate() { self.mem.write_u16(win + WR_PORT_BITS + 6 + 2 * i as u32, v as u16).map_err(&e)?; }
        Ok(())
    }

    /// Rewrite WindowList and every nextWindow link from the z-order (front first).
    pub(crate) fn relink_windows(&mut self) -> Result<(), Refusal> {
        let e = Refusal::mem("window list");
        let order = self.z.clone();
        self.mem.write_u32(WINDOW_LIST, order.first().copied().unwrap_or(0)).map_err(&e)?;
        for (i, w) in order.iter().enumerate() { self.mem.write_u32(w + WR_NEXT, order.get(i + 1).copied().unwrap_or(0)).map_err(&e)?; }
        Ok(())
    }

    fn open_native(&mut self, win: u32) -> Result<(), Refusal> {
        let w = self.windows.get(&win).ok_or_else(|| Refusal(format!("{win:#x} is not an open window")))?;
        let (r, role) = (w.rect, role_of(w.proc_id));
        let model = WindowModel {
            title: w.title.clone(),
            rect: GuestRect { left: i32::from(r.left), top: i32::from(r.top), width: i32::from(r.right - r.left), height: i32::from(r.bottom - r.top) },
            role: role.0, closable: w.go_away, resizable: role.1, token: CallbackToken(u64::from(win)),
        };
        let native = self.ui.open_window(model).map_err(Refusal::ui)?;
        if let Some(w) = self.windows.get_mut(&win) { w.native = Some(native); }
        if self.menus.drawn { let strip = self.menu_strip()?; self.ui.set_menu_strip(native, strip).map_err(Refusal::ui)?; }
        self.mem.write_u8(win + WR_VISIBLE, 1).map_err(Refusal::mem("WindowRecord"))?;
        self.post(ACTIVATE_EVT, win, 0, 0);
        self.ev.queue.back_mut().map(|e| e.modifiers = 1); // bit 0: activate
        self.post(UPDATE_EVT, win, 0, 0);
        crate::qd::lifecycle::window_shown(self, win);
        Ok(())
    }

    pub fn make_window(&mut self, s: WindowSpec) -> Result<u32, Refusal> {
        let r = s.rect;
        let (w, h) = (i32::from(r.right) - i32::from(r.left), i32::from(r.bottom) - i32::from(r.top));
        if w <= 0 || h <= 0 { return Err(Refusal(format!("NewWindow: empty boundsRect ({},{},{},{})", r.top, r.left, r.bottom, r.right))); }
        let owned = s.storage == 0;
        let win = if owned {
            self.mm.new_ptr(&mut self.mem, &mut self.heap, WINDOW_RECORD_SIZE, true).map_err(|e| Refusal(format!("NewWindow: no memory for a WindowRecord (memFullErr {e})")))?
        } else { s.storage };
        let fill = (|| {
            let e = Refusal::mem("WindowRecord");
            self.mem.zero(win, WINDOW_RECORD_SIZE).map_err(&e)?;
            let (base, row) = (self.mem.read_u32(SCRN_BASE).map_err(&e)?, self.mem.read_u16(SCREEN_ROW).map_err(&e)?);
            self.mem.write_u32(win + WR_PORT_BITS, base).map_err(&e)?;
            self.mem.write_u16(win + WR_PORT_BITS + 4, row).map_err(&e)?;
            self.mem.write_u16(win + WR_PN_SIZE, 1).map_err(&e)?;
            self.mem.write_u16(win + WR_PN_SIZE + 2, 1).map_err(&e)?;
            self.mem.write_u16(win + WR_PN_MODE, PAT_COPY).map_err(&e)?;
            self.mem.write_u16(win + WR_TX_SIZE, 12).map_err(&e)?;
            self.mem.write_u16(win + WR_KIND, USER_KIND).map_err(&e)?;
            self.mem.write_u8(win + WR_VISIBLE, 0).map_err(&e)?;
            self.mem.write_u8(win + WR_HILITED, 0).map_err(&e)?;
            self.mem.write_u8(win + WR_GO_AWAY, s.go_away as u8).map_err(&e)?;
            self.mem.write_u32(win + WR_REF_CON, s.ref_con).map_err(&e)
        })();
        let undo = |m: &mut Self| if owned { let _ = m.mm.dispose_ptr(&mut m.heap, win); };
        if let Err(e) = fill { undo(self); return Err(e); }
        self.windows.insert(win, MacWindow { native: None, owned, rect: r, title: s.title, go_away: s.go_away, visible: s.visible, proc_id: s.proc_id });
        // behind: -1 = in front of all, 0 = behind all, else behind that window
        let at = match s.behind { 0xFFFF_FFFF => 0, 0 => self.z.len(), b => self.z.iter().position(|&z| z == b).map_or(0, |i| i + 1) };
        self.z.insert(at, win);
        if let Err(e) = self.set_window_rect(win, r).and_then(|_| self.relink_windows()).and_then(|_| crate::qd::lifecycle::window_created(self, win)) { crate::qd::lifecycle::window_disposed(self, win); self.windows.remove(&win); self.z.retain(|&z| z != win); undo(self); return Err(e); }
        if s.visible {
            if let Err(e) = self.open_native(win) { crate::qd::lifecycle::window_disposed(self, win); self.windows.remove(&win); self.z.retain(|&z| z != win); let _ = self.relink_windows(); undo(self); return Err(e); }
        }
        if self.qd_globals != 0 { set_the_port(self, win)?; }
        Ok(win)
    }

    /// Global point -> FindWindow part and window.
    pub fn find_window(&self, v: i16, h: i16) -> (u16, u32) {
        for (&win, w) in &self.windows {
            if w.go_away && w.native.is_some() && go_away_point(&w.rect) == (v, h) { return (IN_GO_AWAY, win); }
        }
        let mbar = self.mem.read_u16(MBAR_HEIGHT).unwrap_or(20) as i16;
        if v < mbar { return (IN_MENU_BAR, 0); }
        for &win in &self.z {
            let Some(w) = self.windows.get(&win) else { continue };
            if !w.visible || w.native.is_none() { continue; }
            let r = w.rect;
            let in_cols = h >= r.left && h < r.right;
            if in_cols && v >= r.top && v < r.bottom { return (IN_CONTENT, win); }
            if in_cols && v >= r.top - 18 && v < r.top {
                if w.go_away && v >= r.top - 15 && v < r.top - 4 && h >= r.left + 8 && h < r.left + 21 { return (IN_GO_AWAY, win); }
                return (IN_DRAG, win);
            }
        }
        (IN_DESK, 0)
    }
}

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    m.register(0xA912, &[], None, init_windows::<U>)?;
    m.register(0xA913, &[Arg::L, Arg::L, Arg::L, Arg::B, Arg::W, Arg::L, Arg::B, Arg::L], Some(Arg::L), new_window::<U>)?;
    m.register(0xA9BD, &[Arg::W, Arg::L, Arg::L], Some(Arg::L), get_new_window::<U>)?;
    m.register(0xA914, &[Arg::L], None, dispose_window::<U>)?;
    m.register(0xA915, &[Arg::L], None, show_window::<U>)?;
    m.register(0xA91F, &[Arg::L], None, select_window::<U>)?;
    m.register(0xA924, &[], Some(Arg::L), front_window::<U>)?;
    m.register(0xA92C, &[Arg::L, Arg::L], Some(Arg::W), find_window::<U>)?;
    m.register(0xA91E, &[Arg::L, Arg::L], Some(Arg::B), track_go_away::<U>)?;
    m.register(0xA9F4, &[], None, exit_to_shell::<U>)
}

fn init_windows<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    m.z.clear();
    m.mem.write_u32(WINDOW_LIST, 0).map_err(Refusal::mem("WindowList"))?;
    done("[toolbox] InitWindows".into())
}

fn exit_to_shell<U: GuestUi>(_: &mut Mac<U>, cpu: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    cpu.halt();
    done("[toolbox] ExitToShell".into())
}

fn created<U: GuestUi>(m: &mut Mac<U>, what: &str, s: WindowSpec) -> Result<Done, Refusal> {
    let (title, r, proc_id) = (s.title.clone(), s.rect, s.proc_id);
    let win = m.make_window(s)?;
    Ok(Done { result: win, log: format!("[toolbox] {what} \"{title}\" top={} left={} w={} h={} proc={proc_id} -> {win:#x}", r.top, r.left, r.right - r.left, r.bottom - r.top) })
}

fn new_window<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let a = &c.args;
    let spec = WindowSpec {
        storage: a.get(0), rect: read_rect(&m.mem, a.get(1))?, title: read_pstring(&m.mem, a.get(2))?, visible: a.get(3) as u8 != 0,
        proc_id: a.get(4) as u16, behind: a.get(5), go_away: a.get(6) as u8 != 0, ref_con: a.get(7),
    };
    created(m, "NewWindow", spec)
}

/// `WIND` resource: boundsRect (8), procID (2), visible (2), goAwayFlag (2), refCon (4), title (Pascal string).
fn get_new_window<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let id = c.args.get(0) as u16 as i16;
    let Ok(h) = m.get_resource(*b"WIND", id, false) else { return done(format!("[toolbox] GetNewWindow {id}: no WIND resource")) };
    let b = m.handle_bytes(h).unwrap_or_default();
    let be = |i: usize| i16::from_be_bytes([b[i], b[i + 1]]);
    if b.len() < 19 || b.len() < 19 + b[18] as usize { return Err(Refusal(format!("GetNewWindow {id}: WIND resource is {} bytes, too short for its fields and title", b.len()))); }
    let spec = WindowSpec {
        storage: c.args.get(1), rect: Rect { top: be(0), left: be(2), bottom: be(4), right: be(6) },
        title: b[19..19 + b[18] as usize].iter().map(|&x| char::from(x)).collect(),
        proc_id: be(8) as u16, visible: be(10) != 0, go_away: be(12) != 0,
        ref_con: u32::from_be_bytes([b[14], b[15], b[16], b[17]]), behind: c.args.get(2),
    };
    created(m, &format!("GetNewWindow {id}"), spec)
}

fn dispose_window<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let win = c.args.get(0);
    let w = m.windows.remove(&win).ok_or_else(|| Refusal(format!("DisposeWindow: {win:#x} is not an open window")))?;
    crate::qd::lifecycle::window_disposed(m, win);
    if let Some(n) = w.native { m.ui.close_window(n).map_err(Refusal::ui)?; }
    if w.owned { m.mm.dispose_ptr(&mut m.heap, win).map_err(|e| Refusal(format!("DisposeWindow: {e}")))?; }
    m.z.retain(|&z| z != win);
    m.ev.queue.retain(|e| !(matches!(e.what, UPDATE_EVT | ACTIVATE_EVT) && e.message == win));
    if m.qd_globals != 0 && m.mem.read_u32(m.qd_globals).unwrap_or(0) == win { set_the_port(m, 0)?; }
    m.relink_windows()?;
    done(format!("[toolbox] DisposeWindow {win:#x}"))
}

fn show_window<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let win = c.args.get(0);
    let w = m.windows.get_mut(&win).ok_or_else(|| Refusal(format!("ShowWindow: {win:#x} is not an open window")))?;
    w.visible = true;
    if w.native.is_none() { m.open_native(win)?; }
    done(format!("[toolbox] ShowWindow {win:#x}"))
}

fn select_window<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let win = c.args.get(0);
    if !m.windows.contains_key(&win) { return Err(Refusal(format!("SelectWindow: {win:#x} is not an open window"))); }
    let old = m.z.first().copied();
    if old != Some(win) {
        m.z.retain(|&z| z != win);
        m.z.insert(0, win);
        if let Some(o) = old { m.post(ACTIVATE_EVT, o, 0, 0); }
        m.post(ACTIVATE_EVT, win, 0, 0);
        m.ev.queue.back_mut().map(|e| e.modifiers = 1);
        m.relink_windows()?;
    }
    done(format!("[toolbox] SelectWindow {win:#x}"))
}

fn front_window<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    let f = m.z.iter().copied().find(|w| m.windows.get(w).is_some_and(|w| w.visible)).unwrap_or(0);
    Ok(Done { result: f, log: format!("[toolbox] FrontWindow -> {f:#x}") })
}

/// A Point passed by value is one long: v in the high word, h in the low.
fn point(v: u32) -> (i16, i16) { ((v >> 16) as i16, v as i16) }

fn find_window<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (v, h) = point(c.args.get(0));
    let (part, win) = m.find_window(v, h);
    m.mem.write_u32(c.args.get(1), win).map_err(Refusal::mem("FindWindow result"))?;
    Ok(Done { result: u32::from(part), log: format!("[toolbox] FindWindow ({v},{h}) -> part {part} window {win:#x}") })
}

/// `TrackGoAway(window, startPt)`: TRUE when the point lies in that window's go-away box (there is no mouse to track).
fn track_go_away<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let win = c.args.get(0);
    let (v, h) = point(c.args.get(1));
    let hit = m.find_window(v, h) == (IN_GO_AWAY, win);
    Ok(Done { result: u32::from(hit), log: format!("[toolbox] TrackGoAway {win:#x} ({v},{h}) -> {hit}") })
}

