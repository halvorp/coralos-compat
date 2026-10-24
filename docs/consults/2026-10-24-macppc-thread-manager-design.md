# macppc Thread Manager (cooperative threads) and the scheduler Multiprocessing Services needs: design note

2026-10-24. Not implemented in this commit; it is the plan for the next one, written first because the Thread Manager's `YieldToThread` and MP Services'
tasks share ONE mechanism (switching the interpreter between saved register files) and the wrong first cut would have to be thrown away.

## What the engine has to build on
* `retro_ppc::Cpu` is a plain value (`#[derive(Clone)]`, `CpuContext` alias): gpr/fpr/cr/lr/ctr/xer/fpscr/pc/tb + the bound-native set. One interpreter
  can run any number of saved contexts by swapping which `Cpu` the run loop drives (`crates/retro-ppc/src/cpu.rs`).
* `run_loop` / `step` drive ONE `Cpu` held in `Runtime::cpu` (`crates/macppc-engine/src/run.rs`); natives get `&mut Cpu` and may redirect it
  (`set_pc`, as the callback machine does: `callback.rs`, `callback_c.rs`); `GlueOutcome::Idle` parks the CPU on a call (events, ModalDialog).
* Frames of host work in flight (`Runtime::callbacks`) are saved per interrupted call; they belong to ONE thread, so they must move with it.

## Thread Manager (Inside Macintosh: Thread Manager; Threads.h)
Calls: `NewThread(threadStyle, threadEntry, threadParam, stackSize, options, *threadResult, *threadMade)`, `GetCurrentThread`, `YieldToThread(id)`,
`YieldToAnyThread()`, `DisposeThread(id, exitValue, recycle)`, `GetThreadState`, `SetThreadState`, `SetThreadStateEndCritical`, `ThreadBeginCritical`,
`ThreadEndCritical`, `GetFreeThreadCount`, `GetSpecificFreeThreadCount`, `SetThreadScheduler`, `SetThreadSwitcher`, `SetThreadTerminator`, `GetDefaultThreadStackSize`.
* **Model.** Thread table `BTreeMap<ThreadID, Thread>`; `Thread { cpu: Cpu (saved), stack: guest block, state: Running|Ready|Stopped|Waiting,
  entry/param, result, callbacks: Vec<Frame>, critical: u32 }`. ThreadID 1 is the application's main thread (kApplicationThreadID); new ids are
  never reused until disposed (IDs are 32-bit, no table cap).
* **Switch.** A thread call is a native with `&mut Cpu`: the current context is `clone()`d into the current thread's slot, the target's context is
  assigned into `*cpu` (`natives` re-synced), `rt.callbacks` swapped with the target's list. Nothing else changes; the native "returns" into the other thread.
* **NewThread** allocates the stack from the guest heap (`stackSize`, 0 = the default `GetDefaultThreadStackSize` answers), builds a context
  that enters `threadEntry(threadParam)` (a TVector: r2/r12 as `enter_tvector`) with LR = a `$ThreadExit` pseudo-glue: a thread that returns calls
  `DisposeThread(self, result)`; the scheduler then picks the next ready thread. A thread entry running off with nothing ready is a named fault.
* **Scheduler.** Cooperative: only `YieldTo*`, a blocking call, or `DisposeThread` switch. `YieldToAnyThread` round-robins over Ready threads (the
  `SetThreadScheduler` hook, a program routine, is a later Continuation: it picks the target by callback). Preemptive-style threads
  (`kPreemptiveThread`) are created as cooperative with a NAMED log note, because the classic Mac OS also ran them cooperatively unless MP-aware.
* **Blocking inside a thread.** An event call that would park the CPU (`GlueOutcome::Idle`) with other threads Ready yields to them instead of
  idling the whole machine; the run loop only reports `Step::Blocked` when every thread is blocked.
* **Gestalt.** `'thds'` bit 0 (gestaltThreadMgrPresent) moves from `gestalt_ppc::UNANSWERED` to `answer()` in the commit that lands this; bit 2
  (preemptive threads present) stays 0.
* **Tests (planned).** Two threads ping-pong through `YieldToThread` writing an ordered log (PowerPC code on both sides); a returning thread is
  disposed and its result readable; `GetCurrentThread` agrees with the switch; yielding with nothing else ready returns at once; callbacks in
  progress (qsort's comparison) stay with their thread across a yield inside the comparison. Negative controls: a disposed id refuses
  (`threadNotFoundErr` -618), `YieldToThread` on a stopped thread is `threadProtocolErr` (-619), a stack size that cannot be allocated is `memFullErr`.

## What Multiprocessing Services adds (and why it must not be built as threads)
MP Services (`MPLibrary`, Multiprocessing.h) offers tasks that run CONCURRENTLY on other processors, queues, semaphores, event groups, critical regions,
timers, and a task-local address space for notification. Facts that decide the design:
* A task is preemptive; the application may share memory with it freely and expects real parallelism only for speed, not for correctness; but
  it WILL spin on shared flags that another task sets, so a cooperative scheduler deadlocks such programs. The engine needs time slicing.
* One interpreter, one `Cpu` driven at a time: slicing is `resume(host, cpu, budget)` for a bounded instruction count, then a switch. The scheduler
  therefore lives INSIDE `run_loop` (not in a native): when a slice ends, the loop saves the context and picks the next runnable task.
* Everything above shares the context-swap primitive. So: build a `Scheduler { tasks, current, slice }` in `macppc-engine/src/sched.rs` that both
  the Thread Manager (cooperative: slice = infinity, switches only on yield) and MP Services (slice = N instructions, plus blocking on queues/
  semaphores) use. The Thread Manager lands first with `slice = None`; MP Services adds the slice and the wait queues.
* Gestalt `'mpsv'` stays unanswered until `MPProcessors()` is honest: it reports 1 until tasks are
  truly interleaved, and a program is better served by a Gestalt that says "one processor" than by none.

## Order
1. `sched.rs` with the context table and `swap(cpu, to)`; unit tests with synthetic contexts (no Toolbox).
2. Thread Manager calls over it, `$ThreadExit`, the Idle-yield rule, `'thds'`.
3. MP Services: tasks, slices, queues, semaphores, critical regions, `MPProcessors`, `'mpsv'`.
