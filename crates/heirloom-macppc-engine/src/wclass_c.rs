//! The Mac OS 8.5 / Carbon Window Manager's WINDOW CLASS calls (MacWindows.h): a Carbon window carries the class it was created with
//! (`CreateNewWindow` / `CreateCustomWindow`, carbon.rs), and these calls answer from it and from the machine's window order (`Mac::z`, front
//! first). A window made by a classic `NewWindow` / `GetNewWindow` has no recorded class: it answers as a document window
//! (kDocumentWindowClass 6), its modality none.
//!
//! * `GetWindowClass(win, &class)`, `GetFrontWindowOfClass(class, mustBeVisible)`, `GetNextWindowOfClass(win, class, mustBeVisible)`
//!   (kAllWindowClasses 0xFFFFFFFF matches any).
//! * `FrontNonFloatingWindow()`: the front window that is not floating (utility / floating classes 5 and 8 are presented as document
//!   windows, but they are still skipped here: the program asked for the non-floating one).
//! * `GetWindowModality(win, &kind, &unavailableWindow)` / `SetWindowModality`: what was set, else app-modal for the modal / movable-modal /
//!   alert / movable-alert classes (1..4), none otherwise; the unavailable window is NULL (no window-modal sheets here).
//! * `RepositionWindow(win, parent, method)`: the kWindowCenter* / kWindowAlert* / kWindowStagger* positions on the main screen (or
//!   centered on `parent` for the *OnParentWindow methods), through the same MoveWindow trap a program calls; cascades are placed like a
//!   center (logged).
//! * `IsValidWindowPtr(win)`, `SetWindowKind(win, kind)` (the record's windowKind at 108).
//! * `InstallWindowContentPaintProc(win, paintProc, options, refCon)` (Mac OS 9 WindowsLib): the Window Manager calls such a procedure
//!   INSTEAD of erasing an exposed part of the content to the window's background. This Window Manager never erases or repaints content:
//!   a window's canvas starts as its background and keeps what the program drew, and every exposure is the program's update event. So
//!   the procedure is recorded (the latest per window, NULL removes it) and never called, said once per window by name in the log.
use crate::clib::arg;
use crate::machine::MacPpc;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

const DOCUMENT_CLASS: u32 = 6;
const ALL_CLASSES: u32 = 0xFFFF_FFFF;
const FLOATING: u32 = 5;
const UTILITY: u32 = 8;
const WINDOW_KIND: u32 = 108;
const WR_VISIBLE: u32 = 110;
const ERR_INVALID_WINDOW_REF: i32 = -5600;
const PARAM_ERR: i32 = -50;

#[derive(Default)]
pub struct WcState {
    /// window -> the class it was created with.
    pub(crate) class: BTreeMap<u32, u32>,
    /// window -> (modality kind, unavailable window) as SetWindowModality set it.
    modality: BTreeMap<u32, (u32, u32)>,
    /// window -> (content paint procedure, options, refCon) as InstallWindowContentPaintProc recorded it (never called: see the module doc).
    pub(crate) paint_proc: BTreeMap<u32, (u32, u32, u32)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WcFn { GetWindowClass, GetFrontWindowOfClass, GetNextWindowOfClass, FrontNonFloatingWindow, GetWindowModality, SetWindowModality, RepositionWindow, IsValidWindowPtr, SetWindowKind, InstallWindowContentPaintProc }

impl WcFn {
    pub fn by_name(name: &str) -> Option<WcFn> {
        use WcFn::*;
        Some(match name {
            "GetWindowClass" => GetWindowClass, "GetFrontWindowOfClass" => GetFrontWindowOfClass, "GetNextWindowOfClass" => GetNextWindowOfClass,
            "FrontNonFloatingWindow" => FrontNonFloatingWindow, "GetWindowModality" => GetWindowModality, "SetWindowModality" => SetWindowModality,
            "RepositionWindow" => RepositionWindow, "IsValidWindowPtr" => IsValidWindowPtr, "SetWindowKind" => SetWindowKind, "InstallWindowContentPaintProc" => InstallWindowContentPaintProc,
            _ => return None,
        })
    }
}

impl<U: GuestUi> MacPpc<U> {
    fn class_of(&self, w: u32) -> u32 { self.wc.class.get(&w).copied().unwrap_or(DOCUMENT_CLASS) }

