/* SetFunction on an engine library (dos.library/IoErr), chaining to the old function through the returned address,
 * restoring it, plus CacheClearU/CacheControl. Written for this repo (MIT). */
#include "rt.h"
#define V(i) (*(volatile u32 *)&RES[i])
#define DOS(off, d1) LVO5(dos, off, 0, d1, 0, 0, 0, 0, 0)
u32 replaced(void); u32 chained(void);
/* SetFunction(a1 = library, a0 = offset, d0 = function) */
#define SETFUNC(lib, off, fn) LVO(SYSBASE, LVO_exec_SetFunction, (u32)(fn), 0, 0, 0, (u32)(off), (u32)(lib))

void _start(void) {
  u32 dos = OPENLIB("dos.library");
  DOS(LVO_dos_SetIoErr, 77);
  V(0) = DOS(LVO_dos_IoErr, 0);                                    /* 77 */
  u32 old = SETFUNC(dos, LVO_dos_IoErr, replaced);
  V(1) = old != 0 && old != (u32)replaced;
  V(2) = DOS(LVO_dos_IoErr, 0);                                    /* the guest function ran: 0x1234 */
  V(3) = (*(u8 *)(dos + 14) >> 1) & 1;                             /* LIBF_CHANGED */
  /* Put the old one back, then patch again with a wrapper that chains to it. */
  V(4) = SETFUNC(dos, LVO_dos_IoErr, old) == (u32)replaced;       /* the value returned is our own patch */
  V(5) = DOS(LVO_dos_IoErr, 0);                                    /* 77 again: the trampoline behaves as the vector did */
  V(100) = SETFUNC(dos, LVO_dos_IoErr, chained);
  V(6) = DOS(LVO_dos_IoErr, 0);                                    /* chained: old() + 1 = 78 */
  V(7) = V(101);                                                   /* the wrapper really ran once per call */
  /* Negative controls: refused by name, and they leave the table alone. */
  SETFUNC(dos, -131, replaced);                                    /* not a multiple of 6 */
  SETFUNC(dos, 6, replaced);                                       /* positive */
  SETFUNC(dos, -60000, replaced);                                  /* past the table */
  V(8) = DOS(LVO_dos_IoErr, 0);                                    /* still the chained patch: 78 */
  /* Caches. */
  LVO(SYSBASE, LVO_exec_CacheClearU, 0, 0, 0, 0, 0, 0);
  LVO(SYSBASE, LVO_exec_CacheClearE, 4, 0, 0, 0, 0x20000, 0);
  V(9) = LVO(SYSBASE, LVO_exec_CacheControl, 0, 0, 0, 0, 0, 0);    /* old bits: EnableI = 1 */
  V(10) = LVO(SYSBASE, LVO_exec_CacheControl, 2, 2, 0, 0, 0, 0);   /* freeze: returns 1 */
  V(11) = LVO(SYSBASE, LVO_exec_CacheControl, 0, 0, 0, 0, 0, 0);   /* 3 */
  V(12) = LVO(SYSBASE, LVO_exec_CacheControl, 0, 3, 0, 0, 0, 0);   /* clear both: returns 3 */
  V(13) = LVO(SYSBASE, LVO_exec_CacheControl, 0x800 | 8, 0x800 | 8, 0, 0, 0, 0); /* ClearD/ClearI are write-only and the 020 has no D cache: returns 0 */
  V(14) = LVO(SYSBASE, LVO_exec_CacheControl, 0, 0, 0, 0, 0, 0);   /* still 0 */
  CLOSELIB(dos);
}

u32 replaced(void) { return 0x1234; }
u32 chained(void) {
  V(101)++;
  return ((u32 (*)(void))V(100))() + 1;
}
