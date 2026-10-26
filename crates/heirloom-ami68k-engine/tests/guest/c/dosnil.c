/* dos.library's NIL: device and Close(0) (written for this repo, MIT). */
#include "rt.h"
#define DOS(off, d1, d2, d3) LVO5(dos, off, 0, d1, d2, d3, 0, 0, 0)
#define MODE_OLDFILE 1005
#define MODE_NEWFILE 1006
void _start(void) {
  u32 dos = OPENLIB("dos.library");
  char buf[8];
  u32 in = DOS(LVO_dos_Open, "NIL:", MODE_OLDFILE, 0);
  RES[0] = in != 0;
  RES[1] = DOS(LVO_dos_Read, in, buf, 8);                 /* EOF at once: 0 bytes */
  u32 out = DOS(LVO_dos_Open, "nil:", MODE_NEWFILE, 0);   /* the device name is case-insensitive */
  RES[2] = out != 0 && out != in;
  RES[3] = DOS(LVO_dos_Write, out, "hello", 5);           /* accepted ... */
  RES[4] = DOS(LVO_dos_Read, out, buf, 8);                /* ... and gone: still EOF */
  RES[5] = DOS(LVO_dos_Close, in, 0, 0);
  RES[6] = DOS(LVO_dos_Close, out, 0, 0);
  RES[7] = DOS(LVO_dos_Close, 0, 0, 0);                   /* Close(0) is DOSTRUE */
  RES[8] = DOS(LVO_dos_Open, "NOSUCH:x", MODE_OLDFILE, 0); /* CONTROL: a device that does not exist still fails */
  RES[9] = 1;
}
