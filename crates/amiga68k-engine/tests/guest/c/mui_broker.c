/* (_start must stay the first function: the helpers are always_inline.) An Application's commodity (written for this repo, MIT): MUIA_Application_Broker / BrokerPort / BrokerPri / BrokerHook / UseCommodities, and the
 * commands Exchange sends through BrokerCommand. RES[]: 0 app, 1 broker, 2 port, 3 BrokerPri (GetAttr), 4..8 BrokerCommand results (APPEAR, DISABLE,
 * ENABLE, KILL, unknown name), 9 = Active after DISABLE's NewInput, 10 = Active after ENABLE's, 11 = hook calls, 12..15 = the CxMsg ids the hook saw,
 * 16 = hook's object == app, 17 = CTRL_C set after KILL, 18 = NewInput's mask has the port signal, 20 = app2 (UseCommodities FALSE) broker,
 * 21 = app3 (no title) "Unnamed" BrokerCommand result, 22 = after dispose BrokerCommand("Notes"), 23 = hook calls before any command,
 * 24 = Active at birth. */
#include "rt.h"
#define V(i) (*(volatile u32 *)&RES[i])
#define MUIA_Application_Active 0x804260abu
#define MUIA_Application_Broker 0x8042dbceu
#define MUIA_Application_BrokerHook 0x80428f4bu
#define MUIA_Application_BrokerPort 0x8042e0adu
#define MUIA_Application_BrokerPri 0x8042c8d0u
#define MUIA_Application_Title 0x804281b8u
#define MUIA_Application_Version 0x8042b33fu
#define MUIA_Application_UseCommodities 0x80425ee5u
#define MUIM_Application_NewInput 0x80423ba6u
#define LVO_MUI_NewObjectA (-30)
#define LVO_MUI_DisposeObject (-36)
#define LVOA(base, off, D0, A0, A1, A2, A3) ({ \
  register u32 _d0 asm("d0") = (u32)(D0); register u32 _a0 asm("a0") = (u32)(A0); register u32 _a1 asm("a1") = (u32)(A1); \
  register u32 _a2 asm("a2") = (u32)(A2); register u32 _a3 asm("a3") = (u32)(A3); \
  asm volatile("move.l %%a6,-(%%sp)\n\tmove.l %[b],%%a6\n\tjsr %c[o](%%a6)\n\tmove.l (%%sp)+,%%a6" : "+r"(_d0), "+r"(_a0), "+r"(_a1), "+r"(_a2), "+r"(_a3) \
               : [b] "r"((u32)(base)), [o] "i"(off) : "d1", "cc", "memory"); _d0; })

extern u32 hook_entry(void);
u32 hook_c(u32 hook, u32 obj, u32 msg);
static u32 mui, intui, cx;
static u32 hook[5];

static inline __attribute__((always_inline)) u32 get(u32 obj, u32 attr) { u32 v = 0xDEAD; LVOA(intui, LVO_intuition_GetAttr, attr, obj, (u32)&v, 0, 0); return v; }
static inline __attribute__((always_inline)) u32 newinput(u32 app, u32 *sigs) { u32 m[2] = { MUIM_Application_NewInput, (u32)sigs }; u32 cl = rd32((const void *)(app - 4)), entry = rd32((const void *)(cl + 8));
  register u32 _d0 asm("d0") = 0; register u32 _a0 asm("a0") = cl, _a1 asm("a1") = (u32)m, _a2 asm("a2") = app;
  asm volatile("jsr (%[e])" : "+r"(_d0), "+r"(_a0), "+r"(_a1), "+r"(_a2) : [e] "a"(entry) : "d1", "cc", "memory"); return _d0; }
static inline __attribute__((always_inline)) u32 bcmd(const char *name, u32 cmd) { return LVOA(cx, LVO_commodities_BrokerCommand, cmd, name, 0, 0, 0); }

void _start(void) {
  mui = OPENLIB("muimaster.library"); intui = OPENLIB("intuition.library"); cx = OPENLIB("commodities.library");
  hook[2] = (u32)hook_entry;
  u32 at[10] = { MUIA_Application_Title, (u32)"Notes", MUIA_Application_Version, (u32)"$VER: Notes 1.0", MUIA_Application_BrokerPri, (u32)-5, MUIA_Application_BrokerHook, (u32)hook, 0, 0 };
  u32 app = LVOA(mui, LVO_MUI_NewObjectA, 0, "Application.mui", at, 0, 0);
  V(0) = app;
  V(1) = get(app, MUIA_Application_Broker); V(2) = get(app, MUIA_Application_BrokerPort); V(3) = get(app, MUIA_Application_BrokerPri);
  V(24) = get(app, MUIA_Application_Active);
  u32 sigs = 0;
  newinput(app, &sigs);                                   /* nothing was sent: the hook is not called */
  V(23) = V(11); V(18) = (sigs >> *(volatile u8 *)(V(2) + 15)) & 1;
  V(4) = bcmd("Notes", 19);  newinput(app, &sigs);        /* CXCMD_APPEAR: only the hook sees it */
  V(5) = bcmd("Notes", 15);  newinput(app, &sigs); V(9) = get(app, MUIA_Application_Active);   /* CXCMD_DISABLE */
  V(6) = bcmd("Notes", 17);  newinput(app, &sigs); V(10) = get(app, MUIA_Application_Active);  /* CXCMD_ENABLE */
  LVO(SYSBASE, LVO_exec_SetSignal, 0, 0x1000, 0, 0, 0, 0);
  V(7) = bcmd("Notes", 23);  newinput(app, &sigs);                                              /* CXCMD_KILL: CTRL_C */
  V(17) = (LVO(SYSBASE, LVO_exec_SetSignal, 0, 0, 0, 0, 0, 0) >> 12) & 1;
  V(8) = bcmd("Nonesuch", 17);
  /* an Application that does not want a commodity, and one without a title */
  u32 a2t[4] = { MUIA_Application_UseCommodities, 0, 0, 0 };
  u32 app2 = LVOA(mui, LVO_MUI_NewObjectA, 0, "Application.mui", a2t, 0, 0);
  V(20) = get(app2, MUIA_Application_Broker);
  u32 none[2] = { 0, 0 };
  u32 app3 = LVOA(mui, LVO_MUI_NewObjectA, 0, "Application.mui", none, 0, 0);
  V(21) = bcmd("Unnamed", 17);
  LVOA(mui, LVO_MUI_DisposeObject, 0, app, 0, 0, 0);
  V(22) = bcmd("Notes", 17);
  LVOA(mui, LVO_MUI_DisposeObject, 0, app2, 0, 0, 0);
  LVOA(mui, LVO_MUI_DisposeObject, 0, app3, 0, 0, 0);
  V(25) = 1;
}

u32 hook_c(u32 hook_, u32 obj, u32 msg) {
  u32 n = V(11);
  (void)hook_;
  if (n < 4) V(12 + n) = rd32((const void *)(msg + 24));
  V(16) = obj == V(0);
  V(11) = n + 1;
  return 0;
}
__asm__(".text\n\t.globl hook_entry\nhook_entry:\n\tmove.l %a1,-(%sp)\n\tmove.l %a2,-(%sp)\n\tmove.l %a0,-(%sp)\n\tbsr.l hook_c\n\tlea 12(%sp),%sp\n\trts\n");
