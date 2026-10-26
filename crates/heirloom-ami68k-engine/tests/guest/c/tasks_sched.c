/* exec multitasking, part 2: AddTask, round-robin among equal priorities, preemption by a higher-priority task that a
 * Signal wakes (and NOT by a lower one), Forbid/Disable, semaphore contention, RemTask and its tc_MemEntry, and
 * what happens to running tasks when the main program ends. Results in RES[] (written for this repo, MIT). */
#include "rt.h"
#define EXEC(off, d0, a0, a1) LVO(SYSBASE, off, d0, 0, 0, 0, a0, a1)
#define V(i) (*(volatile u32 *)&RES[i])
#define SIG(task, bits) EXEC(LVO_exec_Signal, bits, 0, task)
#define TASKSZ 4192

/* AddTask(a1 = task, a2 = initialPC, a3 = finalPC): LVO() has no a2/a3. */
COLD static u32 add_task_fin(u32 t, void (*pc)(void), void (*fin)(void)) {
  register u32 d0 asm("d0"); register u32 a1 asm("a1") = t; register u32 a2 asm("a2") = (u32)pc; register u32 a3 asm("a3") = (u32)fin;
  asm volatile("move.l %%a6,-(%%sp)\n\tmove.l %[b],%%a6\n\tjsr %c[o](%%a6)\n\tmove.l (%%sp)+,%%a6"
               : "=r"(d0), "+r"(a1) : "r"(a2), "r"(a3), [b] "r"(SYSBASE), [o] "i"(LVO_exec_AddTask) : "cc", "memory", "d1", "a0");
  return d0;
}
#define add_task(t, pc) add_task_fin(t, pc, 0)
/* A Task struct (92 bytes) and a 4000-byte stack in one cleared block. */
COLD static u32 newtask(const char *name, int pri) {
  u32 t = LVO(SYSBASE, LVO_exec_AllocMem, TASKSZ, 0x10000, 0, 0, 0, 0);
  *(u8 *)(t + 9) = (u8)pri; *(u32 *)(t + 10) = (u32)name; *(u32 *)(t + 58) = t + 96; *(u32 *)(t + 62) = t + 96 + 4000;
  return t;
}
#define FINDTASK(name) EXEC(LVO_exec_FindTask, 0, 0, (u32)(name))
#define ALLOCSIG() EXEC(LVO_exec_AllocSignal, -1, 0, 0)

void spin_a(void); void spin_b(void); void hi(void); void lo(void); void fb(void); void dis(void); void busy(void);
void quick(void); void final_fn(void); void sem_task(void); void ender(void); void forever(void);

