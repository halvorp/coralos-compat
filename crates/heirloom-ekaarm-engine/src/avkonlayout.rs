//! `AknLayoutUtils::LayoutMetricsRect` / `Size` / `Position`: avkon's answer to "where is the main pane / status pane / control pane"
//! (uifw/AvKon/src/aknutils.cpp:3405-3860 in the Symbian Foundation classic-UI source, EPL-1.0, consulted as reference), natively over
//! the CDL layout data (`cdl.rs`). The branches follow the source; one that depends on state this engine does not model yet is a named
//! refusal, not a guess.
use alloc::format;
use alloc::string::String;
use crate::cdl::CdlState;
use crate::layoutmath::{compose, layout_rect, Rect};
use crate::loader::Process;
use crate::natives::NativeOutcome;

/// `AknLayoutUtils::TAknLayoutMetrics` (AknUtils.h:1960-2022), by value.
const E_SCREEN: u32 = 0;
const E_APPLICATION_WINDOW: u32 = 1;
const E_STATUS_PANE: u32 = 2;
const E_MAIN_PANE: u32 = 3;
const E_CONTROL_PANE: u32 = 4;
const E_NAVI_PANE: u32 = 10;
const E_POPUP_PARENT: u32 = 22;

const AVKON_LAYOUT: &str = "AknLayoutScalable_Avkon";
const META_DATA: &str = "Layout_Meta_Data";

impl CdlState {
    /// The UID of the interface called `name`.
    pub fn uid_of(&self, name: &str) -> Result<u32, String> {
        let data = self.data.as_ref().ok_or_else(|| String::from("no CDL layout data was handed to the engine"))?;
        data.interfaces.iter().find(|(_, (n, _))| n == name).map(|(u, _)| *u).ok_or_else(|| format!("no interface {name} in the layout data"))
    }

    /// A `Layout_Meta_Data` flag (`IsLandscapeOrientation`, `IsPenEnabled`, ...): the constant of the top-most instance implementing it.
    pub fn meta(&self, name: &str) -> Result<bool, String> {
        let uid = self.uid_of(META_DATA)?;
        let api = self.api_id(uid, name)?;
        let inst = self.implementor(uid, api)?;
        match self.data.as_ref().and_then(|d| d.instances[inst].ops.get(api)) {
            Some(crate::cdl::Op::Const(v)) => Ok(*v != 0),
            _ => Err(format!("{META_DATA}::{name} is not a constant in the layout data")),
        }
    }

    /// `AknLayoutScalable_Avkon::<name>(variety)`.
    fn avkon(&self, name: &str, variety: i32) -> Result<crate::layoutdecode::Words, String> {
        self.window_layout(self.uid_of(AVKON_LAYOUT)?, name, variety)
    }

    fn rsg(&self, name: &str) -> Result<u32, String> {
        self.data.as_ref().and_then(|d| d.rsg.get(name).copied()).ok_or_else(|| format!("the layout data file lacks the avkon resource id {name} (pass avkon.rsg to gen_cdl_layout)"))
    }
}

/// The status-pane layout kinds `LayoutMetricsRect` tells apart (the `R_AVKON_*` ids compared in aknutils.cpp).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pane {
    Usual,
    Idle,
    Small,
    Flat,
    Empty,
    Stacon,
    Widescreen,
}

