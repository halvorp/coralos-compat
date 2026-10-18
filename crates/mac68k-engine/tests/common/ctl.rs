//! The program fragments the Control Manager tests share: NewControl, then WaitNextEvent + GlobalToLocal, then FindControl +
//! TrackControl, with the results in fixed scratch cells.
#![allow(dead_code)]
use super::{Prog, SCRATCH};

pub const EV: u32 = 0x3200;
pub const WHICH: u32 = 0x3300;
pub const OUT: u32 = SCRATCH; // results: +0 ctl, +4 event what, +6 FindControl part, +8 TrackControl part, +10 value

/// NewControl(win, &RECT, &TITLE, visible, value, min, max, proc, 0) -> control handle, saved at `save`.
pub fn new_control(p: &mut Prog, win: u32, rect: u32, title: u32, value: i16, max: i16, proc_id: i16, save: u32) {
    new_control_ranged(p, win, rect, title, value, 0, max, proc_id, save);
}

/// NewControl with an explicit `minValue` (a pop-up's MENU resource ID).
pub fn new_control_ranged(p: &mut Prog, win: u32, rect: u32, title: u32, value: i16, min: i16, max: i16, proc_id: i16, save: u32) {
    p.clr_l().l(win).l(rect).l(title).b(1).w(value).w(min).w(max).w(proc_id).l(0).trap(0xA954).pop_l(save);
}

/// WaitNextEvent(everyEvent, &EV, 60, nil), then GlobalToLocal(EV.where).
pub fn wait_and_localise(p: &mut Prog, win: u32) {
    p.set_port(win).clr_w().w(-1).l(EV).l(60).l(0).trap(0xA860).pop_w(OUT + 4);
    p.l(EV + 10).trap(0xA871);
}

/// FindControl(EV.where, win, &WHICH) then TrackControl(which, EV.where, nil).
pub fn find_and_track(p: &mut Prog, win: u32) {
    p.clr_w().rgn_arg(EV + 10).l(win).l(WHICH).trap(0xA96C).pop_w(OUT + 6);
    p.clr_w().rgn_arg(WHICH).rgn_arg(EV + 10).l(0).trap(0xA968).pop_w(OUT + 8);
}

