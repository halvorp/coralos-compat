/* exec OpenResource of a name nobody added (AmigaOS probes FileSystem.resource): NULL, not a refusal. Written for this repo (MIT). */
#include "rt.h"
void _start(void) {
  /* d0 starts as garbage so a handler that forgets to set the result is caught. */
  RES[0] = LVO(SYSBASE, LVO_exec_OpenResource, 0xDEADBEEF, 0, 0, 0, 0, (u32)"FileSystem.resource");
  RES[1] = 1; /* reached the end */
}
