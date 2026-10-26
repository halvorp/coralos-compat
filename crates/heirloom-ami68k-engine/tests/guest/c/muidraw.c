/* A self-drawing MUI class written for this repo (MIT): MUI_CreateCustomClass over Area.mui, a dispatcher with AskMinMax / Setup / Show / Draw /
 * Hide / Cleanup in the way Zune's custom classes are written (DoSuperMethod first, then the class's own work), drawing through _rp(obj) in the
 * object's box. The macros are Zune's own plain offsets over the object: struct __dummyAreaData__ { MUI_NotifyData mnd (28 bytes);
 * MUI_AreaData mad } with mad_RenderInfo 0, mad_MinMax 12, mad_Box 24, mad_Flags 36, and MUI_RenderInfo's mri_Window 16, mri_RastPort 20.
 * Results in RES[]; RES[63] = 1 asks the program to close the window and dispose the application too; 2 builds the tree and never opens it. */
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

extern u32 disp(void);
static u32 do_method(u32 obj, u32 msg);
u32 dispatch_c(u32 cl, u32 msg, u32 obj);

void _start(void) {
  u32 mui = OPENLIB("muimaster.library"), intui = OPENLIB("intuition.library");
  V(60) = OPENLIB("graphics.library");
  u32 mcc = LVOA(mui, LVO_MUI_CreateCustomClass, 0, 0, (u32)"Area.mui", 0, (u32)disp);
  u32 cl = mcc ? rd32((const void *)(mcc + 24)) : 0;
  u32 none[2] = { 0, 0 };
  u32 obj = cl ? LVOA(intui, LVO_intuition_NewObjectA, 0, cl, 0, (u32)none, 0) : 0;
  u32 wt[6] = { MUIA_Window_Title, (u32)"Plane", MUIA_Window_RootObject, obj, 0, 0 };
  u32 win = obj ? LVOA(mui, LVO_MUI_NewObjectA, 0, (u32)"Window.mui", (u32)wt, 0, 0) : 0;
  u32 at[4] = { MUIA_Application_Window, win, 0, 0 };
  u32 app = win ? LVOA(mui, LVO_MUI_NewObjectA, 0, (u32)"Application.mui", (u32)at, 0, 0) : 0;
  V(0) = mcc; V(1) = cl; V(2) = obj; V(3) = app; V(4) = win;
  if (!app || V(63) == 2) return;                          /* mode 2: built, never opened */
  u32 open[4] = { MUIA_Window_Open, 1, 0, 0 };
  LVOA(intui, LVO_intuition_SetAttrsA, 0, win, (u32)open, 0, 0);
  V(17) = 1;                                              /* the window opened and was drawn before SetAttrs returned */
  if (V(63) == 1) {
    u32 close[4] = { MUIA_Window_Open, 0, 0, 0 };
    LVOA(intui, LVO_intuition_SetAttrsA, 0, win, (u32)close, 0, 0);
    LVOA(mui, LVO_MUI_DisposeObject, 0, app, 0, 0, 0);
    V(18) = 1;
  } else {
    if (V(63) == 3) { V(19) = 1; LVOA(mui, LVO_MUI_Redraw, 3, obj, 0, 0, 0); V(20) = 1; }   /* mode 3: MUI_Redraw(obj, MADF_DRAWOBJECT | DRAWUPDATE) */
    u32 exec_msg[1] = { 0x90420103u };                                 /* MUIM_Application_Execute: the program's own main loop */
    do_method(app, (u32)exec_msg);
  }
}

/* amiga.lib DoMethodA: o_Class is at obj - 4, its h_Entry at 8 */
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

u32 dispatch_c(u32 cl, u32 msg, u32 obj) {
  u32 id = rd32((const void *)msg);
  u32 r = do_super(cl, msg, obj);
  switch (id) {
  case MUIM_Setup: V(5)++; V(15) = rd32((const void *)(A_RI(obj) + 4)) != 0; break;          /* mri_Screen is set at Setup */
  case MUIM_AskMinMax: {
    i16 *mm = (i16 *)rd32((const void *)(msg + 4));                                          /* Min, Max, Def x/y */
    V(6)++; mm[0] += 40; mm[1] += 20; mm[2] = 10000; mm[3] = 10000; mm[4] = 80; mm[5] = 40;
    break; }
  case MUIM_Show: V(7)++; V(16) = rd32((const void *)(A_RI(obj) + 20)); break;               /* mri_RastPort is valid between Show and Hide */
  case MUIM_Draw: {
    u32 flags = rd32((const void *)(msg + 4)), rp = rd32((const void *)(A_RI(obj) + 20));
    V(8)++; V(11) = A_LEFT(obj); V(12) = A_TOP(obj); V(13) = A_W(obj); V(14) = A_H(obj);
    if (flags & 1) {
      u32 g = V(60); i32 x = A_LEFT(obj), y = A_TOP(obj), w = A_W(obj), h = A_H(obj);
      LVO(g, LVO_graphics_SetAPen, 3, 0, 0, 0, 0, rp);
      LVO(g, LVO_graphics_RectFill, x, y, x + w - 1, y + h - 1, 0, rp);
      LVO(g, LVO_graphics_SetAPen, 1, 0, 0, 0, 0, rp);
      LVO(g, LVO_graphics_Move, x, y, 0, 0, 0, rp);
      LVO(g, LVO_graphics_Draw, x + w - 1, y + h - 1, 0, 0, 0, rp);
      LVO(g, LVO_graphics_Move, x + 4, y + h - 4, 0, 0, 0, rp);
      LVO(g, LVO_graphics_Text, 5, 0, 0, 0, (u32)"Plane", rp);
    }
    break; }
  case MUIM_Hide: V(9)++; break;
  case MUIM_Cleanup: V(10)++; break;
  }
  return r;
}
/* the class dispatcher entry: a0 = class, a1 = message, a2 = object -> C arguments */
__asm__(".text\n\t.globl disp\ndisp:\n\tmove.l %a2,-(%sp)\n\tmove.l %a1,-(%sp)\n\tmove.l %a0,-(%sp)\n\tbsr.l dispatch_c\n\tlea 12(%sp),%sp\n\trts\n");
