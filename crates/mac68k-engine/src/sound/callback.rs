//! `callBackCmd`: a channel's completion routine (Inside Macintosh VI "Sound Manager": `callBackCmd`, `SndCallBackProcPtr`).
//!
//! The command goes in the channel's queue after the sounds already given; when those have PLAYED, the Sound Manager calls the routine in
//! the channel's `callBack` field (set by `SndNewChannel`) as `pascal void (SndChannelPtr chan, SndCommand *cmd)`, with `cmd` carrying the
//! command's `param1` and `param2`: games use it to know a sound ended. "Played" is the HOST's clock: the engine counts the bytes it
//! queued on each of the channel's streams and asks the seam how many are still unplayed ([`guestui_abi::GuestUi::audio_queued_bytes`]).
//!
//! The routine runs at a trap boundary, the way a vertical-retrace task does ([`crate::vbl`]): every register and the SR are saved, the
//! two Pascal arguments are pushed under a return into the completion trampoline, and the routine's own `RTD` lands on it, which
//! puts the registers back and returns to the trap that was about to run. A channel with no `callBack` routine ignores the command.
//! NAMED LIMIT: the routine runs at the next Toolbox trap, not at interrupt time; a program that never traps again never sees it.
use super::*;
use crate::guestcall::{Frame, TRAMPOLINE};
use crate::vbl::VblDone;
use guestui_abi::UiObjectRef;

/// A completion routine that is due: the channel, the routine (a ProcPtr for the 68k, a routine descriptor for PowerPC) and the `SndCommand` it is called with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DueCallback { pub chan: u32, pub routine: u32, pub cmd: u32 }

#[derive(Debug, Clone)]
pub struct Pending { chan: u32, routine: u32, p1: i16, p2: u32, waits: Vec<(UiObjectRef, u64)> }

impl<U: GuestUi> Mac<U> {
    /// `callBackCmd`: remember it until what is queued now has played.
    pub(super) fn snd_queue_callback(&mut self, key: u32, c: snd::Cmd) -> Result<(i16, String), Refusal> {
        let routine = self.mem.read_u32(key.wrapping_add(SC_CALL_BACK)).map_err(Refusal::mem("SndChannel.callBack"))?;
        if routine == 0 { return Ok((NO_ERR, String::from("callBackCmd (the channel has no callBack routine: ignored)"))); }
        let waits = self.snd.chans.get(&key).map(|ch| ch.streams.iter().map(|s| (s.r, self.snd.queued_total.get(&s.r.id).copied().unwrap_or(0))).collect()).unwrap_or_default();
        self.snd.pending.push(Pending { chan: key, routine, p1: c.param1, p2: c.param2, waits });
        Ok((NO_ERR, format!("callBackCmd queued: routine {routine:#x} runs when the sound queued so far has played (param1 {}, param2 {:#x})", c.param1, c.param2)))
    }

    fn snd_played(&mut self, r: UiObjectRef, total_at: u64) -> bool {
        match self.ui.audio_queued_bytes(r) {
            Err(_) => true, // the stream is gone: nothing left to wait for
            Ok(unplayed) => self.snd.queued_total.get(&r.id).copied().unwrap_or(0).saturating_sub(u64::from(unplayed)) >= total_at,
        }
    }

    /// If a completion routine is due (what it waited on has played, its channel is alive, none is already running), take it: the command record
    /// it is called with is allocated and `in_callback` set. The processor-specific adapters ([`enter_sound_callback`](Self::enter_sound_callback)
    /// for the 68k, the PowerPC engine's safepoint) call the routine; both end in [`finish_snd_callback`](Self::finish_snd_callback). Due routines
    /// are taken in the order the guest queued them.
    pub fn snd_take_due(&mut self) -> Option<DueCallback> {
        if self.snd.pending.is_empty() || self.snd.in_callback { return None; }
        let mut due = None;
        for i in 0..self.snd.pending.len() {
            let waits = self.snd.pending[i].waits.clone();
            if waits.into_iter().all(|(r, t)| self.snd_played(r, t)) { due = Some(i); break; }
        }
        let p = self.snd.pending.remove(due?);
        if !self.snd.chans.contains_key(&p.chan) { return None; } // disposed meanwhile: dropped
        let cmd = self.mm.new_ptr(&mut self.mem, &mut self.heap, 8, true).ok()?;
        let _ = self.mem.write_u16(cmd, snd::CALL_BACK_CMD).and_then(|_| self.mem.write_u16(cmd + 2, p.p1 as u16)).and_then(|_| self.mem.write_u32(cmd + 4, p.p2));
        self.snd.in_callback = true;
        self.log.push(format!("[sound] callBack routine {:#x} runs (channel {:#x}, param1 {}, param2 {:#x})", p.routine, p.chan, p.p1, p.p2));
        Some(DueCallback { chan: p.chan, routine: p.routine, cmd })
    }

    /// The routine returned (or could not be entered): free its command record and let the next due routine run.
    pub fn finish_snd_callback(&mut self, cmd: u32) {
        let _ = self.mm.dispose_ptr(&mut self.heap, cmd);
        self.snd.in_callback = false;
    }

    /// At a trap boundary: if a completion routine is due, enter it (and run the trap at `retry` afterwards). True = entered.
    pub(crate) fn enter_sound_callback(&mut self, cpu: &mut retro_m68k::Cpu, retry: u32) -> bool {
        let Some(d) = self.snd_take_due() else { return false };
        let mut regs = [0u32; 15];
        for (k, r) in regs.iter_mut().enumerate() { *r = if k < 8 { cpu.dreg(k as u32) } else { cpu.areg(k as u32 - 8) }; }
        let mut sp = cpu.areg(7);
        for v in [retry, d.chan, d.cmd, TRAMPOLINE] { sp = sp.wrapping_sub(4); let _ = self.mem.write_u32(sp, v); }
        cpu.set_areg(7, sp);
        self.call_stack.push(Frame::SndCallBack(VblDone { regs, sr: cpu.sr() }, d.cmd));
        cpu.set_pc(self.mem.strip(d.routine));
        true
    }

    /// The routine returned into the trampoline: the program's registers come back, the command record is freed.
    pub(crate) fn finish_sound_callback(&mut self, cpu: &mut retro_m68k::Cpu, done: VblDone, cmd: u32) {
        for (i, v) in done.regs.iter().enumerate() { if i < 8 { cpu.set_dreg(i as u32, *v) } else { cpu.set_areg(i as u32 - 8, *v) } }
        cpu.set_sr(done.sr);
        self.finish_snd_callback(cmd);
    }
}
