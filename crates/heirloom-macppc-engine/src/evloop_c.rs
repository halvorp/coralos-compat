//! The Carbon Event Loop (`GetMainEventLoop`, `InstallEventLoopTimer`, ...): the timer queue installed before the first GL call.
//! Timers fire at safepoints (glue calls with no callback in progress), same rule as the Time Manager.
//! The main loop is a single opaque ref shared by all threads in this engine.

use crate::callback::{Continuation, Frame, Saved};
use crate::clib::arg;
use crate::cpu::Cpu;
use crate::machine::MacPpc;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use heirloom_ui_abi::GuestUi;

const MAIN_LOOP: u32 = 0x0051_0001;
const PARAM_ERR: i32 = -50;
const NO_ERR: i32 = 0;
const ID_BASE: u32 = 0x0052_0000; // distinct from Carbon's 0x0050_0000
const DISARMED: u64 = u64::MAX;

/// Virtual clock in microseconds (60Hz ticks → µs).
fn now_us(m: &MacPpc<impl GuestUi>) -> u64 {
    m.mac.ev.ticks_total.wrapping_mul(1_000_000) / 60
}

fn deadline(now: u64, seconds: f64) -> u64 {
    if seconds < 0.0 { DISARMED } else { now.saturating_add((seconds * 1e6) as u64) }
}

#[derive(Default)]
pub struct ElState {
    timers: BTreeMap<u32, ElTimer>,
    next: u32,
    /// The earliest due time of an ARMED timer (None: none armed), recomputed at every change of `timers`: every glue call is a
    /// safepoint, and this lets one that has nothing due return without walking the timers.
    next_due: Option<u64>,
}

impl ElState {
    fn rescan(&mut self) { self.next_due = self.timers.values().map(|t| t.due_us).filter(|&d| d != DISARMED).min(); }
}

#[derive(Clone, Copy)]
struct ElTimer {
    due_us: u64,
    interval_us: i64, // <=0 = one-shot (disarmed after fire)
    proc: u32,
    data: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElFn {
    GetMainEventLoop,
    GetCurrentEventLoop,
    InstallEventLoopTimer,
    RemoveEventLoopTimer,
    SetEventLoopTimerNextFireTime,
    GetEventTime,
}

impl ElFn {
    pub fn by_name(name: &str) -> Option<ElFn> {
        Some(match name {
            "GetMainEventLoop" => ElFn::GetMainEventLoop,
            "GetCurrentEventLoop" => ElFn::GetCurrentEventLoop,
            "InstallEventLoopTimer" => ElFn::InstallEventLoopTimer,
            "RemoveEventLoopTimer" => ElFn::RemoveEventLoopTimer,
            "SetEventLoopTimerNextFireTime" => ElFn::SetEventLoopTimerNextFireTime,
            "GetEventTime" => ElFn::GetEventTime,
            _ => return None,
        })
    }
}

impl<U: GuestUi> MacPpc<U> {
    fn el_next_id(&mut self) -> u32 {
        loop {
            self.el.next = self.el.next.wrapping_add(2);
            let id = ID_BASE.wrapping_add(self.el.next) | 1;
            if !self.el.timers.contains_key(&id) { return id; }
        }
    }

