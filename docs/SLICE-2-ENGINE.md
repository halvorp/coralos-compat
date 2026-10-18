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
// Guest state (host side): New -> Runnable <-> Waiting(Wait{set} | WaitPort{port}) -> Exited | Faulted.
// step(): a Waiting guest whose predicate is still unsatisfied executes NO instruction and returns Blocked.
// deliver(): queues and updates readiness only; it never executes guest code and never overwrites a pending D0.
impl<U: GuestUi> Amiga<U> {
    pub fn start_hunk(&mut self, file: &[u8]) -> Result<Loaded, String>; // load + retro_m68k::start
    pub fn step(&mut self, budget: u64) -> Step;     // resume until exit / block / budget
    pub fn deliver(&mut self, ev: GuestUiEvent) -> Result<(), Refusal>; // host event -> IDCMP message
    pub fn is_blocked(&self) -> bool;
}
```
`run_hunk` stays as a driver over `start_hunk` + `step` (one path, not two); it returns when the guest blocks, it never
spins while blocked. `BudgetSpent` means "schedule another host-loop turn"; `Blocked` means "wait for a deliver".
`Amiga` owns its `retro_m68k::CpuContext`. Sharing with mac68k/macppc is the CPU-neutral boundary only (CpuContext,
bounded step, stop reason, GuestUi); Exec signals, ports, menu packing and IntuiMessage bytes stay in this crate.

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
Signals are PER TASK (council review 2026-10-17): host-side `tc_SigRecvd` keyed by the guest Task address (slice 2 had one task; the tasks lane added more: see "Tasks" below). `deliver` reads the port's `mp_SigTask` and sets that task's bit.
Each window's UserPort gets a signal bit
(allocate from bit 16 up; 0..15 are system). 
- `Wait(set)`: if `SigRecvd & set != 0` -> D0 = that, clear ONLY those bits (unrelated bits are kept), return now. Else record the wait set and
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
- Only if the window's IDCMPFlags include that class; otherwise `IgnoredUnsubscribed` (not an error: Intuition simply
  does not send it) and nothing is queued.
- TRANSACTIONAL: reserve the message slot and bookkeeping before linking or signalling; a failed delivery (NOMEM, a
  named Refusal) leaves the queue, signal bits, pending wait, allocator and GuestUi unchanged.
- Allocate the IntuiMessage from a free list in guest RAM (growable region, NOMEM is a named Refusal), link it at the
  tail of the window's UserPort list, set the port's signal bit in SigRecvd; if the guest is blocked on a set that
  includes it, mark it runnable (the next `step` resumes it).
- A message for a window that is already closed (CloseWindow retires the window and its port) is a named Refusal.

### Limits stated (not promised)
Address reuse defeats stale-message detection by address alone: messages carry a host-side incarnation; host events
carry the window's UiObjectRef generation, so a delayed event for a retired window is refused even if the address is
reused. Ports are Intuition-private UserPorts only; a guest-supplied or shared port (ModifyIDCMP tricks) is refused
by name.

### CloseWindow
Retire the window: its port must have no Delivered-but-unreplied messages (otherwise named Refusal, as on AmigaOS
where that is a guest bug); queued unread messages are freed; the GuestUi close_window call is made as today.

## 3. Fixture: `native_quit` (fixture.rs, hand-assembled with retro-hunk's writer, as the slice-1 fixture is)
A guest program that: OpenLibrary("intuition.library"); OpenWindowTagList(title "Native Quit", IDCMP
CLOSEWINDOW|MENUPICK); SetMenuStrip(Project: Open, Quit); then LOOP: WaitPort(win->UserPort); GetMsg; read Class and
Code; ReplyMsg; increment a SENTINEL long at a fixed guest address; check every OpenLibrary/OpenWindow result; drain with GetMsg until it returns 0
(conventional wait-then-drain); copy Class and Code BEFORE ReplyMsg and never read a replied message; if Class == CLOSEWINDOW or (MENUPICK and Code == the
Quit item) -> ClearMenuStrip; CloseWindow; CloseLibrary; RTS to the exit slot; else loop.

## 3b. ABI layout ground truth
Offsets are computed by a compiler with the 68k ABI (4-byte pointers, nothing aligned past 2 bytes: `gcc -m32` +
`#pragma pack(2)` over the AROS field lists), not hand-counted and not taken from a model: Window.Title 32,
IDCMPFlags 82, UserPort 86, sizeof 136; MsgPort 34 (mp_SigBit 15, mp_SigTask 16, mp_MsgList 20); Message 20
(mn_ReplyPort 14, mn_Length 18); IntuiMessage 52 (Class 20, Code 24, IAddress 28, IDCMPWindow 44). The council review
claimed IDCMPFlags 86 / UserPort 90 / sizeof 140; the compiler refutes it. `tests/abi_layout.rs` pins these against
the engine's constants. Next ground truth: FS-UAE + an AROS ROM, dumping the real structures from a running guest.