void _start(void) {
  u32 me = FINDTASK(0);
  u32 mybit = ALLOCSIG();
  V(100) = me; V(101) = 1u << mybit;
  /* A. Round robin: two busy tasks of the main's priority both advance while main spins. */
  u32 ta = newtask("A", 0), tb = newtask("B", 0);
  add_task(ta, spin_a); add_task(tb, spin_b);
  V(0) = V(1) == 0 && V(2) == 0;                                       /* nothing ran yet (equal priority: no preemption) */
  V(3) = FINDTASK("A") == ta && FINDTASK("B") == tb;
  while (V(1) < 50 || V(2) < 50) ;                                      /* main's quantum ends; A, then B, then main run in turn */
  V(4) = V(1) > 1000 && V(2) > 1000;                                    /* both made real progress */
  V(120) = 1;                                                           /* stop */
  EXEC(LVO_exec_Wait, V(101), 0, 0);                                    /* A and B each signal as they end */
  V(5) = FINDTASK("A") == 0 && FINDTASK("B") == 0;                      /* both returned: gone */
  V(120) = 0;

  /* B. Preemption: H (pri 5) outranks main (pri 0). AddTask preempts at once; H sleeps; Signal(H) preempts again. */
  u32 th = newtask("H", 5);
  add_task(th, hi);
  V(10) = V(11) == th;                                                  /* H already ran: AddTask preempted main */
  V(12) = FINDTASK("H") == th;                                          /* ... and sleeps in TaskWait */
  { /* Walk ExecBase->TaskWait as a guest would (DOpus-style): H is on it, the running task is not. */
    u32 n = rd32((void *)(SYSBASE + 420)), found = 0, count = 0, ready = rd32((void *)(SYSBASE + 406));
    while (rd32((void *)n) != 0 && count < 16) { if (n == th) found = 1; count++; n = rd32((void *)n); }
    V(26) = found && count == 1;
    V(27) = rd32((void *)ready) == ready + 4 || rd32((void *)ready) == 0 ? 0 : 1;      /* TaskReady is empty: lh_Head = &lh_Tail */
    V(28) = rd32((void *)(SYSBASE + 276)) == me;                          /* ThisTask */
    V(29) = *(u8 *)(th + 15);                                            /* tc_State = TS_WAIT (4) */
  }
  V(45) = EXEC(LVO_exec_SetTaskPri, 3, 0, th);                           /* returns the old priority */
  V(46) = *(u8 *)(th + 9);
  EXEC(LVO_exec_SetTaskPri, 3, 0, 0x00DEAD00);                           /* not a task: refused */
  V(13) = 7;                                                            /* main's marker H copies when it wakes */
  SIG(th, 1u << V(15));
  V(14) = V(16);                                                        /* H finished before main's next statement */
  V(17) = V(18);                                                        /* the marker H saw: 7 */
  V(19) = FINDTASK("H") == 0;
  /* Negative control: a LOWER priority task does not preempt. */
  u32 tl = newtask("L", -5);
  add_task(tl, lo);
  V(20) = V(21);                                                        /* L has not run */
  EXEC(LVO_exec_Wait, 0, 0, 0);                                         /* (refused: nothing to wait for) */
  { u32 dos = OPENLIB("dos.library"); LVO5(dos, LVO_dos_Delay, 0, 1, 0, 0, 0, 0, 0); V(22) = V(21);   /* main sleeps: L runs now, up to its Wait */
    SIG(tl, 1u << V(21));
    V(23) = V(24);                                                      /* Signal(L) did not preempt: L has not finished */
    LVO5(dos, LVO_dos_Delay, 0, 1, 0, 0, 0, 0, 0);
    V(25) = V(24);                                                      /* it ran while main slept */
    CLOSELIB(dos); }

  /* C. Forbid / Disable: the same F task, switching suppressed until the matching Permit / Enable. */
  u32 tf = newtask("F", 5);
  add_task(tf, fb);                                                     /* preempts: F runs to its Wait and publishes its bit */
  EXEC(LVO_exec_Forbid, 0, 0, 0);
  SIG(tf, 1u << V(31));
  V(32) = V(30);                                                        /* 0: no switch while forbidden */
  EXEC(LVO_exec_Forbid, 0, 0, 0); EXEC(LVO_exec_Permit, 0, 0, 0);
  V(33) = V(30);                                                        /* 0: still nested once */
  EXEC(LVO_exec_Permit, 0, 0, 0);
  V(34) = V(30);                                                        /* 1: F ran at Permit */
  u32 td = newtask("D", 5);
  add_task(td, dis);
  EXEC(LVO_exec_Disable, 0, 0, 0);
  SIG(td, 1u << V(36));
  V(37) = V(35);                                                        /* 0 */
  EXEC(LVO_exec_Enable, 0, 0, 0);
  V(38) = V(35);                                                        /* 1 */
  /* A quantum does not switch a forbidden task either. */
  u32 tx = newtask("X", 0);
  add_task(tx, busy);
  EXEC(LVO_exec_Forbid, 0, 0, 0);
  for (volatile u32 i = 0; i < 200000; i++) ;
  V(40) = V(39);                                                        /* 0: X never ran */
  EXEC(LVO_exec_Permit, 0, 0, 0);
  for (volatile u32 i = 0; i < 200000; i++) ;
  V(41) = V(39) > 0;                                                    /* after Permit the quantum switches to X */
  EXEC(LVO_exec_RemTask, 0, 0, tx);                                     /* remove a ready task from outside */
  V(42) = FINDTASK("X") == 0;
  /* RemTask frees the memory in tc_MemEntry. */
  u32 tm = newtask("M", 0);
  u32 ml = LVO(SYSBASE, LVO_exec_AllocMem, 24, 0x10000, 0, 0, 0, 0);
  *(u16 *)(ml + 14) = 1; *(u32 *)(ml + 16) = tm; *(u32 *)(ml + 20) = TASKSZ;
  V(43) = LVO(SYSBASE, LVO_exec_AvailMem, 0, 0, 0, 0, 0, 0);
  add_task(tm, forever);
  EXEC(LVO_exec_AddTail, 0, tm + 74, ml);
  EXEC(LVO_exec_RemTask, 0, 0, tm);
  V(44) = LVO(SYSBASE, LVO_exec_AvailMem, 0, 0, 0, 0, 0, 0) - V(43);   /* the task block came back */

  /* A task that returns runs its finalPC (here: it notes it and removes itself); without one RemTask(0) is the default. */
  add_task_fin(newtask("Z", 0), quick, final_fn);
  EXEC(LVO_exec_Wait, V(101), 0, 0);
  V(47) = V(48) == 1 && V(49) == 1 && FINDTASK("Z") == 0;

  /* D. A semaphore another task holds blocks the caller. */
  u32 ss = LVO(SYSBASE, LVO_exec_AllocMem, 64, 0x10000, 0, 0, 0, 0);
  EXEC(LVO_exec_InitSemaphore, 0, ss, 0);
  V(121) = ss;
  EXEC(LVO_exec_ObtainSemaphore, 0, ss, 0);
  u32 ts = newtask("S", 0);
  add_task(ts, sem_task);
  for (volatile u32 i = 0; i < 100000; i++) ;                          /* S runs and blocks on the semaphore */
  V(50) = V(51);                                                        /* 0: S is not in its critical section */
  V(52) = rd32((void *)(ss + 40)) == me;                                /* ss_Owner */
  EXEC(LVO_exec_ReleaseSemaphore, 0, ss, 0);
  V(53) = V(51);                                                        /* 0: S was woken but main (equal priority) still runs */
  EXEC(LVO_exec_Wait, V(101), 0, 0);                                    /* S signals when it is done */
  V(54) = V(51);                                                        /* 1 */
  EXEC(LVO_exec_ReleaseSemaphore, 0, ss, 0);                            /* not held: refused */

  /* E. The main program ends with tasks still running: they are dropped. */
  add_task(newtask("E1", 0), forever); add_task(newtask("E2", 0), forever);
}

