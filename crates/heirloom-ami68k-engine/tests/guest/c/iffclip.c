/* A clipboard stream (InitIFFasClip) and a stream hook of the program's own are named refusals at OpenIFF (written for this repo, MIT). */
#include "rt.h"
#define V(i) (*(volatile u32 *)&RES[i])
void _start(void) {
  u32 ib = OPENLIB("iffparse.library");
  u32 iff = LVO(ib, LVO_iffparse_AllocIFF, 0, 0, 0, 0, 0, 0);
  LVO(ib, LVO_iffparse_InitIFFasClip, 0, 0, 0, 0, iff, 0);
  V(0) = LVO(ib, LVO_iffparse_OpenIFF, 0, 0, 0, 0, iff, 0);          /* IFFERR_NOHOOK, refused by name */
  V(1) = *(volatile u32 *)(iff + 4);                                 /* iff_Flags: IFFF_RSEEK only, not open */
  u32 hook[5] = { 0, 0, 0x1234, 0, 0 };
  LVO(ib, LVO_iffparse_InitIFF, 4, 0, 0, 0, iff, (u32)hook);
  V(2) = LVO(ib, LVO_iffparse_OpenIFF, 1, 0, 0, 0, iff, 0);
  LVO(ib, LVO_iffparse_FreeIFF, 0, 0, 0, 0, iff, 0);
  V(3) = LVO(ib, LVO_iffparse_OpenIFF, 0, 0, 0, 0, 0, 0);           /* NULL: IFFERR_NOMEM */
  CLOSELIB(ib);
}
