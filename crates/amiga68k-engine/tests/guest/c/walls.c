/* The walls Directory Opus hit after CreateNewProc, each with a negative control: public ports, low-memory handlers,
 * commodities objects and ParseIX, asl requests, workbench app registrations, StartNotify, DoPkt(ACTION_DISK_INFO),
 * display-mode queries, byte pixel lines, Flood, and the small intuition calls. Written for this repo (MIT). */
#include "rt.h"
#define S(p) ((const char *)(p))
#define V(i) (*(volatile u32 *)&RES[i])
#define CALL(base, off, D0, D1, D2, D3, A0, A1, A2, A3) ({ \
  register u32 _d0 asm("d0") = (u32)(D0); register u32 _d1 asm("d1") = (u32)(D1); register u32 _d2 asm("d2") = (u32)(D2); register u32 _d3 asm("d3") = (u32)(D3); \
  register u32 _a0 asm("a0") = (u32)(A0); register u32 _a1 asm("a1") = (u32)(A1); register u32 _a2 asm("a2") = (u32)(A2); register u32 _a3 asm("a3") = (u32)(A3); \
  asm volatile("move.l %%a6,-(%%sp)\n\tmove.l %[b],%%a6\n\tjsr %c[o](%%a6)\n\tmove.l (%%sp)+,%%a6" \
    : "+r"(_d0), "+r"(_d1), "+r"(_a0), "+r"(_a1) : "r"(_d2), "r"(_d3), "r"(_a2), "r"(_a3), [b] "r"((u32)(base)), [o] "i"(off) : "cc", "memory"); _d0; })
#define EX(off, D0, A0, A1) CALL(SYSBASE, off, D0, 0, 0, 0, A0, A1, 0, 0)

