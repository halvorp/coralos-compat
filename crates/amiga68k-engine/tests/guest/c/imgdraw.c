/* A GUEST-defined image class: DrawImageState / DrawImage send it IM_DRAW through its own dispatcher (rom/intuition/drawimagestate.c: impDraw =
 * MethodID, imp_RPort, imp_Offset {X, Y}, imp_State, imp_DrInfo). Written for this repo (MIT).
 * The dispatcher (below _start) records the message in RES[0..6]: rport, packed X<<16|Y, state, drinfo, object, class, count.
 * RES[8] the class, RES[9] the object, RES[10] the window's RastPort, RES[11] = 1 at the end. */
#include "rt.h"
#define WA 0x80000063u
void disp_(void);
void _start(void) {
  u32 intu = OPENLIB("intuition.library");
  u32 wtags[] = { WA + 3, 100, WA + 4, 60, 0, 0 };
  u32 win = LVO(intu, LVO_intuition_OpenWindowTagList, 0, 0, 0, 0, 0, (u32)wtags);
  u32 rp = *(u32 *)(win + 50);
  RES[10] = rp;
  u32 cl2 = LVOX(intu, LVO_intuition_MakeClass, 8, 0, 0, 0, 0, (u32)"imageclass", 0, 0);
  /* MakeClass(a0 = NULL id, a1 = "imageclass", a2 = NULL super ptr, d0 = instance size 8, d1 = flags 0) */
  RES[8] = cl2;
  *(u32 *)(cl2 + 8) = (u32)disp_;                                /* cl_Dispatcher.h_Entry */
  u32 obj = LVO(intu, LVO_intuition_NewObjectA, 0, 0, 0, 0, cl2, 0);
  RES[9] = obj;
  LVO(intu, LVO_intuition_DrawImage, 3, 4, 0, 0, rp, obj);        /* DrawImage(a0 = rp, a1 = image, d0 = 3, d1 = 4): state 0, no DrawInfo */
  RES[12] = RES[6];                                                 /* the count after DrawImage */
  /* DrawImageState(a0 = rp, a1 = image, d0 = left, d1 = top, d2 = state, a2 = drawinfo) */
  LVOX(intu, LVO_intuition_DrawImageState, 7, 9, 2, 0, rp, obj, 0x1234, 0);
  RES[11] = 1;
}
asm(".text\n"
    "disp_:\n"
    " move.l (%a1),%d0\n cmp.l #0x202,%d0\n beq.s 1f\n"
    " move.l 24(%a0),%a0\n move.l 8(%a0),%a3\n jmp (%a3)\n"
    "1: lea 0x1F000,%a3\n move.l 4(%a1),(%a3)\n move.l 8(%a1),4(%a3)\n move.l 12(%a1),8(%a3)\n move.l 16(%a1),12(%a3)\n"
    " move.l %a2,16(%a3)\n move.l %a0,20(%a3)\n addq.l #1,24(%a3)\n moveq #0,%d0\n rts\n");
