/* A MUI class written for this repo (MIT) that LISTENS: over Area.mui, in Setup it registers (the way Zune's Clock and its pointer-preview class
 * do) a timer input handler (MUIIHNF_TIMER, ihn_Millis 1000) that calls its private method 0x80420ff1, a signal input handler (ihn_Signals =
 * a signal it AllocSignal()s) for the same method, and two window event handlers (A: priority 0, MOUSEBUTTONS|RAWKEY, eats a SELECTDOWN; B: priority -1,
 * MOUSEBUTTONS|MOUSEMOVE) for MUIM_HandleEvent; in Cleanup it removes them. Nodes live at RES+0x400.. (struct MUI_InputHandlerNode: ihn_Object 8,
 * ihn_Millis/ihn_Signals 12, ihn_Flags 16, ihn_Method 20; struct MUI_EventHandlerNode: ehn_Priority 9, ehn_Flags 10, ehn_Object 12, ehn_Class 16,
 * ehn_Events 20). Results in RES[]; RES[63] = 1 closes the window after opening (mode 1), 2 never calls Cleanup's inverse (handlers stay: dispose). */
#include "rt.h"
#define V(i) (*(volatile u32 *)&RES[i])
#define MAD(o) ((o) + 28)
#define A_RI(o) rd32((const void *)(MAD(o) + 0))
#define A_LEFT(o) (*(i16 *)(MAD(o) + 24))
#define A_TOP(o) (*(i16 *)(MAD(o) + 26))
#define A_W(o) (*(i16 *)(MAD(o) + 28))
#define A_H(o) (*(i16 *)(MAD(o) + 30))
#define MUIM_AskMinMax 0x80423874u
#define MUIM_Cleanup 0x8042d985u
#define MUIM_Draw 0x80426f3fu
#define MUIM_Hide 0x8042f20fu
#define MUIM_Setup 0x80428354u
#define MUIM_Show 0x8042cc84u
#define MUIA_Application_Window 0x8042bfe0u
#define MUIA_Window_Open 0x80428aa0u
#define MUIA_Window_RootObject 0x8042cba5u
#define MUIA_Window_Title 0x8042ad3du
#define LVO_MUI_NewObjectA (-30)
#define LVO_MUI_DisposeObject (-36)
#define LVO_MUI_CreateCustomClass (-108)
#define LVO_MUI_Redraw (-102)
/* a library call with A0..A3 and D0 (the rt.h macros stop at A1) */
#define LVOA(base, off, D0, A0, A1, A2, A3) ({ \
  register u32 _d0 asm("d0") = (u32)(D0); register u32 _a0 asm("a0") = (u32)(A0); register u32 _a1 asm("a1") = (u32)(A1); \
  register u32 _a2 asm("a2") = (u32)(A2); register u32 _a3 asm("a3") = (u32)(A3); \
  asm volatile("move.l %%a6,-(%%sp)\n\tmove.l %[b],%%a6\n\tjsr %c[o](%%a6)\n\tmove.l (%%sp)+,%%a6" : "+r"(_d0), "+r"(_a0), "+r"(_a1), "+r"(_a2), "+r"(_a3) \
               : [b] "r"((u32)(base)), [o] "i"(off) : "d1", "cc", "memory"); _d0; })


#define MUIM_AddInput 0x8042f099u
#define MUIM_RemInput 0x8042e7afu
#define MUIM_AddEvent 0x804203b7u
#define MUIM_RemEvent 0x8042679eu
#define MUIM_HandleEvent 0x80426d66u
#define MUIM_Tick 0x80420ff1u
#define NODE(i) ((u32)RES + 0x400 + 0x40 * (i))
#define IDCMP_MOUSEBUTTONS 0x8u
#define IDCMP_MOUSEMOVE 0x10u
#define IDCMP_RAWKEY 0x400u

extern u32 disp(void);
static u32 do_method(u32 obj, u32 msg);
u32 dispatch_c(u32 cl, u32 msg, u32 obj);

void _start(void) {
  u32 mui = OPENLIB("muimaster.library"), intui = OPENLIB("intuition.library");
  u32 mcc = LVOA(mui, LVO_MUI_CreateCustomClass, 0, 0, (u32)"Area.mui", 0, (u32)disp);
  u32 cl = mcc ? rd32((const void *)(mcc + 24)) : 0;
  u32 none[2] = { 0, 0 };
  u32 obj = cl ? LVOA(intui, LVO_intuition_NewObjectA, 0, cl, 0, (u32)none, 0) : 0;
  u32 wt[6] = { MUIA_Window_Title, (u32)"Listener", MUIA_Window_RootObject, obj, 0, 0 };
  u32 win = obj ? LVOA(mui, LVO_MUI_NewObjectA, 0, (u32)"Window.mui", (u32)wt, 0, 0) : 0;
  u32 at[4] = { MUIA_Application_Window, win, 0, 0 };
  u32 app = win ? LVOA(mui, LVO_MUI_NewObjectA, 0, (u32)"Application.mui", (u32)at, 0, 0) : 0;
  V(0) = mcc; V(1) = cl; V(2) = obj; V(3) = app; V(4) = win;
  if (!app) return;
  u32 open[4] = { MUIA_Window_Open, 1, 0, 0 };
  LVOA(intui, LVO_intuition_SetAttrsA, 0, win, (u32)open, 0, 0);
  V(17) = 1;
  if (V(63) == 1) {
    u32 close[4] = { MUIA_Window_Open, 0, 0, 0 };
    LVOA(intui, LVO_intuition_SetAttrsA, 0, win, (u32)close, 0, 0);
    V(18) = 1;
  }
  u32 exec_msg[1] = { 0x90420103u };
  do_method(app, (u32)exec_msg);
}

