//! The Time Manager (`InsTime`, `InsXTime`, `PrimeTime`, `RmvTime`), natively, from the Stage-A spec macos9-specs/TimeManager/{InsTime,PrimeTime,RmvTime}: the TMTask record stays in
//! guest memory with the real `qType` bits (0x8000 active, 0x4000 extended, 0x2000 the other trap-flag bit) and `tmCount` / `tmWakeUp`, because programs read them back after
//! `RmvTime`. Gestalt `'tmgr'` answers 4 (the native Time Manager) with the rest of the PowerPC profile.
//! * `InsTime` clears every `qType` bit above the low five and sets 0x4000 for `InsXTime`; it does not start the task. More than 384 inserted tasks: -1.
//! * `PrimeTime(task, count)`: ignored (0) on an active task; -1 on one never inserted; else `tmCount` = count, active, and the wake time is now + count (positive = milliseconds, negative
//!   = microseconds); an EXTENDED task primed again keeps its cadence (previous wake + count) and `tmWakeUp` holds the wake time in milliseconds of virtual uptime.
//! * `RmvTime`: -1 for a task that is not inserted; else `qType &= 0x1F`, `tmCount` = the time left (an active task) or 0, and the task is forgotten.
//! * Expiry: when the virtual clock (60 Hz ticks) reaches a primed task's wake time, the next SAFEPOINT -- a Toolbox/CFM glue call made by the program with no callback in progress --
//!   clears the active bit and calls the task's `tmAddr` UPP with the TMTask pointer in r3 (the PowerPC form of the 68k A1 = task), then resumes the interrupted call. NAMED LIMITS:
//!   tasks run at a safepoint, NOT at interrupt time (a program that never calls the system is never interrupted), the clock has 1/60 s resolution (a 1 ms period fires on every
//!   tick), and tasks do not nest.
use crate::callback::{Continuation, Frame, Saved};
use crate::clib::arg;
use crate::machine::MacPpc;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

const Q_TYPE: u32 = 4;
const TM_ADDR: u32 = 6;
const TM_COUNT: u32 = 0xA;
const TM_WAKE_UP: u32 = 0xE;
const ACTIVE: u16 = 0x8000;
const EXTENDED: u16 = 0x4000;
const MAX_TASKS: usize = 384;

#[derive(Default)]
pub struct TmState { tasks: BTreeMap<u32, Task> }
#[derive(Clone, Copy, Default)]
struct Task { extended: bool, wake_tick: Option<u64>, prev_wake_tick: u64 }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TmFn { InsTime, InsXTime, PrimeTime, RmvTime }

impl TmFn {
    pub fn by_name(name: &str) -> Option<TmFn> {
        Some(match name { "InsTime" => TmFn::InsTime, "InsXTime" => TmFn::InsXTime, "PrimeTime" => TmFn::PrimeTime, "RmvTime" => TmFn::RmvTime, _ => return None })
    }
}

/// Ticks (1/60 s) for a Duration: positive milliseconds, negative microseconds; at least one tick for a non-zero count.
fn ticks_of(count: i32) -> u64 {
    if count == 0 { return 0; }
    let ms = if count > 0 { i64::from(count) * 1000 } else { -i64::from(count) }; // in microseconds
    ((ms * 60 + 999_999) / 1_000_000).max(1) as u64
}

