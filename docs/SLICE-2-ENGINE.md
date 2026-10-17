# Slice 2, engine half: a resumable guest with a real IDCMP loop (2026-10-17)

Design ruling: Astra (openai/gpt-6-astra) consult 2026-10-17, option (a). The guest is a RESUMABLE state machine driven
by the host's event loop. No second thread, no second loop. This file is the spec for the `coralos-compat` half; the
CoralOS half (a thin `RetroCompat.app` on the CoralApp SDK) consumes it later and is out of scope here.

## 1. retro-m68k: start + resume (one CPU per process, state survives between calls)

- `pub fn start<H: Host>(host: &mut H, entry: u32, sp: u32)`: what `run` does today up to and including the reset
  (init, CPU_68020, pulse_reset, the AbsExecBase save/restore), executing NOTHING.
- `pub fn resume<H: Host>(host: &mut H, max_insns: u64) -> RunStats`: execute from the CPU's current state until
  `Cpu::halt` / `Cpu::block` or `max_insns`, without any reset. Musashi's context is process-global, so it survives.
- `run` = `start` then `resume` (kept; same behaviour, same tests).
- `Cpu::block(&mut self)`: stop after the current instruction like `halt`, but `RunStats` reports `blocked: true`
  (new field) and `halted: false`. A blocked guest is resumed by the next `resume` call, which continues at the
  instruction after the trapping ILLEGAL (the library slot's RTS), so the trapped call returns then, with whatever
  D0 the host set before resuming. Add `pub fn set_d0` use on resume (already exists) — the host sets D0 first.
- The lock/ACTIVE discipline stays: ACTIVE is set for the duration of each `resume`, cleared on return.

## 2. amiga68k-engine: Machine states, Exec ports, IDCMP

Public API (machine.rs, split files by concern, every file < 1000 lines, no_std + alloc):

```rust
pub enum Step { Exited, Blocked, BudgetSpent, Fault(String) }
impl<U: GuestUi> Amiga<U> {
    pub fn start_hunk(&mut self, file: &[u8]) -> Result<Loaded, String>; // load + retro_m68k::start
    pub fn step(&mut self, budget: u64) -> Step;     // resume until exit / block / budget
    pub fn deliver(&mut self, ev: GuestUiEvent) -> Result<(), Refusal>; // host event -> IDCMP message
    pub fn is_blocked(&self) -> bool;
}
```
`run_hunk` stays as a driver over `start_hunk` + `step` (one path, not two).

### Guest structures (offsets from the AmigaOS NDK 3.x includes; each constant names its struct.field)
- `struct Window`: allocated in guest RAM by OpenWindowTagList (today the handle is only a token; it becomes the
  guest address of a real, zero-filled `struct Window`, size 136). Fields written: `UserPort` (offset 86, ptr to a
  MsgPort), `IDCMPFlags` (82), `Title` (32). Keep the existing handle->UiObjectRef map keyed by that address.
- `struct MsgPort` (34 bytes): `mp_SigBit` (15), `mp_SigTask` (16), `mp_MsgList` (20, a struct List, 14 bytes).
- `struct IntuiMessage` (52 bytes): `ExecMessage` (a struct Message, 20 bytes: ln_Succ 0, ln_Pred 4, ln_Type 8 = NT_MESSAGE 5,
  mn_ReplyPort 14, mn_Length 18), `Class` (20, u32), `Code` (24, u16), `Qualifier` (26), `IAddress` (28),
  `MouseX`/`MouseY` (32/34), `IDCMPWindow` (44).
- IDCMP classes used: `IDCMP_CLOSEWINDOW = 0x200`, `IDCMP_MENUPICK = 0x100`. MENUPICK `Code` = the packed menu number
  (`menu | item<<5 | sub<<11`, MENUNULL = 0xFFFF), which `menu.rs` already computes for tokens.

### Exec LVOs to bridge (exec.library, negative offsets from ExecBase)
`Wait` (-318, D0 = signal set), `GetMsg` (-372, A0 = port), `ReplyMsg` (-378, A1 = msg), `WaitPort` (-384, A0 = port).
Signals: one task, a u32 `SigRecvd` the engine owns (host side, not guest RAM). Each window's UserPort gets a signal bit
(allocate from bit 16 up; 0..15 are system). 
- `Wait(set)`: if `SigRecvd & set != 0` -> D0 = that, clear those bits, return now. Else record the wait set and
  `cpu.block()`; on the matching `deliver`, D0 = matched bits (cleared) and the next `step` resumes the guest.
- `WaitPort(port)`: if the port's list is non-empty return its head in D0, else block like `Wait(1<<port.sigbit)` and
  return the head on resume.
- `GetMsg(port)`: unlink and return the head (D0), or 0 if empty. Does NOT free it.
- `ReplyMsg(msg)`: validate `msg` is an outstanding IntuiMessage this engine allocated and delivered (host-side table:
  address -> state Queued|Delivered|Replied). Replied messages return to a free list. A reply of an unknown, forged,
  or already-replied address is a named Refusal (logged, guest continues, D0 untouched).

### deliver(ev)
- `CloseRequested{win}` -> an IntuiMessage Class=CLOSEWINDOW, IDCMPWindow = that window, IAddress = window.
- `MenuCommand{token}` -> Class=MENUPICK, Code = the packed number recovered from the token.
- Only if the window's IDCMPFlags include that class; otherwise a named Refusal and nothing is queued.
- Allocate the IntuiMessage from a free list in guest RAM (growable region, NOMEM is a named Refusal), link it at the
  tail of the window's UserPort list, set the port's signal bit in SigRecvd; if the guest is blocked on a set that
  includes it, mark it runnable (the next `step` resumes it).
- A message for a window that is already closed (CloseWindow retires the window and its port) is a named Refusal.

### CloseWindow
Retire the window: its port must have no Delivered-but-unreplied messages (otherwise named Refusal, as on AmigaOS
where that is a guest bug); queued unread messages are freed; the GuestUi close_window call is made as today.

## 3. Fixture: `native_quit` (fixture.rs, hand-assembled with retro-hunk's writer, as the slice-1 fixture is)
A guest program that: OpenLibrary("intuition.library"); OpenWindowTagList(title "Native Quit", IDCMP
CLOSEWINDOW|MENUPICK); SetMenuStrip(Project: Open, Quit); then LOOP: WaitPort(win->UserPort); GetMsg; read Class and
Code; ReplyMsg; increment a SENTINEL long at a fixed guest address; if Class == CLOSEWINDOW or (MENUPICK and Code == the
Quit item) -> ClearMenuStrip; CloseWindow; CloseLibrary; RTS to the exit slot; else loop.

## 4. Tests first (host, `cargo test --workspace`), each with its negative control
- `tests/wait_resume.rs`: start native_quit; step -> Blocked; SENTINEL unchanged after any number of further steps while
  nothing is delivered; deliver(MenuCommand Open) -> step -> Blocked again, SENTINEL == 1; deliver(CloseRequested) ->
  step -> Exited, SENTINEL == 2, GuestUi recorded close_window. NEGATIVE CONTROLS (as `#[test]`s using a test-only
  fault switch in the engine, `cfg(test)` or a feature `fault-injection`): (a) clear the signal after enqueue -> the
  guest never wakes, the test asserts Blocked forever and the control test asserts THAT fails the happy-path check;
  (b) resume past a blocked call without a delivery -> SENTINEL moves, which the check catches.
- `tests/idcmp_lifecycle.rs`: double ReplyMsg -> named Refusal; ReplyMsg of a forged address -> named Refusal;
  deliver after CloseWindow -> named Refusal; deliver a class not in IDCMPFlags -> Refusal; CloseWindow with a
  delivered-unreplied message -> Refusal; 10 000 deliver/reply cycles leave the guest message free list at its
  high-water size (no leak).
- Existing 28 tests keep passing; slice-1 fixture behaviour unchanged.

## Out of scope (named, not promised)
Multiple guest tasks, Forbid/Permit, timer.device, any IDCMP class but the two above, controls/gadgets, real
binaries (amimon is the next step after the CoralOS frontend lands).
