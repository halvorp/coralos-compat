/* Shared SignalSemaphore holds: two tasks hold it together, an exclusive obtain waits for both, and a shared obtain waits for an
 * exclusive holder. Written for this repo (MIT). */
#include "rt.h"
#define EXEC(off, d0, a0, a1) LVO(SYSBASE, off, d0, 0, 0, 0, a0, a1)
#define V(i) (*(volatile u32 *)&RES[i])
#define SIG(task, bits) EXEC(LVO_exec_Signal, bits, 0, task)
#define TASKSZ 4192
COLD static u32 add_task(u32 t, void (*pc)(void)) {
  register u32 d0 asm("d0"); register u32 a1 asm("a1") = t; register u32 a2 asm("a2") = (u32)pc; register u32 a3 asm("a3") = 0;
  asm volatile("move.l %%a6,-(%%sp)\n\tmove.l %[b],%%a6\n\tjsr %c[o](%%a6)\n\tmove.l (%%sp)+,%%a6"
               : "=r"(d0), "+r"(a1) : "r"(a2), "r"(a3), [b] "r"(SYSBASE), [o] "i"(LVO_exec_AddTask) : "cc", "memory", "d1", "a0");
  return d0;
}
COLD static u32 newtask(const char *name, int pri) {
  u32 t = LVO(SYSBASE, LVO_exec_AllocMem, TASKSZ, 0x10000, 0, 0, 0, 0);
  *(u8 *)(t + 9) = (u8)pri; *(u32 *)(t + 10) = (u32)name; *(u32 *)(t + 58) = t + 96; *(u32 *)(t + 62) = t + 96 + 4000;
  return t;
}
void reader(void); void writer(void);
void _start(void) {
  u32 me = EXEC(LVO_exec_FindTask, 0, 0, 0);
  u32 bit = EXEC(LVO_exec_AllocSignal, -1, 0, 0);
  V(100) = me; V(101) = 1u << bit;
  u32 ss = LVO(SYSBASE, LVO_exec_AllocMem, 64, 0x10000, 0, 0, 0, 0);
  V(102) = ss;
  EXEC(LVO_exec_InitSemaphore, 0, ss, 0);
  EXEC(LVO_exec_ObtainSemaphoreShared, 0, ss, 0);               /* main holds it shared */
  V(0) = rd32((void *)(ss + 40));                               /* ss_Owner: NULL while only shared holders exist */
  add_task(newtask("R", 0), reader);
  EXEC(LVO_exec_Wait, V(101), 0, 0);                            /* R got it SHARED at once, while main still holds it, and signalled */
  V(1) = V(110);                                                /* 1: the reader entered */
  add_task(newtask("W", 0), writer);
  for (volatile u32 i = 0; i < 100000; i++) ;                   /* W runs and blocks (main and R hold it shared) */
  V(2) = V(111);                                                /* 0: the writer is not in */
  EXEC(LVO_exec_ReleaseSemaphore, 0, ss, 0);                    /* main's hold ends; R still holds it */
  for (volatile u32 i = 0; i < 100000; i++) ;
  V(3) = V(111);                                                /* 0: still not in (R holds) */
  SIG(V(112), 1u << 30);                                        /* tell R to release */
  EXEC(LVO_exec_Wait, V(101), 0, 0);                            /* the writer signals when it is in */
  V(4) = V(111);                                                /* 1 */
  /* Negative control: a release by a task that does not hold it is refused. */
  EXEC(LVO_exec_ReleaseSemaphore, 0, ss, 0);
}
void reader(void) {
  V(112) = EXEC(LVO_exec_FindTask, 0, 0, 0);
  EXEC(LVO_exec_ObtainSemaphoreShared, 0, V(102), 0);
  V(110) = 1;
  SIG(V(100), V(101));
  EXEC(LVO_exec_AllocSignal, 30, 0, 0);
  EXEC(LVO_exec_Wait, 1u << 30, 0, 0);
  EXEC(LVO_exec_ReleaseSemaphore, 0, V(102), 0);
}
void writer(void) {
  EXEC(LVO_exec_ObtainSemaphore, 0, V(102), 0);
  V(111) = 1;
  SIG(V(100), V(101));
  EXEC(LVO_exec_ReleaseSemaphore, 0, V(102), 0);
}
