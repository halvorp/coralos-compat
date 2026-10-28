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
use crate::lowmem::{DESK_PORT, GRAY_RGN, MBAR_HEIGHT, SCRN_BASE, SCREEN_ROW, WINDOW_LIST, WMGR_PORT};
use crate::machine::{Call, Done, Mac, MacWindow};
use crate::pascal::{read_pstring, read_rect, Arg, Rect};
use crate::quickdraw::set_the_port;
use crate::Refusal;
use alloc::format;
use alloc::string::String;
use heirloom_ui_abi::{CallbackToken, GuestRect, GuestUi, WindowModel, WindowRole};
use crate::cpu::Cpu;

/// sizeof(WindowRecord) (a GrafPort's 108 bytes + the window fields), Inside Macintosh.
pub const WINDOW_RECORD_SIZE: u32 = 156;
const WR_PORT_BITS: u32 = 2;
const WR_PORT_RECT: u32 = 16;
const WR_PN_SIZE: u32 = 52;
const WR_PN_MODE: u32 = 56;
const WR_TX_SIZE: u32 = 74;
pub(crate) const WR_KIND: u32 = 108;
const WR_VISIBLE: u32 = 110;
pub(crate) const WR_HILITED: u32 = 111;
const WR_GO_AWAY: u32 = 112;
const WR_NEXT: u32 = 144;
pub(crate) const WR_REF_CON: u32 = 152;
/// userKind: a window the application owns (system windows have a negative kind).
const USER_KIND: u16 = 8;
/// dialogKind: a window a DialogRecord is (Inside Macintosh I, "The Dialog Manager": windowKind of a dialog or alert).
pub(crate) const DIALOG_KIND: u16 = 2;
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

/// The Window Manager's procID table (Inside Macintosh I): `procID = 16 * WDEF id + variation`. Only the system WDEFs have a
/// native role: WDEF 0 (documentProc 0 and zoomDocProc 8 have a grow box; noGrowDocProc 4 / zoomNoGrow 12 are documents
/// without one; dBoxProc 1, plainDBox 2, altDBoxProc 3, movableDBoxProc 5 and the rest are dialog frames) and WDEF 1
/// (rDocProc 16: a rounded document window, no grow box, its variation is the corner size). Any other WDEF is the program's
/// own window definition: no native presentation contract exists for it (HEIRLOOM-ROLES-REQUIRE-A-CONTRACT), so it is a named
/// refusal (docs/consults/2026-10-20-wdef.md), never a document window by its low bits.
pub(crate) fn role_of(proc_id: u16) -> Result<(WindowRole, bool), Refusal> {
    match (proc_id >> 4, proc_id & 15) {
        (0, v) => Ok(match v & 7 { 0 => (WindowRole::Document, true), 4 => (WindowRole::Document, false), _ => (WindowRole::Dialog, false) }),
        (1, _) => Ok((WindowRole::Document, false)),
        (id, v) => Err(Refusal(format!("procID {proc_id} selects WDEF {id} variation {v}, the program's own window definition: no native presentation contract [WDEF-UNSUPPORTED]"))),
    }
}

impl<U: GuestUi> Mac<U> {
    /// Write the geometry fields of a WindowRecord for content rectangle `r` (global coordinates; local origin 0,0).
    pub(crate) fn set_window_rect(&mut self, win: u32, r: Rect) -> Result<(), Refusal> {
        let e = Refusal::mem("WindowRecord");
        let w = self.windows.get_mut(&win).ok_or_else(|| Refusal(format!("{win:#x} is not an open window")))?;
        w.rect = r;
        let (h, wd) = (r.bottom - r.top, r.right - r.left);
        for (i, v) in [0, 0, h, wd].into_iter().enumerate() { self.mem.write_u16(win + WR_PORT_RECT + 2 * i as u32, v as u16).map_err(&e)?; }
        let bits = crate::qd::port::bits_addr(&self.mem, win)?; // portBits, or a colour window's PixMap
        // portBits.bounds maps local coordinates to the screen: its top-left is minus the global origin of the content.
        for (i, v) in [-r.top, -r.left, -r.top + h, -r.left + wd].into_iter().enumerate() { self.mem.write_u16(bits + 6 + 2 * i as u32, v as u16).map_err(&e)?; }
        self.sync_window_regions(win)
    }

