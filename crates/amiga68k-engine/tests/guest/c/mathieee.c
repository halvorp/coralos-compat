/* mathieeesingbas / mathieeedoubbas arithmetic (written for this repo, MIT). Doubles travel in D0:D1 and D2:D3. */
#include "rt.h"
#define V(i) (*(volatile u32 *)&RES[i])
/* A double-precision call: returns the high word, stores the low word at *lo. */
#define DCALL(base, off, hi0, lo0, hi1, lo1, lo) ({ \
  register u32 _d0 asm("d0") = (u32)(hi0); register u32 _d1 asm("d1") = (u32)(lo0); register u32 _d2 asm("d2") = (u32)(hi1); register u32 _d3 asm("d3") = (u32)(lo1); \
  asm volatile("move.l %%a6,-(%%sp)\n\tmove.l %[b],%%a6\n\tjsr %c[o](%%a6)\n\tmove.l (%%sp)+,%%a6" : "+r"(_d0), "+r"(_d1) : "r"(_d2), "r"(_d3), [b] "r"((u32)(base)), [o] "i"(off) : "cc", "memory", "a0", "a1"); \
  *(lo) = _d1; _d0; })
#define SCALL(base, off, X, Y) ({ register u32 _d0 asm("d0") = (u32)(X); register u32 _d1 asm("d1") = (u32)(Y); \
  asm volatile("move.l %%a6,-(%%sp)\n\tmove.l %[b],%%a6\n\tjsr %c[o](%%a6)\n\tmove.l (%%sp)+,%%a6" : "+r"(_d0), "+r"(_d1) : [b] "r"((u32)(base)), [o] "i"(off) : "cc", "memory", "a0", "a1"); _d0; })
void _start(void) {
  u32 sb = OPENLIB("mathieeesingbas.library"), db = OPENLIB("mathieeedoubbas.library");
  V(0) = sb != 0 && db != 0;
  u32 lo;
  /* singles: 1.5 = 3FC00000, 2.25 = 40100000 */
  V(1) = SCALL(sb, LVO_mathieeesingbas_IEEESPAdd, 0x3FC00000, 0x40100000);              /* 3.75 = 40700000 */
  V(2) = SCALL(sb, LVO_mathieeesingbas_IEEESPSub, 0x3FC00000, 0x40100000);              /* -0.75 = BF400000 */
  V(3) = SCALL(sb, LVO_mathieeesingbas_IEEESPFix, 0xC0300000, 0);                        /* -2.75 -> -2 */
  V(4) = SCALL(sb, LVO_mathieeesingbas_IEEESPCmp, 0x3FC00000, 0x40100000);              /* -1 */
  V(5) = SCALL(sb, LVO_mathieeesingbas_IEEESPCmp, 0x40100000, 0x40100000);              /* 0: the negative control */
  V(6) = SCALL(sb, LVO_mathieeesingbas_IEEESPFlt, 7, 0);                                  /* 7.0 = 40E00000 */
  /* doubles: 1.5 = 3FF8.., 2.25 = 4002.., -2.75 = C006.. */
  V(7) = DCALL(db, LVO_mathieeedoubbas_IEEEDPAdd, 0x3FF80000, 0, 0x40020000, 0, &lo);  V(8) = lo;      /* 3.75 = 400E0000 00000000 */
  V(9) = DCALL(db, LVO_mathieeedoubbas_IEEEDPSub, 0x3FF80000, 0, 0x40020000, 0, &lo);                    /* -0.75 = BFE80000 */
  V(10) = DCALL(db, LVO_mathieeedoubbas_IEEEDPDiv, 0x40020000, 0, 0x3FF80000, 0, &lo);                   /* 2.25 / 1.5 = 1.5 = 3FF80000 */
  V(11) = DCALL(db, LVO_mathieeedoubbas_IEEEDPFix, 0xC0060000, 0, 0, 0, &lo);                            /* -2 */
  V(12) = DCALL(db, LVO_mathieeedoubbas_IEEEDPFloor, 0xC0060000, 0, 0, 0, &lo);                          /* -3.0 = C0080000 */
  V(13) = DCALL(db, LVO_mathieeedoubbas_IEEEDPCeil, 0xC0060000, 0, 0, 0, &lo);                           /* -2.0 = C0000000 */
  V(14) = DCALL(db, LVO_mathieeedoubbas_IEEEDPCmp, 0x3FF80000, 0, 0x40020000, 0, &lo);                   /* -1 */
  V(15) = DCALL(db, LVO_mathieeedoubbas_IEEEDPTst, 0xC0060000, 0, 0, 0, &lo);                            /* -1 */
  V(16) = DCALL(db, LVO_mathieeedoubbas_IEEEDPFlt, 7, 0, 0, 0, &lo);                                       /* 7.0 = 401C0000 */
  V(17) = DCALL(db, LVO_mathieeedoubbas_IEEEDPDiv, 0x3FF80000, 0, 0, 0, &lo); V(18) = lo;                /* x / 0 = +inf = 7FF00000 00000000 */
  V(19) = DCALL(db, LVO_mathieeedoubbas_IEEEDPNeg, 0x3FF80000, 0, 0, 0, &lo);                            /* -1.5 = BFF80000 */
  V(20) = DCALL(db, LVO_mathieeedoubbas_IEEEDPAbs, 0xC0060000, 0, 0, 0, &lo);                            /* 2.75 = 40060000 */
}