    pub(crate) fn el_call(&mut self, f: ElFn, cpu: &mut Cpu) -> Result<(), String> {
        use ElFn::*;
        let r: i32 = match f {
            GetMainEventLoop | GetCurrentEventLoop => {
                cpu.gpr[3] = MAIN_LOOP;
                return Ok(());
            }
            InstallEventLoopTimer => {
                // The doubles occupy GPR parameter slots even though their values arrive in FPRs.
                let loop_ref = cpu.gpr[3];
                let fire_delay = f64::from_bits(cpu.fpr[1]);
                let interval = f64::from_bits(cpu.fpr[2]);
                let proc = cpu.gpr[8];
                let data = cpu.gpr[9];
                let out_timer = cpu.gpr[10];
                if loop_ref != MAIN_LOOP || proc == 0 {
                    cpu.gpr[3] = PARAM_ERR as u32;
                    return Ok(());
                }
                let id = self.el_next_id();
                let due_us = deadline(now_us(self), fire_delay);
                let interval_us = (interval * 1e6) as i64;
                if out_timer != 0 {
                    self.put32(out_timer, id, "InstallEventLoopTimer")?;
                }
                self.el.timers.insert(id, ElTimer { due_us, interval_us, proc, data });
                self.el.rescan();
                NO_ERR
            }
            RemoveEventLoopTimer => {
                let timer = arg(self, cpu, 0)?;
                if self.el.timers.remove(&timer).is_some() { self.el.rescan(); NO_ERR } else { PARAM_ERR }
            }
            SetEventLoopTimerNextFireTime => {
                let timer = arg(self, cpu, 0)?;
                let seconds = f64::from_bits(cpu.fpr[1]);
                let due_us = deadline(now_us(self), seconds);
                if let Some(t) = self.el.timers.get_mut(&timer) {
                    t.due_us = due_us;
                    self.el.rescan();
                    NO_ERR
                } else { PARAM_ERR }
            }
            GetEventTime => {
                let event_ref = arg(self, cpu, 0)?;
                // an EventRef the Carbon Event Manager made (crate::carbon); its time is the engine's tick clock (events carry no own stamp)
                if self.rt.carbon.is_event(event_ref) {
                    cpu.fpr[1] = (f64::from(self.mac.ev.ticks) / 60.0).to_bits();
                } else {
                    self.mac.log.push(format!("[evloop] GetEventTime: {event_ref:#x} is not an EventRef this engine made"));
                    cpu.fpr[1] = 0.0_f64.to_bits();
                }
                return Ok(());
            }
        };
        cpu.gpr[3] = r as u32;
        Ok(())
    }

    pub(crate) fn evloop_safepoint(&mut self, cpu: &mut Cpu, call_pc: u32) -> Result<bool, String> {
        if !self.rt.callbacks.is_empty() { return Ok(false); }
        let now = now_us(self);
        // nothing armed is due (exact: the walk below finds a timer only when the earliest armed due time has come)
        if self.el.next_due.is_none_or(|d| d > now) { return Ok(false); }
        let due_id = self.el.timers.iter()
            .filter(|(_, t)| t.due_us != DISARMED && t.due_us <= now)
            .min_by_key(|(_, t)| t.due_us)
            .map(|(&id, _)| id);
        let Some(timer_id) = due_id else { return Ok(false); };
        let timer = self.el.timers[&timer_id];

        // Re-arm BEFORE entering proc: TimerTask returns to this same glue safepoint.
        // Skip missed periods arithmetically, preserving phase without a catch-up loop.
        let mut missed = 1;
        let due_us = if timer.interval_us > 0 {
            let interval = timer.interval_us as u64;
            let elapsed = now - timer.due_us;
            missed = elapsed / interval + 1;
            now.saturating_add(interval - elapsed % interval)
        } else {
            // Keep the ref valid for Remove and SetNextFireTime after a one-shot fires.
            DISARMED
        };
        if let Some(t) = self.el.timers.get_mut(&timer_id) { t.due_us = due_us; }
        self.el.rescan();
        self.mac.log.push(format!("[evloop] timer {timer_id:#x} fires (missed {missed})"));

        let saved = Saved::of(cpu, call_pc);
        let sp = cpu.gpr[1];
        let r = if heirloom_mac68k_engine::mixed_mode::parse(&self.mac.mem, timer.proc).is_ok() {
            self.enter_upp(cpu, timer.proc, &[timer_id, timer.data], sp)
        } else {
            self.enter_tvector(cpu, timer.proc, &[timer_id, timer.data], sp)
        };
        r.map_err(|e| format!("Event Loop timer {timer_id:#x}: {e}"))?;
        self.rt.callbacks.push(Frame { saved, cont: Continuation::TimerTask });
        Ok(true)
    }
}