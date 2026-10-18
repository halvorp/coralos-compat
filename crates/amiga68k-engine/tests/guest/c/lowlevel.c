/* lowlevel.library: ReadJoyPort / GetLanguageSelection / GetKey / QueryKeys (written for this repo, MIT). */
#include "rt.h"
void _start(void) {
  u32 ll = OPENLIB("lowlevel.library");
  RES[0] = ll;
  RES[1] = LVO(ll, -30, 1, 0, 0, 0, 0, 0);          /* ReadJoyPort(1) */
  RES[2] = LVO(ll, -36, 0, 0, 0, 0, 0, 0);          /* GetLanguageSelection */
  RES[3] = LVO(ll, -48, 0, 0, 0, 0, 0, 0);          /* GetKey */
  u16 q[8] = {0x45, 0xFFFF, 0x46, 0xFFFF, 0x47, 0xFFFF, 0x48, 0xFFFF};  /* four KeyQuery { code, pressed } */
  LVO(ll, -54, 0, 3, 0, 0, q, 0);                   /* QueryKeys(q, 3): the fourth stays untouched */
  for (int i = 0; i < 8; i++) RES[4 + i] = q[i];
  RES[12] = LVO(ll, -60, 0, 0, 0, 0, 0, 0);         /* AddKBInt: known, not bridged -> a named refusal */
  CLOSELIB(ll);
}
