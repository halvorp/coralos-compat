//! The update region: `InvalRect`/`InvalRgn`/`ValidRect`/`ValidRgn`, `BeginUpdate`/`EndUpdate`.
//!
//! A window's update region (LOCAL coordinates) is what needs redrawing. A window starts with its whole portRect
//! invalid (the Window Manager's `updateEvt` for it is queued when it opens or is resized). `BeginUpdate` narrows the
//! window's `visRgn` to `visRgn AND updateRgn` and empties the update region, so the program's ordinary drawing lands
//! only on the invalid part; `EndUpdate` restores the `visRgn`. `InvalRect` posts an `updateEvt` if none is queued.
use super::port::{raster, P_RECT, P_VIS};
use super::target::current_port;
use crate::events::UPDATE_EVT;
use crate::machine::{Call, Done, Mac};
use crate::pascal::{read_rect, Arg};
use crate::Refusal;
use alloc::format;
use heirloom_ui_abi::GuestUi;
use heirloom_cpu_68k::Cpu;
use heirloom_raster::Region;

impl<U: GuestUi> Mac<U> {
    fn require_window(&self, win: u32, what: &str) -> Result<(), Refusal> {
        if self.windows.contains_key(&win) { Ok(()) } else { Err(Refusal(format!("{what}: {win:#x} is not an open window"))) }
    }
    fn add_invalid(&mut self, win: u32, g: &Region) {
        self.qd.update.entry(win).or_default().or_region(g);
        if !self.ev.queue.iter().any(|e| e.what == UPDATE_EVT && e.message == win) { self.post(UPDATE_EVT, win, 0, 0); }
        let _ = self.sync_window_regions(win);
    }
}

/// The whole content of `win` needs redrawing (its updateEvt is queued by the caller).
pub(crate) fn invalidate_all<U: GuestUi>(m: &mut Mac<U>, win: u32) -> Result<(), Refusal> {
    let r = raster(read_rect(&m.mem, win.wrapping_add(P_RECT))?);
    m.qd.update.insert(win, Region::from_rect(r));
    m.sync_window_regions(win)
}

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    use super::reg;
    reg(m, 0xA922, "BeginUpDate", &[Arg::P], None, begin_update::<U>)?;
    reg(m, 0xA923, "EndUpDate", &[Arg::P], None, end_update::<U>)?;
    reg(m, 0xA928, "InvalRect", &[Arg::L], None, inval_rect::<U>)?;
    reg(m, 0xA92A, "ValidRect", &[Arg::L], None, valid_rect::<U>)?;
    reg(m, 0xA927, "InvalRgn", &[Arg::L], None, inval_rgn::<U>)?;
    reg(m, 0xA929, "ValidRgn", &[Arg::L], None, valid_rgn::<U>)
}

/// `BeginUpdate(win)`: `visRgn` narrows to `visRgn AND updateRgn` and the update region is emptied. Returns the narrowed region.
pub(crate) fn begin_update_of<U: GuestUi>(m: &mut Mac<U>, win: u32) -> Result<Region, Refusal> {
    m.require_window(win, "BeginUpdate")?;
    if m.qd.saved_vis.contains_key(&win) { return Err(Refusal(format!("BeginUpdate {win:#x}: already between BeginUpdate and EndUpdate"))); }
    let h = m.mem.read_u32(win.wrapping_add(P_VIS)).map_err(Refusal::mem("visRgn"))?;
    let vis = m.read_region(h)?;
    let mut narrowed = vis.clone();
    narrowed.and_region(&m.qd.update.remove(&win).unwrap_or_default());
    m.write_region(h, &narrowed)?;
    m.qd.saved_vis.insert(win, vis);
    m.sync_window_regions(win)?; // the update region was emptied
    Ok(narrowed)
}
/// `EndUpdate(win)`: the `visRgn` `BeginUpdate` narrowed comes back.
pub(crate) fn end_update_of<U: GuestUi>(m: &mut Mac<U>, win: u32) -> Result<(), Refusal> {
    m.require_window(win, "EndUpdate")?;
    let vis = m.qd.saved_vis.remove(&win).ok_or_else(|| Refusal(format!("EndUpdate {win:#x}: no matching BeginUpdate")))?;
    let h = m.mem.read_u32(win.wrapping_add(P_VIS)).map_err(Refusal::mem("visRgn"))?;
    m.write_region(h, &vis)
}

fn begin_update<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let win = super::target::checked_port(c.args.get(0))?;
    let narrowed = begin_update_of(m, win)?;
    Ok(Done { result: 0, log: format!("[quickdraw] BeginUpdate {win:#x}: visRgn narrowed to {} rect(s)", narrowed.rects().len()) })
}
fn end_update<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let win = super::target::checked_port(c.args.get(0))?;
    end_update_of(m, win)?;
    Ok(Done { result: 0, log: format!("[quickdraw] EndUpdate {win:#x}") })
}

fn rect_region<U: GuestUi>(m: &Mac<U>, a: u32) -> Result<Region, Refusal> { Ok(Region::from_rect(raster(read_rect(&m.mem, a)?))) }

fn inval<U: GuestUi>(m: &mut Mac<U>, g: Region, name: &str) -> Result<Done, Refusal> {
    let win = current_port(m)?;
    m.require_window(win, name)?;
    m.add_invalid(win, &g);
    Ok(Done { result: 0, log: format!("[quickdraw] {name} {win:#x}") })
}
fn valid<U: GuestUi>(m: &mut Mac<U>, g: Region, name: &str) -> Result<Done, Refusal> {
    let win = current_port(m)?;
    m.require_window(win, name)?;
    if let Some(u) = m.qd.update.get_mut(&win) { u.clear_region(&g); }
    m.sync_window_regions(win)?;
    Ok(Done { result: 0, log: format!("[quickdraw] {name} {win:#x}") })
}
fn inval_rect<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { let g = rect_region(m, c.args.get(0))?; inval(m, g, "InvalRect") }
fn valid_rect<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { let g = rect_region(m, c.args.get(0))?; valid(m, g, "ValidRect") }
fn inval_rgn<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { let g = m.read_region(c.args.get(0))?; inval(m, g, "InvalRgn") }
fn valid_rgn<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { let g = m.read_region(c.args.get(0))?; valid(m, g, "ValidRgn") }