    /// Rewrite WindowList and every nextWindow link from the z-order (front first).
    pub(crate) fn relink_windows(&mut self) -> Result<(), Refusal> {
        let e = Refusal::mem("window list");
        let order = self.z.clone();
        self.mem.write_u32(WINDOW_LIST, order.first().copied().unwrap_or(0)).map_err(&e)?;
        for (i, w) in order.iter().enumerate() { self.mem.write_u32(w + WR_NEXT, order.get(i + 1).copied().unwrap_or(0)).map_err(&e)?; }
        Ok(())
    }

    pub(crate) fn open_native(&mut self, win: u32) -> Result<(), Refusal> {
        let w = self.windows.get(&win).ok_or_else(|| Refusal(format!("{win:#x} is not an open window")))?;
        let (r, proc_id) = (w.rect, w.proc_id);
        let role = self.window_role(proc_id)?;
        let w = self.windows.get(&win).ok_or_else(|| Refusal(format!("{win:#x} is not an open window")))?;
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
        self.project_window(win)?;
        Ok(())
    }

    pub fn make_window(&mut self, s: WindowSpec) -> Result<u32, Refusal> { self.make_window_as(s, false) }

    /// [`Mac::make_window`], with a colour window's CGrafPort (8-bit canvas) when `color` and the machine has Color QuickDraw.
    pub fn make_window_as(&mut self, s: WindowSpec, color: bool) -> Result<u32, Refusal> {
        self.window_role(s.proc_id)?; // before anything is allocated or shown
        let color = color && self.profile.color_qd;
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
            if color { self.begin_window_cport(win)?; } else {
                let (base, row) = (self.mem.read_u32(SCRN_BASE).map_err(&e)?, self.mem.read_u16(SCREEN_ROW).map_err(&e)?);
                self.mem.write_u32(win + WR_PORT_BITS, base).map_err(&e)?;
                self.mem.write_u16(win + WR_PORT_BITS + 4, row).map_err(&e)?;
            }
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
        let title = s.title.clone();
        self.windows.insert(win, MacWindow { native: None, owned, rect: r, title: s.title, go_away: s.go_away, visible: s.visible, proc_id: s.proc_id, color });
        // behind: -1 = in front of all, 0 = behind all, else behind that window
        let at = match s.behind { 0xFFFF_FFFF => 0, 0 => self.z.len(), b => self.z.iter().position(|&z| z == b).map_or(0, |i| i + 1) };
        self.z.insert(at, win);
        if let Err(e) = self.put_window_title(win, &title) { self.windows.remove(&win); self.z.retain(|&z| z != win); undo(self); return Err(e); }
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
    m.register(0xA914, &[Arg::P], None, dispose_window::<U>)?;
    m.register(0xA915, &[Arg::P], None, show_window::<U>)?;
    m.register(0xA916, &[Arg::P], None, hide_window_trap::<U>)?;
    m.register(0xA91F, &[Arg::P], None, select_window::<U>)?;
    m.register(0xA925, &[Arg::P, Arg::L, Arg::P], None, drag_window::<U>)?;
    m.register(0xA924, &[], Some(Arg::L), front_window::<U>)?;
    m.register(0xA92C, &[Arg::L, Arg::L], Some(Arg::W), find_window::<U>)?;
    m.register(0xA91E, &[Arg::L, Arg::L], Some(Arg::B), track_go_away::<U>)?;
    m.register(0xA9F4, &[], None, exit_to_shell::<U>)
}

/// The corner radius of the main screen's desktop (Inside Macintosh I: the desktop has rounded corners; `ROMlib_InitGrayRgn` and
/// the System's `InitWindows` both frame the screen with a 16 x 16 round rectangle).
const DESKTOP_CORNER: i32 = 16;

/// `InitWindows`: the Window Manager port (`WMgrPort`, `DeskPort`: a port over the screen) and the desktop region (`GrayRgn`: the
/// screen with its rounded corners minus the menu bar), which a program reads from low memory to hit-test the desktop (`PtInRgn`),
/// to drag within it (`DragWindow`'s bounds) or to draw on it. The Window Manager port is `thePort` afterwards (`InitGraf` left it
/// NIL; the program's own `GetPort`/`SetPort` pairs around its first window must save and restore a real port). The host draws
/// the desktop itself, so what a program paints into the Window Manager port is not shown.
fn init_windows<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    if m.qd_globals == 0 { return Err(Refusal("InitWindows: InitGraf has not run (the Window Manager port needs screenBits)".into())); }
    m.z.clear();
    m.mem.write_u32(WINDOW_LIST, 0).map_err(Refusal::mem("WindowList"))?;
    let e = Refusal::mem("Window Manager globals");
    let mut port = m.mem.read_u32(WMGR_PORT).map_err(&e)?;
    if port == 0 { port = m.mm.new_ptr(&mut m.mem, &mut m.heap, crate::qd::port::PORT_SIZE, true).map_err(|x| Refusal(format!("InitWindows: no memory for the Window Manager port (memFullErr {x})")))?; }
    let screen = crate::qd::lifecycle::open_screen_port(m, port)?;
    let mut desktop = crate::qd::shapes::round_rect(screen, DESKTOP_CORNER, DESKTOP_CORNER);
    let mbar = i32::from(m.mem.read_u16(MBAR_HEIGHT).map_err(&e)? as i16);
    desktop.clear_rect(heirloom_raster::Rect::new(screen.x0, screen.y0, screen.x1, screen.y0 + mbar));
    let gray = match m.mem.read_u32(GRAY_RGN).map_err(&e)? { 0 => m.new_region(&desktop)?, h => { m.write_region(h, &desktop)?; h } };
    let clip = m.mem.read_u32(port + crate::qd::port::P_CLIP).map_err(&e)?;
    m.write_region(clip, &desktop)?;
    for (at, v) in [(WMGR_PORT, port), (DESK_PORT, port), (GRAY_RGN, gray)] { m.mem.write_u32(at, v).map_err(&e)?; }
    crate::quickdraw::set_the_port(m, port)?; // as `OpenPort` does: the Window Manager port is current until the program makes a window current
    done(format!("[toolbox] InitWindows: Window Manager port {port:#x}, GrayRgn {gray:#x}"))
}