void _start(void) {
  u32 dos = OPENLIB("dos.library"), gfx = OPENLIB("graphics.library"), intu = OPENLIB("intuition.library");
  u32 cx = OPENLIB("commodities.library"), asl = OPENLIB("asl.library"), wb = OPENLIB("workbench.library");
  /* --- public ports --- */
  u32 port = EX(LVO_exec_CreateMsgPort, 0, 0, 0);
  *(u32 *)(port + 10) = (u32)"walls.port";
  EX(LVO_exec_AddPort, 0, 0, port);
  V(0) = EX(LVO_exec_FindPort, 0, 0, (u32)"walls.port") == port;
  V(1) = EX(LVO_exec_FindPort, 0, 0, (u32)"no.such.port");               /* NULL */
  EX(LVO_exec_RemPort, 0, 0, port);
  V(2) = EX(LVO_exec_FindPort, 0, 0, (u32)"walls.port");                  /* gone */
  EX(LVO_exec_RemPort, 0, 0, port);                                       /* refused: not on the list */
  /* --- low-memory handlers --- */
  u32 intr[6] = { 0 };
  EX(LVO_exec_AddMemHandler, 0, 0, (u32)intr);
  EX(LVO_exec_RemMemHandler, 0, 0, (u32)intr);
  EX(LVO_exec_RemMemHandler, 0, 0, (u32)intr);                            /* refused */
  /* --- commodities --- */
  {
    u8 nb[26]; memset(nb, 0, sizeof nb);
    nb[0] = 5; *(u32 *)(nb + 2) = (u32)"Walls"; *(u16 *)(nb + 14) = 1; nb[18] = 0; *(u32 *)(nb + 20) = port;
    u32 err = 99;
    u32 br = CALL(cx, LVO_commodities_CxBroker, (u32)&err, 0, 0, 0, (u32)nb, 0, 0, 0);
    V(3) = br != 0 && err == 0;
    V(4) = CALL(cx, LVO_commodities_CxObjType, 0, 0, 0, 0, br, 0, 0, 0);                       /* CX_BROKER 6 */
    u32 err2 = 99;
    V(5) = CALL(cx, LVO_commodities_CxBroker, (u32)&err2, 0, 0, 0, (u32)nb, 0, 0, 0);         /* NBU_UNIQUE duplicate: NULL */
    V(6) = err2;                                                                                 /* CBERR_DUP 2 */
    nb[0] = 4; u32 err3 = 99;
    CALL(cx, LVO_commodities_CxBroker, (u32)&err3, 0, 0, 0, (u32)nb, 0, 0, 0);
    V(7) = err3;                                                                                 /* CBERR_VERSION 3 */
    u32 filt = CALL(cx, LVO_commodities_CreateCxObj, 1, 0, 0, 0, (u32)"ctrl alt f1", 0, 0, 0);
    u32 send = CALL(cx, LVO_commodities_CreateCxObj, 3, 0, 0, 0, port, 7, 0, 0);
    CALL(cx, LVO_commodities_AttachCxObj, 0, 0, 0, 0, filt, send, 0, 0);
    CALL(cx, LVO_commodities_AttachCxObj, 0, 0, 0, 0, br, filt, 0, 0);
    V(8) = CALL(cx, LVO_commodities_ActivateCxObj, 1, 0, 0, 0, br, 0, 0, 0);                 /* was inactive: 0 */
    V(9) = CALL(cx, LVO_commodities_ActivateCxObj, 1, 0, 0, 0, br, 0, 0, 0);                 /* now it was active: 1 */
    u8 ix[12]; memset(ix, 0, sizeof ix); ix[0] = 2;
    V(10) = CALL(cx, LVO_commodities_ParseIX, 0, 0, 0, 0, (u32)"ctrl alt f1", (u32)ix, 0, 0);   /* 0 */
    V(11) = *(u16 *)(ix + 2);                                                                    /* f1 = 0x50 */
    V(12) = *(u16 *)(ix + 6);                                                                    /* control 0x08 | alt 0x30 */
    V(13) = CALL(cx, LVO_commodities_ParseIX, 0, 0, 0, 0, (u32)"f1 f2", (u32)ix, 0, 0);       /* trailing token: -1 */
    CALL(cx, LVO_commodities_SetFilterIX, 0, 0, 0, 0, filt, (u32)ix, 0, 0);
    CALL(cx, LVO_commodities_DeleteCxObjAll, 0, 0, 0, 0, br, 0, 0, 0);
    CALL(cx, LVO_commodities_CxObjType, 0, 0, 0, 0, filt, 0, 0, 0);                            /* freed with the tree: refused */
  }
  /* --- asl --- */
  {
    u32 tags[] = { 0x80080003, 5, 0x80080006, 77, 0, 0 };                                       /* ASLFR_InitialLeftEdge, InitialHeight */
    u32 rq = CALL(asl, LVO_asl_AllocAslRequest, 0, 0, 0, 0, (u32)tags, 0, 0, 0);
    V(14) = rq != 0 && *(i16 *)(rq + 22) == 5 && *(i16 *)(rq + 28) == 77;
    V(15) = CALL(asl, LVO_asl_AslRequest, 0, 0, 0, 0, rq, 0, 0, 0);                            /* no UI: FALSE */
    CALL(asl, LVO_asl_FreeAslRequest, 0, 0, 0, 0, rq, 0, 0, 0);
    CALL(asl, LVO_asl_FreeAslRequest, 0, 0, 0, 0, rq, 0, 0, 0);                                 /* refused: freed already */
    V(16) = CALL(asl, LVO_asl_AllocAslRequest, 1, 0, 0, 0, 0, 0, 0, 0);                         /* a font requester: refused, NULL */
  }
  /* --- workbench --- */
  {
    u32 ai = CALL(wb, LVO_workbench_AddAppIconA, 1, 2, 0, 0, (u32)"icon", port, 0, 0);
    V(17) = ai != 0;
    V(18) = CALL(wb, LVO_workbench_AddAppIconA, 1, 2, 0, 0, (u32)"icon", 0, 0, 0);               /* NULL message port: NULL */
    CALL(wb, LVO_workbench_AddAppWindowA, 3, 0, 0, 0, 0x1234, port, 0, 0);                      /* not a window: refused */
    CALL(wb, -66 /* RemoveAppIcon */, 0, 0, 0, 0, ai, 0, 0, 0);
    CALL(wb, -66, 0, 0, 0, 0, ai, 0, 0, 0);                                                      /* refused: removed already */
  }
  /* --- StartNotify --- */
  {
    u32 nr[7] = { 0 };
    nr[0] = (u32)"RAM:foo";
    V(19) = CALL(dos, LVO_dos_StartNotify, 0, (u32)nr, 0, 0, 0, 0, 0, 0);
    V(20) = strcmp(S(nr[1]), "RAM:foo") == 0;                                                   /* nr_FullName */
    CALL(dos, LVO_dos_StartNotify, 0, (u32)nr, 0, 0, 0, 0, 0, 0);                                /* refused: already started */
    CALL(dos, LVO_dos_EndNotify, 0, (u32)nr, 0, 0, 0, 0, 0, 0);
    CALL(dos, LVO_dos_EndNotify, 0, (u32)nr, 0, 0, 0, 0, 0, 0);                                  /* refused */
  }
  /* --- DoPkt --- */
  {
    u32 dev = CALL(dos, LVO_dos_GetDeviceProc, 0, (u32)"RAM:", 0, 0, 0, 0, 0, 0);
    u32 portp = rd32((void *)dev);
    u32 info[9] = { 0 };
    V(21) = CALL(dos, LVO_dos_DoPkt, 0, portp, 25, ((u32)info) >> 2, 0, 0, 0, 0);               /* ACTION_DISK_INFO -> DOSTRUE */
    V(22) = info[6];                                                                             /* id_DiskType: DOS\0 */
    CALL(dos, LVO_dos_DoPkt, 0, portp, 1234, 0, 0, 0, 0, 0);                                     /* an action nobody models: refused */
    CALL(dos, LVO_dos_DoPkt, 0, 0x1234, 25, 0, 0, 0, 0, 0);                                      /* not a device port: refused */
  }
  /* --- display modes and pixel lines --- */
  {
    u32 scr = CALL(intu, LVO_intuition_LockPubScreen, 0, 0, 0, 0, 0, 0, 0, 0);
    V(23) = CALL(gfx, LVO_graphics_GetVPModeID, 0, 0, 0, 0, scr + 44, 0, 0, 0);                 /* the Workbench mode 0x29000 */
    V(24) = CALL(gfx, LVO_graphics_GetVPModeID, 0, 0, 0, 0, 0, 0, 0, 0);                         /* NULL ViewPort: INVALID_ID */
    u8 di[56];
    V(25) = CALL(gfx, LVO_graphics_GetDisplayInfoData, 56, 0x80000000, 0x29000, 0, 0, (u32)di, 0, 0);   /* 56 bytes */
    V(26) = *(u32 *)di;                                                                          /* DTAG_DISP */
    V(27) = CALL(gfx, LVO_graphics_GetDisplayInfoData, 56, 0x80000000, 0x11111, 0, 0, (u32)di, 0, 0);   /* unknown mode: 0 */
    CALL(gfx, LVO_graphics_GetDisplayInfoData, 56, 0x80002000, 0x29000, 0, 0, (u32)di, 0, 0);    /* DTAG_MNTR: refused (DIMS and NAME are modelled now) */
    /* A bitmap-backed RastPort: an outlined box, Flood inside it, ReadPixelLine8/WritePixelLine8. */
    u32 bm = CALL(gfx, LVO_graphics_AllocBitMap, 16, 16, 8, 0, 0, 0, 0, 0);
    u32 rp[25]; memset(rp, 0, sizeof rp);
    CALL(gfx, LVO_graphics_InitRastPort, 0, 0, 0, 0, 0, (u32)rp, 0, 0);
    rp[1] = bm;
    CALL(gfx, LVO_graphics_SetAPen, 2, 0, 0, 0, 0, (u32)rp, 0, 0);
    CALL(gfx, LVO_graphics_Move, 2, 2, 0, 0, 0, (u32)rp, 0, 0); CALL(gfx, LVO_graphics_Draw, 9, 2, 0, 0, 0, (u32)rp, 0, 0);
    CALL(gfx, LVO_graphics_Draw, 9, 9, 0, 0, 0, (u32)rp, 0, 0); CALL(gfx, LVO_graphics_Draw, 2, 9, 0, 0, 0, (u32)rp, 0, 0);
    CALL(gfx, LVO_graphics_Draw, 2, 2, 0, 0, 0, (u32)rp, 0, 0);
    CALL(gfx, LVO_graphics_SetAPen, 5, 0, 0, 0, 0, (u32)rp, 0, 0);
    V(28) = CALL(gfx, LVO_graphics_Flood, 5, 5, 1, 0, 0, (u32)rp, 0, 0);                         /* no TmpRas: FALSE */
    u32 tr[2]; u8 buf[32];
    CALL(gfx, LVO_graphics_InitTmpRas, 32, 0, 0, 0, (u32)tr, (u32)buf, 0, 0);
    rp[3] = (u32)tr;                                                                             /* RastPort.TmpRas at +12 */
    V(29) = CALL(gfx, LVO_graphics_Flood, 5, 5, 1, 0, 0, (u32)rp, 0, 0);                         /* colour mode: TRUE */
    V(30) = CALL(gfx, LVO_graphics_ReadPixel, 5, 5, 0, 0, 0, (u32)rp, 0, 0);                     /* inside: 5 */
    V(31) = CALL(gfx, LVO_graphics_ReadPixel, 12, 12, 0, 0, 0, (u32)rp, 0, 0);                   /* outside the box: still 0 */
    V(32) = CALL(gfx, LVO_graphics_ReadPixel, 2, 5, 0, 0, 0, (u32)rp, 0, 0);                     /* the outline itself: 2 */
    u8 line[8];
    V(33) = CALL(gfx, LVO_graphics_ReadPixelLine8, 0, 5, 8, 0, (u32)rp, 0, (u32)line, 0);        /* row 5, x 0..7: 8 pixels */
    V(34) = line[0] * 1000000 + line[2] * 10000 + line[3] * 100 + line[7];                        /* 0, 2, 5, 5 -> 0020505 */
    u8 w[3] = { 9, 9, 9 };
    CALL(gfx, LVO_graphics_WritePixelLine8, 12, 12, 3, 0, (u32)rp, 0, (u32)w, 0);
    V(35) = CALL(gfx, LVO_graphics_ReadPixel, 13, 12, 0, 0, 0, (u32)rp, 0, 0);                   /* 9 */
    /* --- small intuition calls --- */
    V(36) = CALL(intu, LVO_intuition_LockIBase, 0, 0, 0, 0, 0, 0, 0, 0) != 0;
    CALL(intu, LVO_intuition_UnlockIBase, 0, 0, 0, 0, 1, 0, 0, 0);
    CALL(intu, LVO_intuition_UnlockIBase, 0, 0, 0, 0, 1, 0, 0, 0);                               /* refused: not locked */
    u32 list = CALL(intu, LVO_intuition_LockPubScreenList, 0, 0, 0, 0, 0, 0, 0, 0);
    u32 first = rd32((void *)list);
    V(37) = strcmp(S(rd32((void *)(first + 10))), "Workbench") == 0 && rd32((void *)(first + 14)) == scr;
    CALL(intu, LVO_intuition_ScreenToFront, 0, 0, 0, 0, scr, 0, 0, 0);
    CALL(intu, LVO_intuition_ScreenToFront, 0, 0, 0, 0, 0x1234, 0, 0, 0);                         /* refused: not the screen */
    u32 wtags[] = { 0x80000064, 10, 0x80000065, 20, 0x80000066, 100, 0x80000067, 60, 0, 0 };     /* WA_Left/Top/Width/Height */
    u32 win = CALL(intu, LVO_intuition_OpenWindowTagList, 0, 0, 0, 0, 0, (u32)wtags, 0, 0);
    V(38) = CALL(intu, LVO_intuition_WindowLimits, 50, 40, 200, 150, win, 0, 0, 0);               /* TRUE */
    V(39) = *(u16 *)(win + 16) * 1000000 + *(u16 *)(win + 18) * 10000 + *(u16 *)(win + 20) * 10 + (*(u16 *)(win + 22) == 150);
    CALL(intu, LVO_intuition_WindowLimits, 0, 0, 0, 0, win, 0, 0, 0);                              /* zeros keep the limits */
    V(40) = *(u16 *)(win + 16);
    CALL(intu, LVO_intuition_ChangeWindowBox, 30, 40, 120, 80, win, 0, 0, 0);
    V(41) = *(u16 *)(win + 4) * 1000000 + *(u16 *)(win + 6) * 10000 + *(u16 *)(win + 8) * 10 + (*(u16 *)(win + 10) == 80);
    CALL(intu, LVO_intuition_ChangeWindowBox, 0, 0, 0, 0, 0x1234, 0, 0, 0);                        /* refused: not a window */
    CALL(intu, LVO_intuition_CloseWindow, 0, 0, 0, 0, win, 0, 0, 0);
  }
}