## 4. Tests first (host, `cargo test --workspace`), each with its negative control
- `tests/wait_resume.rs`: start native_quit; step -> Blocked; SENTINEL unchanged after any number of further steps while
  nothing is delivered; deliver(MenuCommand Open) -> step -> Blocked again, SENTINEL == 1; deliver(CloseRequested) ->
  step -> Exited, SENTINEL == 2, GuestUi recorded close_window. NEGATIVE CONTROLS (as `#[test]`s using a test-only
  fault switch in the engine, `cfg(test)` or a feature `fault-injection`): (a) clear the signal after enqueue -> the
  guest never wakes, the test asserts Blocked forever and the control test asserts THAT fails the happy-path check;
  (b) resume past a blocked call without a delivery -> SENTINEL moves, which the check catches.
- More `wait_resume.rs` controls (council review): a delivery whose bit is NOT in the Wait set leaves the guest Blocked;
  two delivers before one step -> WaitPort returns, GetMsg twice succeeds, the third returns 0; no guest instruction
  executes while a wait is unsatisfied (instruction counter unchanged across steps).
- `tests/cpu_context.rs`: two Amiga machines alternated step by step with different programs/registers; neither
  disturbs the other.
- `tests/idcmp_lifecycle.rs`: ReplyMsg of a Queued (not yet GetMsg'd) message -> Refusal; exhaust the message pool ->
  NOMEM Refusal with nothing changed, then one ReplyMsg makes the next deliver succeed; double ReplyMsg -> named Refusal; ReplyMsg of a forged address -> named Refusal;
  deliver after CloseWindow -> named Refusal; deliver a class not in IDCMPFlags -> Refusal; CloseWindow with a
  delivered-unreplied message -> Refusal; 10 000 deliver/reply cycles leave the guest message free list at its
  high-water size (no leak).
- Existing 28 tests keep passing; slice-1 fixture behaviour unchanged.

## Out of scope (named, not promised)
Multiple guest tasks, Forbid/Permit, timer.device, any IDCMP class but the two above, controls/gadgets, real
binaries (amimon is the next step after the CoralOS frontend lands).

## Tasks (exec multitasking in one engine)

`sched.rs` is the scheduler behind the one host entry `Amiga::step(budget)`. Every guest task (the program's Process,
`AddTask` tasks, `CreateNewProc`/`CreateProc` processes) has a real `struct Task`/`Process` in guest RAM, its own stack and
`CpuContext`, and host-side state (`sched::Task`): wait condition, Forbid/Disable nesting, local variables and host-started
guest calls (swapped in while it runs), and what a dos process owns. `ExecBase.ThisTask`, `TDNestCnt`/`IDNestCnt` and the
`TaskReady`/`TaskWait` lists (rebuilt from the host table at every switch) are real. The rules:

- highest `ln_Pri` first; equal priorities round-robin on a quantum of 20 000 instructions; a Signal/PutMsg/ReleaseSemaphore
  or timer wake that makes a higher-priority task ready ends the running task's slice at the end of that library call;
- Forbid()/Disable() stop both switches (a task that Waits inside one is switched away anyway). Disable/Enable act as
  Forbid/Permit: interrupts are not modelled;
- a task parks in Wait / WaitPort / WaitIO / DoIO / Delay / ObtainSemaphore (`exec_wait::Waiting`); with ONE task DoIO/Delay
  keep their old shortcuts (fast-forward / return at once), with more they sleep on the virtual clock the host advances;
- `step` returns `Blocked` when every task is parked, `BudgetSpent`, or `Exited` when the MAIN program ends. The other
  tasks are then dropped and named in the log and `sched.dropped` (AmigaOS would keep running them);
- signals and ports follow `mp_SigTask`, never a global; Signal/PutMsg to an address that is not a live task is a named
  refusal. `Wait` for a signal the task never allocated is refused (it could never wake);
- created processes get the argument line in A0/D0, return into a finalizer that runs `NP_ExitCode`, and give back their
  stack, streams (`NP_Close*`), seglist (`NP_FreeSeglist`), current/home directory locks and variables when they end or are
  `RemTask`ed. Not modelled, refused by name: `NP_Synchronous`, `NP_NotifyOnDeath`, `NP_Affinity`.

Sources of the facts: AROS rom/exec (addtask/newaddtask/remtask/wait/signal/settaskpri) and rom/dos (createnewproc,
createproc, exit) as interface facts; the DOpus 5 IPC layer (Library/ipc.c) for the startup handshake the real binary does.