fn classify(cdl: &CdlState, id: u32) -> Result<Pane, String> {
    let is = |names: &[&str]| -> Result<bool, String> {
        for n in names {
            if cdl.rsg(n)? == id { return Ok(true) }
        }
        Ok(false)
    };
    Ok(if is(&["R_AVKON_STATUS_PANE_LAYOUT_USUAL_WITH_BATTERY_PANE", "R_AVKON_STATUS_PANE_LAYOUT_USUAL", "R_AVKON_STATUS_PANE_LAYOUT_POWER_OFF_RECHARGE", "R_AVKON_STATUS_PANE_LAYOUT_USUAL_MIRRORED", "R_AVKON_STATUS_PANE_LAYOUT_POWER_OFF_RECHARGE_MIRRORED", "R_AVKON_STATUS_PANE_LAYOUT_VT", "R_AVKON_STATUS_PANE_LAYOUT_VT_MIRRORED", "R_AVKON_STATUS_PANE_LAYOUT_USUAL_EXT"])? {
        Pane::Usual
    } else if is(&["R_AVKON_STATUS_PANE_LAYOUT_IDLE", "R_AVKON_STATUS_PANE_LAYOUT_IDLE_MIRRORED"])? {
        Pane::Idle
    } else if is(&["R_AVKON_STATUS_PANE_LAYOUT_SMALL", "R_AVKON_STATUS_PANE_LAYOUT_SMALL_WITH_SIGNAL_PANE", "R_AVKON_STATUS_PANE_LAYOUT_SMALL_WITH_SIGNAL_PANE_MIRRORED"])? {
        Pane::Small
    } else if is(&["R_AVKON_STATUS_PANE_LAYOUT_USUAL_FLAT", "R_AVKON_STATUS_PANE_LAYOUT_IDLE_FLAT"])? {
        Pane::Flat
    } else if is(&["R_AVKON_STATUS_PANE_EMPTY", "R_AVKON_STATUS_PANE_LAYOUT_EMPTY"])? {
        Pane::Empty
    } else if is(&["R_AVKON_STACON_PANE_LAYOUT_USUAL_SOFTKEYS_RIGHT", "R_AVKON_STACON_PANE_LAYOUT_USUAL_SOFTKEYS_LEFT", "R_AVKON_STACON_PANE_LAYOUT_EMPTY_SOFTKEYS_RIGHT", "R_AVKON_STACON_PANE_LAYOUT_EMPTY_SOFTKEYS_LEFT", "R_AVKON_STACON_PANE_LAYOUT_IDLE_SOFTKEYS_RIGHT", "R_AVKON_STACON_PANE_LAYOUT_IDLE_SOFTKEYS_LEFT"])? {
        Pane::Stacon
    } else if is(&["R_AVKON_WIDESCREEN_PANE_LAYOUT_USUAL", "R_AVKON_WIDESCREEN_PANE_LAYOUT_IDLE", "R_AVKON_WIDESCREEN_PANE_LAYOUT_USUAL_FLAT", "R_AVKON_WIDESCREEN_PANE_LAYOUT_IDLE_FLAT", "R_AVKON_WIDESCREEN_PANE_LAYOUT_USUAL_FLAT_NO_SOFTKEYS", "R_AVKON_WIDESCREEN_PANE_LAYOUT_IDLE_FLAT_NO_SOFTKEYS"])? {
        Pane::Widescreen
    } else {
        // The source's `default:` arms are the "usual" ones (status pane) and variety 3 (main pane).
        Pane::Usual
    })
}

/// `AknLayout::screen()` (the adaptation layer): `AknLayoutScalable_Avkon::Screen()` with l and t forced to 0.
fn screen_rect(cdl: &CdlState) -> Result<Rect, String> {
    let l = crate::layoutmath::layout_line(&cdl.avkon("Screen", 0)?);
    Ok(Rect::new(0, 0, l.w, l.h))
}

/// The main pane of the current status pane layout (or, with `whole_window`, the application window), when the layout data is loaded and the app switched the layout: what
/// `CAknAppUi::ClientRect()` (the application rect less the status pane and the CBA) comes to in a portrait layout.
pub(crate) fn client_rect(p: &Process, whole_window: bool) -> Option<Result<Rect, String>> {
    if p.nat.cdl.data.is_none() || p.nat.shell.layout.is_none() {
        return None;
    }
    Some(metrics_rect(p, if whole_window { E_APPLICATION_WINDOW } else { E_MAIN_PANE }).and_then(|r| r.ok_or_else(|| String::from("no main pane"))))
}

