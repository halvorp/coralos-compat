/* icon.library against real .info bytes on the volume (written for this repo, MIT). */
#include "rt.h"
#define U8(a) (*(volatile u8 *)(a))
#define U16(a) (*(volatile u16 *)(a))
#define SEQ(p, s) ((p) ? (u32)(strcmp((const char *)(p), (s)) == 0) : 99)
void _start(void) {
  u32 ic = OPENLIB("icon.library"), dos = OPENLIB("dos.library");
  u32 d = LVO(ic, -78, 0, 0, 0, 0, "SYS:Tool", 0);                 /* GetDiskObject */
  RES[0] = d;
  RES[1] = U8(d + 48);                                              /* do_Type */
  RES[2] = SEQ(rd32((void *)(d + 50)), "SYS:C/More");               /* do_DefaultTool */
  u32 tt = rd32((void *)(d + 54));
  RES[3] = SEQ(LVO(ic, -96, 0, 0, 0, 0, tt, "ICON"), "Foo");        /* FindToolType */
  RES[4] = SEQ(LVO(ic, -96, 0, 0, 0, 0, tt, "keys"), "A|b");        /* case-insensitive key */
  RES[5] = LVO(ic, -96, 0, 0, 0, 0, tt, "OFF");                     /* "(OFF=1)" is disabled: NULL */
  RES[6] = SEQ(LVO(ic, -96, 0, 0, 0, 0, tt, "FLAG"), "");           /* bare "FLAG": empty value, not NULL */
  RES[7] = LVO(ic, -102, 0, 0, 0, 0, "A|b", "B");                   /* MatchToolValue yes */
  RES[8] = LVO(ic, -102, 0, 0, 0, 0, "A|b", "C");                   /* no */
  RES[9] = LVO(ic, -102, 0, 0, 0, 0, "A|b", "A|b");                 /* a value with | never matches */
  u32 gr = rd32((void *)(d + 22));                                  /* do_Gadget.GadgetRender */
  RES[10] = U16(gr + 4) | (U16(gr + 6) << 16);                      /* Image width, height */
  RES[11] = rd32((void *)(gr + 10)) != 0;                           /* ImageData points at real data */
  RES[12] = U8(rd32((void *)(gr + 10)));                            /* its first byte (planar data) */
  u32 d2 = LVO(ic, -78, 0, 0, 0, 0, "SYS:Nope", 0);
  RES[13] = d2;
  RES[14] = LVO(dos, -132, 0, 0, 0, 0, 0, 0);                       /* IoErr: 205 */
  RES[15] = LVO(ic, -84, 0, 0, 0, 0, "RAM:Out", d);                 /* PutDiskObject */
  LVO(ic, -84, 0, 0, 0, 0, "RAM:Gone", d);
  RES[16] = LVO(ic, -84, 0, 0, 0, 0, "SYS:Out", d);                 /* SYS: is write protected: FALSE */
  RES[17] = LVO(dos, -132, 0, 0, 0, 0, 0, 0);                       /* IoErr: 214 */
  u32 dup = LVO(ic, -150, 0, 0, 0, 0, d, 0);                        /* DupDiskObjectA */
  RES[18] = dup != 0 && dup != d && rd32((void *)(dup + 54)) != tt;
  LVO(ic, -90, 0, 0, 0, 0, d, 0);                                   /* FreeDiskObject x2 */
  LVO(ic, -90, 0, 0, 0, 0, dup, 0);
  u32 n = LVO(ic, -132, 0, 0, 0, 0, "SYS:Dir", 0);                  /* GetDiskObjectNew: drawer without .info */
  RES[19] = n ? U8(n + 48) : 99;
  RES[20] = rd32((void *)(n + 66)) != 0;                            /* drawers carry DrawerData */
  RES[21] = U16(rd32((void *)(n + 66)) + 4);                        /* its window width */
  LVO(ic, -90, 0, 0, 0, 0, n, 0);
  u32 p = LVO(ic, -132, 0, 0, 0, 0, "SYS:Prog", 0);                 /* an executable: WBTOOL */
  RES[22] = p ? U8(p + 48) : 99;
  LVO(ic, -90, 0, 0, 0, 0, p, 0);
  RES[23] = LVO(ic, -132, 0, 0, 0, 0, "SYS:Missing", 0);            /* nothing there at all: NULL */
  u32 g = LVO(ic, -120, 3, 0, 0, 0, 0, 0);                          /* GetDefDiskObject(WBTOOL) */
  RES[24] = g ? U8(g + 48) : 99;
  LVO(ic, -90, 0, 0, 0, 0, g, 0);
  RES[25] = LVO(ic, -120, 99, 0, 0, 0, 0, 0);                       /* no such type: NULL */
  RES[26] = LVO(ic, -138, 0, 0, 0, 0, "RAM:Gone", 0);              /* DeleteDiskObject */
  RES[27] = LVO(ic, -138, 0, 0, 0, 0, "RAM:Gone", 0);              /* already gone: FALSE */
  CLOSELIB(ic);
}
