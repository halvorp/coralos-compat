/* dos SetVBuf on the console handle, a NULL handle, a misaligned buffer and a handle nobody opened. Written for this repo (MIT). */
#include "rt.h"
#define DOS(off, d1, d2, d3, d4) LVO5(dos, off, 0, d1, d2, d3, d4, 0, 0)
void _start(void) {
  u32 dos = OPENLIB("dos.library");
  u32 out = LVO5(dos, LVO_dos_Output, 0, 0, 0, 0, 0, 0, 0);
  RES[0] = DOS(LVO_dos_SetVBuf, out, 0, 1, (u32)-1);          /* BUF_LINE on Output(): 0 */
  RES[1] = DOS(LVO_dos_SetVBuf, 0, 0, 1, (u32)-1);            /* NULL handle: EOF (-1) */
  RES[2] = DOS(LVO_dos_SetVBuf, out, 3, 1, (u32)-1);          /* buffer not longword aligned: EOF */
  RES[3] = DOS(LVO_dos_SetVBuf, 0x1234, 0, 1, (u32)-1);       /* a handle nobody opened: a named refusal (d0 = 0) */
  RES[4] = 1;
  CLOSELIB(dos);
}