impl<U: GuestUi> MacPpc<U> {
    pub(crate) fn tm_call(&mut self, f: TmFn, cpu: &mut Cpu) -> Result<(), String> {
        let (task, count) = (arg(self, cpu, 0)?, arg(self, cpu, 1).unwrap_or(0) as i32);
        let n = format!("{f:?}");
        let now = self.mac.ev.ticks_total;
        let rd16 = |s: &Self, at: u32| s.mac.mem.read_u16(at).map_err(|_| format!("{n}: the TMTask at {task:#x} leaves guest RAM"));
        let r: i32 = match f {
            TmFn::InsTime | TmFn::InsXTime => {
                if !self.tm.tasks.contains_key(&task) && self.tm.tasks.len() >= MAX_TASKS { -1 } else {
                    let q = rd16(self, task + Q_TYPE)?;
                    let ext = f == TmFn::InsXTime;
                    self.mac.mem.write_u16(task + Q_TYPE, (q & 0x1F) | if ext { EXTENDED } else { 0 }).map_err(|_| format!("{n}: the TMTask leaves guest RAM"))?;
                    let wake = if ext { self.mac.mem.read_u32(task + TM_WAKE_UP).unwrap_or(0) } else { 0 };
                    self.tm.tasks.insert(task, Task { extended: ext, wake_tick: None, prev_wake_tick: if wake != 0 { u64::from(wake) * 60 / 1000 } else { 0 } });
                    0
                }
            }
            TmFn::PrimeTime => {
                let q = rd16(self, task + Q_TYPE)?;
                if q & ACTIVE != 0 { 0 } else if let Some(t) = self.tm.tasks.get(&task).copied() {
                    let wake = if t.extended && t.prev_wake_tick != 0 { t.prev_wake_tick + ticks_of(count) } else { now + ticks_of(count) };
                    self.mac.mem.write_u32(task + TM_COUNT, count as u32).map_err(|_| format!("{n}: the TMTask leaves guest RAM"))?;
                    self.mac.mem.write_u16(task + Q_TYPE, q | ACTIVE).map_err(|_| format!("{n}: the TMTask leaves guest RAM"))?;
                    if t.extended { self.mac.mem.write_u32(task + TM_WAKE_UP, (wake * 1000 / 60) as u32).map_err(|_| format!("{n}: the TMTask leaves guest RAM"))?; }
                    self.tm.tasks.insert(task, Task { wake_tick: Some(wake), prev_wake_tick: if t.extended { wake } else { 0 }, ..t });
                    0
                } else { -1 }
            }
            TmFn::RmvTime => match self.tm.tasks.remove(&task) {
                None => -1,
                Some(t) => {
                    let q = rd16(self, task + Q_TYPE)?;
                    let left = if q & ACTIVE != 0 { t.wake_tick.map_or(0, |w| w.saturating_sub(now)) * 1000 / 60 } else { 0 };
                    self.mac.mem.write_u16(task + Q_TYPE, q & 0x1F).and_then(|_| self.mac.mem.write_u32(task + TM_COUNT, left as u32)).map_err(|_| format!("{n}: the TMTask leaves guest RAM"))?;
                    if t.extended { let w = t.wake_tick.map_or(0, |w| (w * 1000 / 60) as u32); self.mac.mem.write_u32(task + TM_WAKE_UP, w).map_err(|_| format!("{n}: the TMTask leaves guest RAM"))?; }
                    0
                }
            },
        };
        cpu.gpr[3] = r as u32;
        Ok(())
    }

    /// The earliest wake tick among primed tasks (so a parked program wakes for it).
    pub(crate) fn tm_next_wake(&self) -> Option<u64> { self.tm.tasks.values().filter_map(|t| t.wake_tick).min() }

    /// A primed task whose wake time has come: its pointer (the active bit is cleared when it is taken).
    fn tm_take_due(&mut self) -> Option<u32> {
        let now = self.mac.ev.ticks_total;
        let due = self.tm.tasks.iter().filter(|(_, t)| t.wake_tick.is_some_and(|w| w <= now)).min_by_key(|(_, t)| t.wake_tick).map(|(&p, _)| p)?;
        if let Some(t) = self.tm.tasks.get_mut(&due) { t.wake_tick = None; }
        if let Ok(q) = self.mac.mem.read_u16(due + Q_TYPE) { let _ = self.mac.mem.write_u16(due + Q_TYPE, q & !ACTIVE); }
        Some(due)
    }

    /// At a safepoint (`call_pc` is the glue call about to run): if a task is due, save the interrupted call and enter the task's procedure. `Ok(true)` = the CPU now runs the task.
    pub(crate) fn tm_safepoint(&mut self, cpu: &mut Cpu, call_pc: u32) -> Result<bool, String> {
        if !self.rt.callbacks.is_empty() { return Ok(false); }
        while let Some(task) = self.tm_take_due() {
            let upp = self.mac.mem.read_u32(task + TM_ADDR).unwrap_or(0);
            if upp == 0 { continue; }
            let saved = Saved::of(cpu, call_pc);
            let sp = cpu.gpr[1];
            self.enter_upp(cpu, upp, &[task], sp).map_err(|e| format!("Time Manager task {task:#x}: {e}"))?;
            self.mac.log.push(format!("[time] task {task:#x} due: its procedure runs at this safepoint"));
            self.rt.callbacks.push(Frame { saved, cont: Continuation::TimerTask });
            return Ok(true);
        }
        Ok(false)
    }
}
