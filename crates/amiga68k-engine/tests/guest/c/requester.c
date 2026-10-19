/* intuition requesters: EasyRequestArgs, AutoRequest, BuildEasyRequestArgs + SysReqHandler + FreeSysRequest (written for this repo, MIT).
 * The test answers each request in order (RecordingGuestUi::request_answers) and checks RES[]. */
#include "rt.h"
#define V(i) (*(volatile u32 *)&RES[i])
#define IB(off, d0, d1, d2, d3, a0, a1, a2, a3) ({ \
  register u32 _d0 asm("d0") = (u32)(d0); register u32 _d1 asm("d1") = (u32)(d1); register u32 _d2 asm("d2") = (u32)(d2); register u32 _d3 asm("d3") = (u32)(d3); \
  register u32 _a0 asm("a0") = (u32)(a0); register u32 _a1 asm("a1") = (u32)(a1); register u32 _a2 asm("a2") = (u32)(a2); register u32 _a3 asm("a3") = (u32)(a3); \
  asm volatile("move.l %%a6,-(%%sp)\n\tmove.l %[b],%%a6\n\tjsr %c[o](%%a6)\n\tmove.l (%%sp)+,%%a6" : "+r"(_d0), "+r"(_d1), "+r"(_a0), "+r"(_a1) \
               : "r"(_d2), "r"(_d3), "r"(_a2), "r"(_a3), [b] "r"(ib), [o] "i"(off) : "cc", "memory"); _d0; })
struct Easy { u32 size, flags; const char *title, *text, *gadgets; };
struct IText { u8 fp, bp, dm; i16 left, top; void *font; const char *text; struct IText *next; };

#define SIGBIT(win) (*(volatile u8 *)(*(volatile u32 *)((win) + 86) + 15))

void _start(void) {
  u32 ib = OPENLIB("intuition.library");
  struct Easy es = { 20, 0, "Save", "Save %s (%ld bytes)?", "Yes|No|Cancel" };
  struct __attribute__((packed)) { u32 s; u32 n; } args = { (u32)"notes", 1234 };
  /* The host answers the leftmost, middle and rightmost button: 1, 2 and 0 (Intuition's GadgetIDs). */
  V(0) = IB(LVO_intuition_EasyRequestArgs, 0, 0, 0, 0, 0, &es, 0, &args);
  V(1) = IB(LVO_intuition_EasyRequestArgs, 0, 0, 0, 0, 0, &es, 0, &args);
  V(2) = IB(LVO_intuition_EasyRequestArgs, 0, 0, 0, 0, 0, &es, 0, &args);
  /* One button: it is the rightmost, so it returns 0. A title-less EasyStruct is "System Request". */
  struct Easy one = { 20, 0, 0, "Done", "OK" };
  V(3) = IB(LVO_intuition_EasyRequestArgs, 0, 0, 0, 0, 0, &one, 0, 0);
  /* A NULL EasyStruct: 0 without asking (BuildEasyRequestArgs returns NULL). */
  V(4) = IB(LVO_intuition_EasyRequestArgs, 0, 0, 0, 0, 0, 0, 0, 0);
  /* AutoRequest: TRUE for the positive text, FALSE for the negative. */
  struct IText neg = { 0, 0, 0, 6, 3, 0, "Cancel", 0 }, pos = { 0, 0, 0, 6, 3, 0, "Delete", 0 };
  struct IText l2 = { 0, 0, 0, 10, 20, 0, "really?", 0 }, body = { 0, 0, 0, 10, 8, 0, "Delete the file", &l2 };
  V(5) = IB(LVO_intuition_AutoRequest, 1, 2, 300, 80, 0, &body, &pos, &neg);
  V(6) = IB(LVO_intuition_AutoRequest, 1, 2, 300, 80, 0, &body, &pos, &neg);
  /* Without a positive text there is one button: always FALSE. */
  V(7) = IB(LVO_intuition_AutoRequest, 1, 2, 300, 80, 0, &body, 0, &neg);
  /* BuildEasyRequestArgs: a real Window with a UserPort; nothing answered yet -> -2. Wait on its port, then SysReqHandler returns the number. */
  u32 flags = 0;
  u32 req = IB(LVO_intuition_BuildEasyRequestArgs, 0, 0, 0, 0, 0, &es, 0, &args);
  V(10) = req != 0;
  V(11) = IB(LVO_intuition_SysReqHandler, 0, 0, 0, 0, req, &flags, 0, 0);
  LVO(SYSBASE, LVO_exec_Wait, 1u << SIGBIT(req), 0, 0, 0, 0, 0);
  V(12) = IB(LVO_intuition_SysReqHandler, 0, 0, 0, 0, req, &flags, 0, 0);
  V(13) = IB(LVO_intuition_SysReqHandler, 0, 0, 0, 0, req, &flags, 0, 0);     /* the answer was consumed: -2 again */
  IB(LVO_intuition_FreeSysRequest, 0, 0, 0, 0, req, 0, 0, 0);
  /* WaitInput = TRUE: the call itself waits for the answer. */
  req = IB(LVO_intuition_BuildEasyRequestArgs, 0, 0, 0, 0, 0, &es, 0, &args);
  V(14) = IB(LVO_intuition_SysReqHandler, 1, 0, 0, 0, req, 0, 0, 0);
  IB(LVO_intuition_FreeSysRequest, 0, 0, 0, 0, req, 0, 0, 0);
  /* Freeing an unanswered requester withdraws it. */
  req = IB(LVO_intuition_BuildEasyRequestArgs, 0, 0, 0, 0, 0, &es, 0, &args);
  IB(LVO_intuition_FreeSysRequest, 0, 0, 0, 0, req, 0, 0, 0);
  V(15) = 1;
  /* NULL and 1 are the results of a requester that could not open: 0 and 1. */
  V(16) = IB(LVO_intuition_SysReqHandler, 0, 0, 0, 0, 0, 0, 0, 0);
  V(17) = IB(LVO_intuition_SysReqHandler, 0, 0, 0, 0, 1, 0, 0, 0);
  CLOSELIB(ib);
}