void spin_a(void) { while (!V(120)) V(1)++; SIG(V(100), V(101)); }
void spin_b(void) { while (!V(120)) V(2)++; SIG(V(100), V(101)); }
void hi(void) { u32 b = ALLOCSIG(); V(15) = b; V(11) = FINDTASK(0); EXEC(LVO_exec_Wait, 1u << b, 0, 0); V(18) = V(13); V(16) = 1; }
void lo(void) { u32 b = ALLOCSIG(); V(21) = b; EXEC(LVO_exec_Wait, 1u << b, 0, 0); V(24) = 1; }
void fb(void) { u32 b = ALLOCSIG(); V(31) = b; EXEC(LVO_exec_Wait, 1u << b, 0, 0); V(30) = 1; }
void dis(void) { u32 b = ALLOCSIG(); V(36) = b; EXEC(LVO_exec_Wait, 1u << b, 0, 0); V(35) = 1; }
void busy(void) { for (;;) V(39)++; }
void sem_task(void) {
  EXEC(LVO_exec_ObtainSemaphore, 0, V(121), 0);
  V(51) = 1;
  EXEC(LVO_exec_ReleaseSemaphore, 0, V(121), 0);
  SIG(V(100), V(101));
}
void forever(void) { for (;;) ; }
void quick(void) { V(48) = 1; }
void final_fn(void) { V(49) = V(48); SIG(V(100), V(101)); EXEC(LVO_exec_RemTask, 0, 0, 0); V(49) = 99; }
