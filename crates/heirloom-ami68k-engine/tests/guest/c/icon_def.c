/* icon.library default icons (written for this repo, MIT): the guest keeps three DiskObjects alive for the host test. */
#include "rt.h"
void _start(void) {
  u32 ic = OPENLIB("icon.library");
  RES[0] = LVO(ic, LVO_icon_GetDefDiskObject, 2, 0, 0, 0, 0, 0);                    /* WBDRAWER default */
  RES[1] = LVO(ic, LVO_icon_GetDiskObjectNew, 0, 0, 0, 0, "SYS:Dir", 0);             /* a drawer with no .info */
  RES[2] = LVO(ic, LVO_icon_GetDiskObject, 0, 0, 0, 0, "SYS:Tool", 0);               /* a real .info */
}