fn exit_to_shell<U: GuestUi>(_: &mut Mac<U>, cpu: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    cpu.halt();
    done("[toolbox] ExitToShell".into())
}

pub(crate) fn created<U: GuestUi>(m: &mut Mac<U>, what: &str, s: WindowSpec) -> Result<Done, Refusal> { created_as(m, what, s, false) }

/// [`created`] for a colour window (`NewCWindow`/`GetNewCWindow`) when `color`.
pub(crate) fn created_as<U: GuestUi>(m: &mut Mac<U>, what: &str, s: WindowSpec, color: bool) -> Result<Done, Refusal> {
    let (title, r, proc_id) = (s.title.clone(), s.rect, s.proc_id);
    let win = m.make_window_as(s, color)?;
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
fn get_new_window<U: GuestUi>(m: &mut Mac<U>, cpu: &mut Cpu, c: &Call) -> Result<Done, Refusal> { get_new_window_named(m, cpu, c, "GetNewWindow", false) }

/// `GetNewWindow` and the Color Window Manager's `GetNewCWindow`: the same `WIND` resource, the same native window.
pub(crate) fn get_new_window_named<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call, what: &str, color: bool) -> Result<Done, Refusal> {
    let id = c.args.get(0) as u16 as i16;
    let Ok(h) = m.get_resource(*b"WIND", id, false) else { return done(format!("[toolbox] {what} {id}: no WIND resource")) };
    let b = m.handle_bytes(h).unwrap_or_default();
    let be = |i: usize| i16::from_be_bytes([b[i], b[i + 1]]);
    if b.len() < 19 || b.len() < 19 + b[18] as usize { return Err(Refusal(format!("{what} {id}: WIND resource is {} bytes, too short for its fields and title", b.len()))); }
    let spec = WindowSpec {
        storage: c.args.get(1), rect: Rect { top: be(0), left: be(2), bottom: be(4), right: be(6) },
        title: b[19..19 + b[18] as usize].iter().map(|&x| char::from(x)).collect(),
        proc_id: be(8) as u16, visible: be(10) != 0, go_away: be(12) != 0,
        ref_con: u32::from_be_bytes([b[14], b[15], b[16], b[17]]), behind: c.args.get(2),
    };
    created_as(m, &format!("{what} {id}"), spec, color)
}

