/* AROS task.resource: storage slots and task hooks (LVOs -54 InitTaskHooks, -60 AddTaskHook, -66 RunTaskHooks, -72 Alloc,
 * -78 Free, -96 Set, -102 Get). Written for this repo (MIT). */
#include "rt.h"
#define TR(base, off, d0, d1, a0) LVO(base, off, d0, d1, 0, 0, a0, 0)
void _start(void) {
  u32 tr = LVO(SYSBASE, LVO_exec_OpenResource, 0, 0, 0, 0, 0, (u32)"task.resource");
  u32 tr2 = LVO(SYSBASE, LVO_exec_OpenResource, 0, 0, 0, 0, 0, (u32)"TASK.resource");
  RES[0] = tr != 0 && tr == tr2;                               /* one resource, case-insensitive name */
  u32 s1 = TR(tr, -72, 0, 0, 0), s2 = TR(tr, -72, 0, 0, 0);
  RES[1] = s1; RES[2] = s2;                                    /* 1, 2 */
  RES[3] = TR(tr, -102, s1, 0, 0);                             /* never set: 0 */
  RES[4] = TR(tr, -96, s1, 0xCAFE, 0);                         /* TRUE */
  TR(tr, -96, s2, 0xBEEF, 0);
  RES[5] = TR(tr, -102, s1, 0, 0); RES[6] = TR(tr, -102, s2, 0, 0); /* 0xCAFE, 0xBEEF: slots are independent */
  TR(tr, -78, s1, 0, 0);                                       /* free slot 1 */
  RES[7] = TR(tr, -72, 0, 0, 0);                               /* reused: 1 */
  RES[8] = TR(tr, -102, 1, 0, 0);                              /* its old value is gone: 0 */
  RES[9] = TR(tr, -54, 1, 2, 0x1234);                          /* InitTaskHooks: TRUE */
  RES[10] = TR(tr, -66, 1, 0, 0x1234);                         /* RunTaskHooks with no hooks: TRUE */
  /* negative controls: refused by name, and the table is untouched */
  TR(tr, -96, 99, 5, 0);                                       /* SetTaskStorageSlot on a slot nobody allocated */
  TR(tr, -78, 99, 0, 0);                                       /* FreeTaskStorageSlot of the same */
  RES[11] = TR(tr, -102, 99, 0, 0);                            /* 0 */
  TR(tr, -60, 1, 0, 0x5678);                                   /* AddTaskHook type 1 ... */
  RES[12] = TR(tr, -66, 1, 0, 0x1234);                         /* ... so RunTaskHooks(1) is now refused: d0 = 0 */
  RES[13] = TR(tr, -108, 1, 0, 0);                             /* GetParentTaskStorageSlot of the main task: 0, not a refusal */
  RES[14] = 1;
}