static u32 do_method(u32 obj, u32 msg) {
  u32 cl = rd32((const void *)(obj - 4)), entry = rd32((const void *)(cl + 8));
  register u32 _d0 asm("d0") = 0; register u32 _a0 asm("a0") = cl, _a1 asm("a1") = msg, _a2 asm("a2") = obj;
  asm volatile("jsr (%[e])" : "+r"(_d0), "+r"(_a0), "+r"(_a1), "+r"(_a2) : [e] "a"(entry) : "d1", "cc", "memory");
  return _d0;
}

static u32 do_super(u32 cl, u32 msg, u32 obj) {
  u32 sup = rd32((const void *)(cl + 24)), entry = rd32((const void *)(sup + 8));
  register u32 _d0 asm("d0") = 0; register u32 _a0 asm("a0") = sup, _a1 asm("a1") = msg, _a2 asm("a2") = obj;
  asm volatile("jsr (%[e])" : "+r"(_d0), "+r"(_a0), "+r"(_a1), "+r"(_a2) : [e] "a"(entry) : "d1", "cc", "memory");
  return _d0;
}

static void w32(u32 at, u32 v) { *(volatile u32 *)at = v; }
static void w16(u32 at, u32 v) { *(volatile u16 *)at = (u16)v; }
static void w8(u32 at, u32 v) { *(volatile u8 *)at = (u8)v; }
static u32 app_of(u32 obj) { return rd32((const void *)(rd32((const void *)obj) + 4)); }
static u32 win_of(u32 obj) { return rd32((const void *)A_RI(obj)); }
static u32 send1(u32 target, u32 id, u32 arg) { u32 m[2] = { id, arg }; return do_method(target, (u32)m); }

static void ehn(u32 n, u32 obj, u32 cl, i32 pri, u32 flags, u32 events) {
  w8(n + 9, pri); w16(n + 10, flags); w32(n + 12, obj); w32(n + 16, cl); w32(n + 20, events);
}

u32 dispatch_c(u32 cl, u32 msg, u32 obj) {
  u32 id = rd32((const void *)msg);
  switch (id) {
  case MUIM_Tick: V(6)++; return 0;                       /* the timer's method */
  case MUIM_Tick + 1: V(13)++; return 0;                  /* the signal handler's */
  case MUIM_Cleanup:                                       /* like Zune's classes: remove the handlers, THEN let the super class tear down */
    send1(app_of(obj), MUIM_RemInput, NODE(0));
    send1(app_of(obj), MUIM_RemInput, NODE(1));
    send1(win_of(obj), MUIM_RemEvent, NODE(2));
    send1(win_of(obj), MUIM_RemEvent, NODE(3));
    V(31) = 1;
    break;
  case MUIM_HandleEvent: {
    u32 im = rd32((const void *)(msg + 4));
    V(7)++; V(8) = rd32((const void *)(im + 20)); V(9) = *(u16 *)(im + 24); V(10) = *(i16 *)(im + 32); V(11) = *(i16 *)(im + 34);
    V(24) = *(u16 *)(im + 26);                           /* Qualifier */
    V(25) = rd32((const void *)(msg + 8));               /* muikey */
    V(26) = im ? rd32((const void *)(im + 44)) : 0;      /* IDCMPWindow */
    /* A handler eats a SELECTDOWN or any RAWKEY (RES[62] = 1 makes it eat nothing: the negative control). */
    if (V(62) != 1 && ((rd32((const void *)(im + 20)) == IDCMP_MOUSEBUTTONS && *(u16 *)(im + 24) == 0x68) || rd32((const void *)(im + 20)) == IDCMP_RAWKEY)) return 1;
    return 0; }
  }
  u32 r = do_super(cl, msg, obj);
  switch (id) {
  case MUIM_Setup:
    V(5)++;
    w32(NODE(0) + 8, obj); w16(NODE(0) + 12, 1000); w32(NODE(0) + 16, 1); w32(NODE(0) + 20, MUIM_Tick);   /* timer */
    w32(NODE(1) + 8, obj); V(44) = LVO(SYSBASE, LVO_exec_AllocSignal, (u32)-1, 0, 0, 0, 0, 0); w32(NODE(1) + 12, 1u << V(44)); w32(NODE(1) + 16, 0); w32(NODE(1) + 20, MUIM_Tick + 1); /* signal */
    ehn(NODE(2), obj, 0, 0, 0, IDCMP_MOUSEBUTTONS | IDCMP_RAWKEY);
    ehn(NODE(3), obj, cl, -1, 0, IDCMP_MOUSEBUTTONS | IDCMP_MOUSEMOVE);
    V(40) = send1(app_of(obj), MUIM_AddInput, NODE(0));
    V(41) = send1(app_of(obj), MUIM_AddInput, NODE(1));
    V(42) = send1(win_of(obj), MUIM_AddEvent, NODE(2));
    V(43) = send1(win_of(obj), MUIM_AddEvent, NODE(3));
    break;
  case MUIM_AskMinMax: {
    i16 *mm = (i16 *)rd32((const void *)(msg + 4));
    mm[0] += 40; mm[1] += 20; mm[2] = 10000; mm[3] = 10000; mm[4] = 80; mm[5] = 40;
    break; }
  }
  return r;
}
__asm__(".text\n\t.globl disp\ndisp:\n\tmove.l %a2,-(%sp)\n\tmove.l %a1,-(%sp)\n\tmove.l %a0,-(%sp)\n\tbsr.l dispatch_c\n\tlea 12(%sp),%sp\n\trts\n");