    fn visible(&self, w: u32) -> bool { self.mac.mem.read_u8(w + WR_VISIBLE).unwrap_or(0) != 0 }

    /// The first window in front-to-back order after `after` (None = from the front) of `class` (and visible when asked).
    fn window_of_class(&self, after: Option<u32>, class: u32, must_be_visible: bool) -> u32 {
        let start = after.and_then(|a| self.mac.z.iter().position(|&w| w == a)).map_or(0, |i| i + 1);
        self.mac.z[start.min(self.mac.z.len())..].iter().copied()
            .find(|&w| (class == ALL_CLASSES || self.class_of(w) == class) && (!must_be_visible || self.visible(w))).unwrap_or(0)
    }

    pub(crate) fn wc_call(&mut self, f: WcFn, cpu: &mut Cpu) -> Result<(), String> {
        use WcFn::*;
        let a = |m: &Self, i: usize| arg(m, cpu, i);
        let (a0, a1, a2) = (a(self, 0)?, a(self, 1).unwrap_or(0), a(self, 2).unwrap_or(0));
        let valid = |m: &Self, w: u32| m.mac.windows.contains_key(&w);
        let r: u32 = match f {
            GetWindowClass => if valid(self, a0) { let c = self.class_of(a0); self.put32(a1, c, "GetWindowClass")?; 0 } else { ERR_INVALID_WINDOW_REF as u32 },
            GetFrontWindowOfClass => self.window_of_class(None, a0, a1 & 0xFF != 0),
            GetNextWindowOfClass => self.window_of_class(Some(a0), a1, a2 & 0xFF != 0),
            FrontNonFloatingWindow => self.mac.z.iter().copied().find(|&w| self.visible(w) && !matches!(self.class_of(w), FLOATING | UTILITY)).unwrap_or(0),
            GetWindowModality => {
                if !valid(self, a0) { ERR_INVALID_WINDOW_REF as u32 } else {
                    let (kind, unavailable) = self.wc.modality.get(&a0).copied().unwrap_or_else(|| (if (1..=4).contains(&self.class_of(a0)) { 2 } else { 0 }, 0));
                    self.put32(a1, kind, "GetWindowModality")?;
                    self.put32(a2, unavailable, "GetWindowModality")?;
                    0
                }
            }
            SetWindowModality => if valid(self, a0) { self.wc.modality.insert(a0, (a1, a2)); 0 } else { ERR_INVALID_WINDOW_REF as u32 },
            IsValidWindowPtr => u32::from(valid(self, a0)),
            SetWindowKind => { if valid(self, a0) { let _ = self.mac.mem.write_u16(a0 + WINDOW_KIND, a1 as u16); } 0 }
            InstallWindowContentPaintProc => {
                if !valid(self, a0) { ERR_INVALID_WINDOW_REF as u32 } else {
                    let refcon = a(self, 3).unwrap_or(0);
                    if a1 == 0 { self.wc.paint_proc.remove(&a0); } else if self.wc.paint_proc.insert(a0, (a1, a2, refcon)).is_none() {
                        self.mac.log.push(format!("[window] InstallWindowContentPaintProc {a0:#x} proc {a1:#x} options {a2:#x}: recorded, never called (this Window Manager does not erase or repaint content; exposures are the program's update events)"));
                    }
                    0
                }
            }
            RepositionWindow => { // (window, parentWindow, WindowPositionMethod)
                if !valid(self, a0) { ERR_INVALID_WINDOW_REF as u32 } else {
                    let m = &self.mac.mem;
                    let rd = |o: u32| m.read_i16(a0 + 16 + o).unwrap_or(0) as i32;
                    let (w, h) = (rd(6) - rd(2), rd(4) - rd(0));
                    let (sw, sh) = heirloom_mac68k_engine::lowmem::screen_size(self.mac.profile.color_qd);
                    let (sw, sh) = (i32::from(sw), i32::from(sh));
                    let menu_bar = 20;
                    let on_parent = matches!(a2, 2 | 5 | 8) && a1 != 0 && valid(self, a1);
                    let (ox, oy, ow, oh) = if on_parent {
                        let p = &self.mac.mem;
                        let prd = |o: u32| p.read_i16(a1 + 16 + o).unwrap_or(0) as i32;
                        let (gx, gy) = self.window_global_origin(a1);
                        (gx, gy, prd(6) - prd(2), prd(4) - prd(0))
                    } else { (0, menu_bar, sw, sh - menu_bar) };
                    let (x, y) = match a2 {
                        1..=3 => (ox + (ow - w) / 2, oy + (oh - h) / 2), // kWindowCenterOnMainScreen / OnParentWindow / OnParentWindowScreen
                        4..=6 => (ox + (ow - w) / 2, oy + (oh - h) / 3), // kWindowAlertPosition*
                        7..=9 => { self.mac.log.push(String::from("[window] RepositionWindow cascade: placed like a center (no cascade bookkeeping)")); (ox + (ow - w) / 2, oy + (oh - h) / 2) }
                        _ => { cpu.gpr[3] = PARAM_ERR as u32; return Ok(()) }
                    };
                    self.toolbox_call("MoveWindow", &[a0, x.max(0) as u32, y.max(menu_bar) as u32, 0]);
                    self.mac.log.push(format!("[window] RepositionWindow {a0:#x} method {a2} -> ({x},{y})"));
                    0
                }
            }
        };
        cpu.gpr[3] = r;
        Ok(())
    }

