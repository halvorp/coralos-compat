/* IDCMP classes handed to a requester close it early: EasyRequestArgs returns -1 and stores the class; AutoRequest maps the class to
 * its positive (pFlag) or negative (nFlag) answer (written for this repo, MIT). Results in RES[]. */
#include "rt.h"
#define V(i) (*(volatile u32 *)&RES[i])
#define IB(off, d0, d1, d2, d3, a0, a1, a2, a3) ({ \
  register u32 _d0 asm("d0") = (u32)(d0); register u32 _d1 asm("d1") = (u32)(d1); register u32 _d2 asm("d2") = (u32)(d2); register u32 _d3 asm("d3") = (u32)(d3); \
  register u32 _a0 asm("a0") = (u32)(a0); register u32 _a1 asm("a1") = (u32)(a1); register u32 _a2 asm("a2") = (u32)(a2); register u32 _a3 asm("a3") = (u32)(a3); \
  asm volatile("move.l %%a6,-(%%sp)\n\tmove.l %[b],%%a6\n\tjsr %c[o](%%a6)\n\tmove.l (%%sp)+,%%a6" : "+r"(_d0), "+r"(_d1), "+r"(_a0), "+r"(_a1) \
               : "r"(_d2), "r"(_d3), "r"(_a2), "r"(_a3), [b] "r"(ib), [o] "i"(off) : "cc", "memory"); _d0; })
struct Easy { u32 size, flags; const char *title, *text, *gadgets; };
struct IText { u8 fp, bp, dm; i16 left, top; void *font; const char *text; struct IText *next; };
void _start(void) {
  u32 ib = OPENLIB("intuition.library");
  struct Easy es = { 20, 0, "Disk", "Insert the disk", "Retry|Cancel" };
  u32 flags = 0x8000;                       /* IDCMP_DISKINSERTED */
  V(0) = IB(LVO_intuition_EasyRequestArgs, 0, 0, 0, 0, 0, &es, &flags, 0);   /* the host reports DISKINSERTED: -1 */
  V(1) = flags;
  flags = 0x8000;
  V(2) = IB(LVO_intuition_EasyRequestArgs, 0, 0, 0, 0, 0, &es, &flags, 0);   /* another class is reported (ignored); the user clicks Retry: 1 */
  V(3) = flags;                              /* untouched */
  struct IText neg = { 0, 0, 0, 6, 3, 0, "No", 0 }, pos = { 0, 0, 0, 6, 3, 0, "Yes", 0 }, body = { 0, 0, 0, 10, 8, 0, "Insert disk?", 0 };
  V(4) = IB(LVO_intuition_AutoRequest, 0x8000, 0x4000, 200, 60, 0, &body, &pos, &neg);   /* DISKREMOVED = nFlag: FALSE */
  V(5) = IB(LVO_intuition_AutoRequest, 0x8000, 0x4000, 200, 60, 0, &body, &pos, &neg);   /* DISKINSERTED = pFlag: TRUE */
  CLOSELIB(ib);
}