fn dispose_window<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let win = c.args.get(0);
    m.dispose_window(win)?;
    done(format!("[toolbox] DisposeWindow {win:#x}"))
}

fn show_window<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let win = c.args.get(0);
    m.show_window(win)?;
    done(format!("[toolbox] ShowWindow {win:#x}"))
}

impl<U: GuestUi> Mac<U> {
    /// Close a window and everything it owns: its controls and dialog items (the host retires them with the window), its
    /// canvas, its record when the engine allocated it. Shared by `DisposeWindow` and `DisposeDialog`.
    pub(crate) fn dispose_window(&mut self, win: u32) -> Result<(), Refusal> {
        if !self.windows.contains_key(&win) { return Err(Refusal(format!("DisposeWindow: {win:#x} is not an open window"))); }
        self.kill_controls(win, false)?;
        self.forget_window_projections(win);
        self.pal.by_window.remove(&win);
        let dialog_storage = self.release_dialog(win);
        let w = self.windows.remove(&win).ok_or_else(|| Refusal(format!("DisposeWindow: {win:#x} is not an open window")))?;
        self.free_window_title(win);
        crate::qd::lifecycle::window_disposed(self, win);
        if let Some(n) = w.native { self.ui.close_window(n).map_err(Refusal::ui)?; }
        if w.owned { self.mm.dispose_ptr(&mut self.heap, win).map_err(|e| Refusal(format!("DisposeWindow: {e}")))?; }
        self.z.retain(|&z| z != win);
        self.ev.queue.retain(|e| !(matches!(e.what, UPDATE_EVT | ACTIVATE_EVT) && e.message == win));
        if self.qd_globals != 0 && self.mem.read_u32(self.qd_globals).unwrap_or(0) == win { set_the_port(self, 0)?; }
        self.relink_windows()?;
        if let Some(p) = dialog_storage { self.mm.dispose_ptr(&mut self.heap, p).map_err(|e| Refusal(format!("DisposeDialog: {e}")))?; }
        Ok(())
    }

    /// `ShowWindow`: the window becomes visible and gets its native window the first time.
    pub(crate) fn show_window(&mut self, win: u32) -> Result<(), Refusal> {
        let w = self.windows.get_mut(&win).ok_or_else(|| Refusal(format!("ShowWindow: {win:#x} is not an open window")))?;
        let was_visible = w.visible;
        w.visible = true;
        if w.native.is_none() { self.open_native(win)?; }
        else if !was_visible {
            // a window the program hid with HideWindow: the host window is shown again, and its content is redrawn
            let n = self.windows[&win].native.expect("checked");
            self.ui.show_window(n).map_err(Refusal::ui)?;
            self.mem.write_u8(win + WR_VISIBLE, 1).map_err(Refusal::mem("WindowRecord"))?;
            self.post(UPDATE_EVT, win, 0, 0);
        }
        Ok(())
    }