/// `LayoutMetricsRect`: `Ok(None)` = the function returned EFalse (rect left 0,0,0,0).
fn metrics_rect(p: &Process, param: u32) -> Result<Option<Rect>, String> {
    let cdl = &p.nat.cdl;
    let screen = screen_rect(cdl)?;
    let layout_id = p.nat.shell.layout.ok_or_else(|| String::from("the app never switched the status pane layout (the initial layout is the application's core resource, which is not read)"))?;
    let pane = classify(cdl, layout_id)?;
    let landscape = cdl.meta("IsLandscapeOrientation")?;
    let pen = cdl.meta("IsPenEnabled")?;
    let rect = |parent: Rect, w: crate::layoutdecode::Words| layout_rect(parent, &w);
    Ok(Some(match param {
        E_SCREEN => screen,
        E_APPLICATION_WINDOW => rect(screen, cdl.avkon("application_window", 0)?),
        E_STATUS_PANE => {
            if pane == Pane::Stacon || pane == Pane::Widescreen {
                return Err(format!("EStatusPane in the {pane:?} status pane layout ({layout_id:#x})"));
            }
            let (parent, line) = match pane {
                Pane::Idle => (cdl.avkon("area_top_pane", 7)?, cdl.avkon("status_idle_pane", 0)?),
                Pane::Small => (cdl.avkon("application_window", 0)?, cdl.avkon("area_top_pane", if landscape { return Err(String::from("EStatusPane, small layout, landscape: CbaLocation is not modelled")) } else { 1 })?),
                Pane::Flat => (cdl.avkon("area_top_pane", if landscape && pen { 2 } else { 6 })?, cdl.avkon("status_pane", 1)?),
                _ => (cdl.avkon("area_top_pane", 0)?, cdl.avkon("status_pane", 0)?),
            };
            rect(screen, compose(parent, line))
        }
        E_POPUP_PARENT => {
            let (w, h) = (screen.width(), screen.height());
            let variety = if (w, h) == (640, 360) {
                return Err(String::from("EPopupParent on the 640x360 screen needs the HD status pane layout"));
            } else if (w, h) == (360, 640) {
                1
            } else {
                return metrics_rect(p, E_MAIN_PANE);
            };
            rect(screen, compose(cdl.avkon("application_window", 0)?, cdl.avkon("main_pane", variety)?))
        }
        E_MAIN_PANE => {
            let variety = match pane {
                Pane::Stacon => return Err(format!("EMainPane in a stacon layout ({layout_id:#x}): the fixed toolbar's visibility decides the variety")),
                // aknutils.cpp:3482-3512 (RD_SCALABLE_UI_V2): the touch pane and the fixed toolbar (flags, IsShown) pick varieties 15 and 18.
                Pane::Usual => return Err(format!("EMainPane in the usual layout ({layout_id:#x}): CAknAppUi::TouchPane()/CurrentFixedToolbar()->ToolbarFlags() are not modelled")),
                Pane::Idle => 8,
                Pane::Flat if landscape => return Err(String::from("EMainPane, flat layout, landscape: toolbar visibility is not modelled")),
                Pane::Flat => 7,
                Pane::Small if landscape => if pen { 4 } else { 9 },
                Pane::Small => 6,
                Pane::Empty => 1,
                Pane::Widescreen => 3,
            };
            rect(screen, compose(cdl.avkon("application_window", 0)?, cdl.avkon("main_pane", variety)?))
        }
        E_CONTROL_PANE => {
            if pane == Pane::Stacon || pane == Pane::Widescreen || (landscape && pane == Pane::Flat) {
                return Err(format!("EControlPane in the {pane:?} layout, landscape {landscape}"));
            }
            rect(screen, cdl.avkon("area_bottom_pane", 0)?)
        }
        // Status pane descendants: `CEikStatusPaneBase::PaneRectL(id)` reads the rectangle from the current layout's tree
        // (eikspmod.cpp `AknLayoutRect`). In `R_AVKON_STATUS_PANE_LAYOUT_SMALL` the navi pane gets the whole small status pane (the
        // top area, variety 1, in the application window: eikspmod.cpp:1649).
        E_NAVI_PANE if layout_id == cdl.rsg("R_AVKON_STATUS_PANE_LAYOUT_SMALL")? && !landscape => {
            let app = rect(screen, cdl.avkon("application_window", 0)?);
            rect(app, cdl.avkon("area_top_pane", 1)?)
        }
        other => return Err(format!("layout metric {other} in layout {layout_id:#x}: the status pane layout tree (STATUS_PANE_LAYOUT resource) is not read")),
    }))
}

/// Reads and writes of the guest's `TRect` (iTl, iBr: four ints), `TSize` (two) and `TPoint` (two).
pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let (param, out) = (p.cpu.regs[0], p.cpu.regs[1]);
    let write = |p: &mut Process, vals: &[i32]| vals.iter().enumerate().all(|(i, v)| p.mem.vm.write_u32(out.wrapping_add(i as u32 * 4), *v as u32).is_ok());
    let kind = match name {
        "_ZN14AknLayoutUtils17LayoutMetricsRectENS_17TAknLayoutMetricsER5TRect" => 0,
        "_ZN14AknLayoutUtils17LayoutMetricsSizeENS_17TAknLayoutMetricsER5TSize" => 1,
        "_ZN14AknLayoutUtils21LayoutMetricsPositionENS_17TAknLayoutMetricsER6TPoint" => 2,
        _ => return None,
    };
    let refuse = |m: String| NativeOutcome::Refused(format!("NEXT-NEEDED avkon {name}({param}): {m}"));
    let r = match metrics_rect(p, param) {
        Ok(r) => r,
        Err(m) => return Some(refuse(m)),
    };
    let rc = r.unwrap_or_default();
    let ok = match kind {
        0 => write(p, &[rc.x0, rc.y0, rc.x1, rc.y1]),
        1 => write(p, &[rc.width(), rc.height()]),
        _ => write(p, &[rc.x0, rc.y0]),
    };
    if !ok {
        return Some(refuse(format!("the result at {out:#x} is unwritable")));
    }
    p.cpu.regs[0] = r.is_some() as u32;
    Some(NativeOutcome::Return)
}
