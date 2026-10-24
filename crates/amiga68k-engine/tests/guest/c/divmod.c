/* utility.library SDivMod32 (-150) / UDivMod32 (-156): D0 = quotient, D1 = remainder (utility_lib.fd). Written for this repo (MIT).
 * RES[0..1] 100 / 7 signed, RES[2..3] -100 / 7, RES[4..5] 100 / -7, RES[6..7] 0xFFFFFFF0 / 16 unsigned, RES[8..9] 0xFFFFFFFF / 10 unsigned. */
#include "rt.h"
#define DIVMOD(base, off, n, d, qi) ({ \
  register u32 _d0 asm("d0") = (u32)(n); register u32 _d1 asm("d1") = (u32)(d); \
  asm volatile("move.l %%a6,-(%%sp)\n\tmove.l %[b],%%a6\n\tjsr %c[o](%%a6)\n\tmove.l (%%sp)+,%%a6" : "+r"(_d0), "+r"(_d1) : [b] "r"((u32)(base)), [o] "i"(off) : "cc", "memory", "a0", "a1"); \
  RES[qi] = _d0; RES[(qi) + 1] = _d1; })
void _start(void) {
  u32 u = OPENLIB("utility.library");
  DIVMOD(u, LVO_utility_SDivMod32, 100, 7, 0);
  DIVMOD(u, LVO_utility_SDivMod32, -100, 7, 2);
  DIVMOD(u, LVO_utility_SDivMod32, 100, -7, 4);
  DIVMOD(u, LVO_utility_UDivMod32, 0xFFFFFFF0u, 16, 6);
  DIVMOD(u, LVO_utility_UDivMod32, 0xFFFFFFFFu, 10, 8);
  RES[10] = 1;
  DIVMOD(u, LVO_utility_SDivMod32, 5, 0, 12);   /* division by zero: refused by name, the registers keep the inputs */
  RES[11] = 1;
}
