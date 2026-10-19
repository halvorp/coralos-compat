/* A synchronous requester parks only the calling TASK: another task keeps counting while the main program waits for the answer
 * (written for this repo, MIT). Results in RES[]. */
#include "rt.h"
#define V(i) (*(volatile u32 *)&RES[i])
#define TASKSZ 4192
struct Easy { u32 size, flags; const char *title, *text, *gadgets; };
COLD static u32 newtask(const char *name, int pri) {
  u32 t = LVO(SYSBASE, LVO_exec_AllocMem, TASKSZ, 0x10000, 0, 0, 0, 0);
  *(u8 *)(t + 9) = (u8)pri; *(u32 *)(t + 10) = (u32)name; *(u32 *)(t + 58) = t + 96; *(u32 *)(t + 62) = t + 96 + 4000;
  return t;
}
COLD static u32 add_task(u32 t, void (*pc)(void)) {
  register u32 d0 asm("d0"); register u32 a1 asm("a1") = t; register u32 a2 asm("a2") = (u32)pc; register u32 a3 asm("a3") = 0;
  asm volatile("move.l %%a6,-(%%sp)\n\tmove.l %[b],%%a6\n\tjsr %c[o](%%a6)\n\tmove.l (%%sp)+,%%a6"
               : "=r"(d0), "+r"(a1) : "r"(a2), "r"(a3), [b] "r"(SYSBASE), [o] "i"(LVO_exec_AddTask) : "cc", "memory", "d1", "a0");
  return d0;
}
void counter(void);
void _start(void) {
  u32 ib = OPENLIB("intuition.library");
  add_task(newtask("counter", 0), counter);
  struct Easy es = { 20, 0, "Wait", "Still there?", "Yes|No" };
  register u32 d0 asm("d0") = 0; register u32 a0 asm("a0") = 0; register u32 a1 asm("a1") = (u32)&es; register u32 a2 asm("a2") = 0; register u32 a3 asm("a3") = 0;
  asm volatile("move.l %%a6,-(%%sp)\n\tmove.l %[b],%%a6\n\tjsr %c[o](%%a6)\n\tmove.l (%%sp)+,%%a6" : "+r"(d0), "+r"(a0), "+r"(a1) : "r"(a2), "r"(a3), [b] "r"(ib), [o] "i"(LVO_intuition_EasyRequestArgs) : "cc", "memory", "d1");
  V(62) = d0 + 100;               /* set only when the request returned (answer 1 -> 101) */
  V(61) = 1;                      /* stop the counter */
  CLOSELIB(ib);
}
void counter(void) { while (!V(61)) V(60)++; }