    /// `HideWindow`: the window stays open (record, canvas, controls) but is not shown. The host is told through `GuestUi::hide_window` -- a host without it refuses by name and the
    /// window is left VISIBLE (the guest's state is not changed to something the screen does not show).
    pub(crate) fn hide_window(&mut self, win: u32) -> Result<(), Refusal> {
        let w = self.windows.get(&win).ok_or_else(|| Refusal(format!("HideWindow: {win:#x} is not an open window")))?;
        if !w.visible { return Ok(()); }
        if let Some(n) = w.native { self.ui.hide_window(n).map_err(Refusal::ui)?; }
        if let Some(w) = self.windows.get_mut(&win) { w.visible = false; }
        self.mem.write_u8(win + WR_VISIBLE, 0).map_err(Refusal::mem("WindowRecord"))?;
        Ok(())
    }
}

fn hide_window_trap<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let win = c.args.get(0);
    m.hide_window(win)?;
    done(format!("[toolbox] HideWindow {win:#x}"))
}

fn select_window<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let win = c.args.get(0);
    m.select_window_front(win)?;
    done(format!("[toolbox] SelectWindow {win:#x}"))
}

impl<U: GuestUi> Mac<U> {
    /// Bring `win` to the front: activate events for the old front window (deactivate) and for `win`, WindowList relinked.
    pub(crate) fn select_window_front(&mut self, win: u32) -> Result<(), Refusal> {
        if !self.windows.contains_key(&win) { return Err(Refusal(format!("SelectWindow: {win:#x} is not an open window"))); }
        let old = self.z.first().copied();
        if old != Some(win) {
            self.z.retain(|&z| z != win);
            self.z.insert(0, win);
            if let Some(o) = old { self.post(ACTIVATE_EVT, o, 0, 0); }
            self.post(ACTIVATE_EVT, win, 0, 0);
            self.ev.queue.back_mut().map(|e| e.modifiers = 1);
            self.relink_windows()?;
        }
        Ok(())
    }
}

fn front_window<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    let f = m.z.iter().copied().find(|w| m.windows.get(w).is_some_and(|w| w.visible)).unwrap_or(0);
    Ok(Done { result: f, log: format!("[toolbox] FrontWindow -> {f:#x}") })
}

/// A Point passed by value is one long: v in the high word, h in the low.
pub(crate) fn point(v: u32) -> (i16, i16) { ((v >> 16) as i16, v as i16) }

fn find_window<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (v, h) = point(c.args.get(0));
    let (part, win) = m.find_window(v, h);
    m.mem.write_u32(c.args.get(1), win).map_err(Refusal::mem("FindWindow result"))?;
    Ok(Done { result: u32::from(part), log: format!("[toolbox] FindWindow ({v},{h}) -> part {part} window {win:#x}") })
}

/// `DragWindow(window, startPt, boundsRect)`: the title bar is the host's, so the user's drag is a native window move and the
/// guest has no mouse to track. The call validates the window and returns; the window's rectangle follows the native position
/// through the same path a native move always takes (`set_window_rect`).
fn drag_window<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let win = c.args.get(0);
    if !m.windows.contains_key(&win) { return Err(Refusal(format!("DragWindow: {win:#x} is not an open window"))); }
    done(format!("[toolbox] DragWindow {win:#x}: the host moves the window (native title bar); nothing for the guest to track"))
}

/// `TrackGoAway(window, startPt)`: TRUE when the point lies in that window's go-away box (there is no mouse to track).
fn track_go_away<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let win = c.args.get(0);
    let (v, h) = point(c.args.get(1));
    let hit = m.find_window(v, h) == (IN_GO_AWAY, win);
    Ok(Done { result: u32::from(hit), log: format!("[toolbox] TrackGoAway {win:#x} ({v},{h}) -> {hit}") })
}
