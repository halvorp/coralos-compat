/* AROS task.resource GetParentTaskStorageSlot (-108): the parent is the task that was running when AddTask added the child
 * (rom/exec/newaddtask.c:189 GET_THIS_TASK -> InitETask(task, parent), rom/task/GetParentTaskStorageSlot.c). Written for this repo (MIT).
 * RES[0] main's parent value (the launcher owns none: 0), RES[1] the child's view of the parent's slot (0xCAFE), RES[2] the child's own
 * value of that slot (0: not inherited), RES[3] the child ran, RES[4] a grandchild's parent value is the CHILD's (0xF00D). */
#include "rt.h"
#define TR(base, off, d0) LVO(base, off, d0, 0, 0, 0, 0, 0)
#define TASKSZ 4192
COLD static u32 add_task(u32 t, void (*pc)(void)) {
  register u32 d0 asm("d0"); register u32 a1 asm("a1") = t; register u32 a2 asm("a2") = (u32)pc; register u32 a3 asm("a3") = 0;
  asm volatile("move.l %%a6,-(%%sp)\n\tmove.l %[b],%%a6\n\tjsr %c[o](%%a6)\n\tmove.l (%%sp)+,%%a6"
               : "=r"(d0), "+r"(a1) : "r"(a2), "r"(a3), [b] "r"(SYSBASE), [o] "i"(LVO_exec_AddTask) : "cc", "memory", "d1", "a0");
  return d0;
}
COLD static u32 newtask(const char *name) {
  u32 t = LVO(SYSBASE, LVO_exec_AllocMem, TASKSZ, 0x10000, 0, 0, 0, 0);
  *(u8 *)(t + 9) = 0; *(u32 *)(t + 10) = (u32)name; *(u32 *)(t + 58) = t + 96; *(u32 *)(t + 62) = t + 96 + 4000;
  return t;
}
#define TRES() LVO(SYSBASE, LVO_exec_OpenResource, 0, 0, 0, 0, 0, (u32)"task.resource")
void grandchild(void); void child(void);
void _start(void) {
  u32 tr = TRES();
  u32 slot = TR(tr, -72, 0);                   /* AllocTaskStorageSlot */
  RES[10] = slot;
  LVO(tr, -96, slot, 0xCAFE, 0, 0, 0, 0);      /* SetTaskStorageSlot(slot, 0xCAFE) */
  RES[0] = TR(tr, -108, slot);
  add_task(newtask("kid"), child);
  while (RES[3] == 0 || RES[4] == 0) ;         /* main spins; the kid and then the grandchild get their turns */
}

void child(void) {
  u32 tr = TRES(), slot = RES[10];
  RES[1] = TR(tr, -108, slot);                 /* GetParentTaskStorageSlot */
  RES[2] = TR(tr, -102, slot);                 /* GetTaskStorageSlot: this task's own value */
  LVO(tr, -96, slot, 0xF00D, 0, 0, 0, 0);      /* SetTaskStorageSlot(slot, 0xF00D) */
  add_task(newtask("grandchild"), grandchild);
  while (RES[4] == 0) ;                        /* stay alive: a parent that has ended orphans its children */
  RES[3] = 1;
}
void grandchild(void) { u32 tr = TRES(), slot = RES[10]; RES[4] = TR(tr, -108, slot); }
