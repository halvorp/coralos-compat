/* A BitMap built with InitBitMap over the program's own planes is a blit source read from those planes (a DiskObject's icon
 * Image, as Directory Opus draws it). Written for this repo (MIT). */
#include "rt.h"
#define V(i) (*(volatile u32 *)&RES[i])
#define GFX(off, d0, d1, d2, d3, d4, d5, d6, d7, a0, a1, a2) ({ \
  register u32 _d0 asm("d0") = (u32)(d0); register u32 _d1 asm("d1") = (u32)(d1); register u32 _d2 asm("d2") = (u32)(d2); register u32 _d3 asm("d3") = (u32)(d3); \
  register u32 _d4 asm("d4") = (u32)(d4); register u32 _d5 asm("d5") = (u32)(d5); register u32 _d6 asm("d6") = (u32)(d6); register u32 _d7 asm("d7") = (u32)(d7); \
  register u32 _a0 asm("a0") = (u32)(a0); register u32 _a1 asm("a1") = (u32)(a1); register u32 _a2 asm("a2") = (u32)(a2); \
  asm volatile("move.l %%a6,-(%%sp)\n\tmove.l %[b],%%a6\n\tjsr %c[o](%%a6)\n\tmove.l (%%sp)+,%%a6" \
    : "+r"(_d0), "+r"(_d1), "+r"(_a0), "+r"(_a1) : "r"(_d2), "r"(_d3), "r"(_d4), "r"(_d5), "r"(_d6), "r"(_d7), "r"(_a2), [b] "r"(gfx), [o] "i"(off) : "cc", "memory"); _d0; })
void _start(void) {
  u32 gfx = OPENLIB("graphics.library");
  static u8 p0[4] = { 0xF0, 0x0F, 0xAA, 0x00 }, p1[4] = { 0xFF, 0x00, 0x00, 0xFF };   /* 2 planes, 16 bits x 2 rows (BytesPerRow 2) */
  u32 src[10]; memset(src, 0, sizeof src);
  GFX(LVO_graphics_InitBitMap, 2, 16, 2, 0, 0, 0, 0, 0, (u32)src, 0, 0);                 /* depth 2, 16x2 */
  src[2] = (u32)p0; src[3] = (u32)p1;                                                   /* Planes[0], Planes[1] at +8, +12 */
  u32 dst = GFX(LVO_graphics_AllocBitMap, 16, 2, 8, 0, 0, 0, 0, 0, 0, 0, 0);
  u32 rp[25]; memset(rp, 0, sizeof rp);
  GFX(LVO_graphics_InitRastPort, 0, 0, 0, 0, 0, 0, 0, 0, 0, (u32)rp, 0); rp[1] = dst;
  GFX(LVO_graphics_BltBitMap, 0, 0, 0, 0, 16, 2, 0xC0, 0xFF, (u32)src, dst, 0);        /* copy */
  for (int i = 0; i < 8; i++) V(i) = GFX(LVO_graphics_ReadPixel, i, 0, 0, 0, 0, 0, 0, 0, 0, (u32)rp, 0);   /* row 0, x 0..7 */
  V(8) = GFX(LVO_graphics_ReadPixel, 0, 1, 0, 0, 0, 0, 0, 0, 0, (u32)rp, 0);            /* row 1, x 0 */
  /* The program changes its plane memory: the next blit sees the new data (the host copy is not stale). */
  p0[0] = 0x00; p1[0] = 0x00;
  GFX(LVO_graphics_BltBitMap, 0, 0, 0, 0, 16, 2, 0xC0, 0xFF, (u32)src, dst, 0);
  V(9) = GFX(LVO_graphics_ReadPixel, 0, 0, 0, 0, 0, 0, 0, 0, 0, (u32)rp, 0);
  /* A NULL plane reads as zeros: with Planes[1] = NULL only bit 0 survives. */
  src[3] = 0; p0[0] = 0xF0;
  GFX(LVO_graphics_BltBitMap, 0, 0, 0, 0, 16, 2, 0xC0, 0xFF, (u32)src, dst, 0);
  V(10) = GFX(LVO_graphics_ReadPixel, 0, 0, 0, 0, 0, 0, 0, 0, 0, (u32)rp, 0);
  /* What the engine draws into a planar bitmap is written back to the program's planes (DOpus's image remap writes with
   * WritePixelLine8 and then draws from the planes): pens 3,1,2,0,3,3,0,1 -> plane 0 = 11001101 (CD), plane 1 = 10101100 (AC). */
  static u8 q0[4] = { 0, 0x55, 0, 0 }, q1[4] = { 0, 0x77, 0, 0 };
  static u8 pens[8] = { 3, 1, 2, 0, 3, 3, 0, 1 }, back[8];
  src[2] = (u32)q0; src[3] = (u32)q1;
  u32 srp[25]; memset(srp, 0, sizeof srp);
  GFX(LVO_graphics_InitRastPort, 0, 0, 0, 0, 0, 0, 0, 0, 0, (u32)srp, 0); srp[1] = (u32)src;
  GFX(LVO_graphics_WritePixelLine8, 0, 0, 8, 0, 0, 0, 0, 0, (u32)srp, 0, (u32)pens);
  V(11) = q0[0]; V(12) = q1[0];
  V(13) = q0[1];                                                                         /* control: row 0's second byte and row 1 untouched */
  /* Reading a line sees plane bytes the program wrote itself: byte 1 of row 0 is 0x55 / 0x77. */
  GFX(LVO_graphics_ReadPixelLine8, 8, 0, 8, 0, 0, 0, 0, 0, (u32)srp, 0, (u32)back);
  V(14) = back[0]; V(15) = back[1];                                                     
}