    /// The global top-left of a window's content (its port's portBits.bounds at 8, negated).
    fn window_global_origin(&self, w: u32) -> (i32, i32) {
        let m = &self.mac.mem;
        (-(m.read_i16(w + 10).unwrap_or(0) as i32), -(m.read_i16(w + 8).unwrap_or(0) as i32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use heirloom_ui_abi::RecordingGuestUi;
    type M = MacPpc<RecordingGuestUi>;
    const SP: u32 = 0x0007_0000;
    const R: u32 = 0x4000;

    fn call(m: &mut M, f: WcFn, args: &[u32]) -> u32 {
        let mut cpu = Cpu::new(0x0005_0000, SP);
        for (i, a) in args.iter().enumerate() { cpu.gpr[3 + i] = *a; }
        m.wc_call(f, &mut cpu).unwrap();
        cpu.gpr[3]
    }

    #[test]
    fn window_classes_front_windows_modality_and_reposition() {
        let mut m: M = MacPpc::new(RecordingGuestUi::new()).unwrap();
        m.implicit_graf();
        for (i, v) in [40i16, 40, 140, 240].iter().enumerate() { m.mac.mem.write_u16(R + 2 * i as u32, *v as u16).unwrap(); }
        assert_eq!(m.create_window(6, 0, R, R + 16, None).unwrap(), Some(0));
        assert_eq!(m.create_window(4, 0, R, R + 20, None).unwrap(), Some(0));
        let (doc, modal) = (m.mac.mem.read_u32(R + 16).unwrap(), m.mac.mem.read_u32(R + 20).unwrap());
        for w in [doc, modal] { m.toolbox_call("ShowWindow", &[w]); }
        assert_eq!(call(&mut m, WcFn::GetWindowClass, &[modal, R + 24]), 0);
        assert_eq!(m.mac.mem.read_u32(R + 24).unwrap(), 4, "kMovableModalWindowClass as created");
        assert_eq!(call(&mut m, WcFn::GetFrontWindowOfClass, &[6, 1]), doc);
        assert_eq!(call(&mut m, WcFn::GetFrontWindowOfClass, &[ALL_CLASSES, 1]), m.mac.z[0]);
        assert_eq!(call(&mut m, WcFn::GetWindowModality, &[modal, R + 28, R + 32]), 0);
        assert_eq!(m.mac.mem.read_u32(R + 28).unwrap(), 2, "a movable modal is app-modal");
        assert_eq!(call(&mut m, WcFn::GetWindowModality, &[doc, R + 28, R + 32]), 0);
        assert_eq!(m.mac.mem.read_u32(R + 28).unwrap(), 0, "a document window is not modal");
        assert_eq!(call(&mut m, WcFn::IsValidWindowPtr, &[doc]), 1);
        assert_eq!(call(&mut m, WcFn::IsValidWindowPtr, &[R]), 0, "NEGATIVE CONTROL: not a window");
        assert_eq!(call(&mut m, WcFn::RepositionWindow, &[modal, 0, 1]) as i32, 0);
        assert!(m.mac.log.iter().any(|l| l.contains("RepositionWindow") && l.contains("method 1")), "{:?}", m.mac.log.last());
        assert_eq!(call(&mut m, WcFn::RepositionWindow, &[modal, 0, 99]) as i32, PARAM_ERR, "NEGATIVE CONTROL: no such method");
    }

    #[test]
    fn a_content_paint_proc_is_recorded_and_named_never_called() {
        let mut m: M = MacPpc::new(RecordingGuestUi::new()).unwrap();
        m.implicit_graf();
        for (i, v) in [40i16, 40, 140, 240].iter().enumerate() { m.mac.mem.write_u16(R + 2 * i as u32, *v as u16).unwrap(); }
        assert_eq!(m.create_window(6, 0, R, R + 16, None).unwrap(), Some(0));
        let w = m.mac.mem.read_u32(R + 16).unwrap();
        assert_eq!(call(&mut m, WcFn::InstallWindowContentPaintProc, &[w, 0x5000, 0, 7]), 0);
        assert_eq!(m.wc.paint_proc.get(&w), Some(&(0x5000, 0, 7)));
        assert!(m.mac.log.iter().any(|l| l.contains("InstallWindowContentPaintProc") && l.contains("never called")));
        // NULL removes it; NEGATIVE CONTROL: not a window -> errInvalidWindowRef
        assert_eq!(call(&mut m, WcFn::InstallWindowContentPaintProc, &[w, 0, 0, 0]), 0);
        assert!(m.wc.paint_proc.is_empty());
        assert_eq!(call(&mut m, WcFn::InstallWindowContentPaintProc, &[R, 0x5000, 0, 7]) as i32, ERR_INVALID_WINDOW_REF);
    }

    /// HEIRLOOM-CUSTOM-WINDOW-NEEDS-A-BINDING: CreateCustomWindow with the program's own definition (a WindowDefUPP) or a procID of a
    /// program WDEF is refused by name, no window; a kWindowDefProcID spec naming a system WDEF (documentProc 0) makes the native window.
    #[test]
    fn create_custom_window_refuses_a_programs_own_definition_and_takes_a_system_proc_id() {
        let mut m: M = MacPpc::new(RecordingGuestUi::new()).unwrap();
        m.implicit_graf();
        for (i, v) in [40i16, 40, 140, 240].iter().enumerate() { m.mac.mem.write_u16(R + 2 * i as u32, *v as u16).unwrap(); }
        let custom = |m: &mut M, ty: u32, v: u32| -> (i32, u32) {
            m.mac.mem.write_u32(R + 40, ty).unwrap(); m.mac.mem.write_u32(R + 44, v).unwrap(); m.mac.mem.write_u32(R + 48, 0xDEAD).unwrap();
            let mut cpu = Cpu::new(0x0005_0000, SP);
            for (i, a) in [R + 40, 6, 0, R, R + 48].iter().enumerate() { cpu.gpr[3 + i] = *a; }
            m.carbon_call(crate::carbon::CarbonFn::CreateCustomWindow, &mut cpu, 0).unwrap();
            (cpu.gpr[3] as i32, m.mac.mem.read_u32(R + 48).unwrap())
        };
        let before = m.mac.windows.len();
        assert_eq!(custom(&mut m, 0, 0x1234), (-50, 0), "a WindowDefUPP: refused, no window");
        assert_eq!(custom(&mut m, 2, 34 * 16), (-50, 0), "procID of the program's WDEF 34: refused");
        assert_eq!(m.mac.windows.len(), before, "nothing was allocated");
        assert!(m.mac.refused.iter().all(|r| r.contains("[WDEF-UNSUPPORTED]")) && m.mac.refused.len() == 2, "{:?}", m.mac.refused);
        let (e, w) = custom(&mut m, 2, 0);
        assert_eq!(e, 0, "documentProc through the one table");
        assert!(m.mac.windows.contains_key(&w));
    }
}
