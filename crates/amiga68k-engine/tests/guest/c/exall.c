/* ExAll: data levels, continuation, match string, match hook (guest code called by the engine), errors (written for this repo, MIT). */
#include "rt.h"
#define DOS(off, d1, d2, d3, d4) LVO5(dos, off, 0, d1, d2, d3, d4, 0, 0)
#define IOERR() DOS(LVO_dos_IoErr, 0, 0, 0, 0)
#define W(a, off) rd32((void *)((a) + (off)))
extern void hook_entry(void);
#define exall(dos, lock, buf, size, data, ctl) LVO6(dos, LVO_dos_ExAll, 0, lock, buf, size, data, ctl, 0, 0)
COLD static u32 names_sum(u32 first) { u32 s = 0; for (u32 e = first; e; e = W(e, 0)) s += *(u8 *)W(e, 4); return s; }
void _start(void) {
  u32 dos = OPENLIB("dos.library");
  u32 lock = DOS(LVO_dos_Lock, "SYS:Dir", -2, 0, 0);
  u32 ctl = DOS(LVO_dos_AllocDosObject, 1, 0, 0, 0);         /* DOS_EXALLCONTROL */
  u32 big[200];
  /* ED_NAME, everything fits */
  RES[0] = exall(dos, lock, (u32)big, sizeof big, 1, ctl);   /* FALSE: done */
  RES[1] = IOERR();                                          /* 232 */
  RES[2] = W(ctl, 0);                                        /* eac_Entries: 4 */
  RES[3] = names_sum((u32)big);                              /* a b c S = 97+98+99+83 = 377 */
  u32 last = (u32)big;
  while (W(last, 0)) last = W(last, 0);
  RES[4] = W(last, 0);                                       /* ed_Next of the last record: 0 */
  /* ED_COMMENT */
  RES[5] = exall(dos, lock, (u32)big, sizeof big, 6, ctl);
  u32 e = (u32)big;
  RES[6] = strcmp((const char *)W(e, 4), "a.txt") == 0 && W(e, 12) == 3 && W(e, 8) == (u32)-3;   /* name, size, ST_FILE */
  RES[7] = strcmp((const char *)W(e, 32), "hello") == 0;     /* ed_Comment */
  e = W(W(W(e, 0), 0), 0);                                   /* the 4th: Sub */
  RES[8] = strcmp((const char *)W(e, 4), "Sub") == 0 && W(e, 8) == 2;
  /* small buffer: continuation through eac_LastKey */
  u32 total = 0, calls = 0, sum = 0, r;
  u32 small[16];                                             /* 64 bytes at level 3: 1-2 records at a time */
  *(volatile u32 *)(ctl + 4) = 0;
  do {
    r = exall(dos, lock, (u32)small, sizeof small, 3, ctl);
    total += W(ctl, 0); calls++;
    sum += names_sum((u32)small);
  } while (r != 0 && calls < 20);
  RES[9] = total;                                            /* 4 */
  RES[10] = sum;                                             /* 377: every entry exactly once */
  RES[11] = calls > 1;                                       /* it really took several calls */
  /* match string */
  u32 pat[12];
  DOS(LVO_dos_ParsePattern, "#?.txt", pat, 48, 0);
  *(volatile u32 *)(ctl + 8) = (u32)pat;
  RES[12] = exall(dos, lock, (u32)big, sizeof big, 1, ctl);
  RES[13] = W(ctl, 0);                                       /* 2: a.txt b.txt */
  /* match hook, called as guest code */
  u32 hook[5] = {0, 0, (u32)hook_entry, 0, 0};
  *(volatile u32 *)(ctl + 8) = 0;
  *(volatile u32 *)(ctl + 12) = (u32)hook;
  RES[14] = exall(dos, lock, (u32)big, sizeof big, 3, ctl);
  RES[15] = W(ctl, 0);                                       /* hook keeps only size >= 5: b.txt -> 1 */
  RES[16] = RES[40];                                         /* the hook counted its own calls in RES[40]: all 4 entries */
  RES[17] = strcmp((const char *)W((u32)big, 4), "b.txt") == 0;
  *(volatile u32 *)(ctl + 12) = 0;
  /* too small for one record */
  RES[18] = exall(dos, lock, (u32)big, 8, 1, ctl);
  RES[19] = IOERR();                                         /* 303 */
  RES[20] = exall(dos, lock, (u32)big, sizeof big, 8, ctl);  /* no such level */
  RES[21] = IOERR();                                         /* 115 */
  u32 flock = DOS(LVO_dos_Lock, "SYS:Dir/a.txt", -2, 0, 0);
  RES[22] = exall(dos, flock, (u32)big, sizeof big, 1, ctl); /* a file is not a directory */
  RES[23] = IOERR();                                         /* 212 */
  /* ExAllEnd mid-scan, then a restart from the top */
  r = exall(dos, lock, (u32)small, sizeof small, 3, ctl);
  RES[24] = r != 0 && W(ctl, 4) != 0;
  LVO6(dos, LVO_dos_ExAllEnd, 0, lock, (u32)small, sizeof small, 3, ctl, 0, 0);
  RES[25] = W(ctl, 4);                                       /* eac_LastKey reset */
  DOS(LVO_dos_FreeDosObject, 1, ctl, 0, 0);
  DOS(LVO_dos_UnLock, flock, 0, 0, 0);
  DOS(LVO_dos_UnLock, lock, 0, 0, 0);
  CLOSELIB(dos);
}
/* A hook entry: A0 = hook, A1 = ExAllData, A2 = LONG *type -> BOOL. Keep entries of 5 bytes or more. */
COLD u32 hook_body(u32 hook, u32 ead, u32 type_ptr) { (void)hook; RES[40]++; return rd32((void *)type_ptr) >= 3 && W(ead, 12) >= 5; }
__asm__(".text\n.globl hook_entry\nhook_entry:\n\tmove.l %a2,-(%sp)\n\tmove.l %a1,-(%sp)\n\tmove.l %a0,-(%sp)\n\tbsr.l hook_body\n\tlea 12(%sp),%sp\n\trts\n");
