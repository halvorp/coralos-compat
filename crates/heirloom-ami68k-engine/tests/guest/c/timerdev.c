/* timer.device's own library vectors: GetSysTime (-66) and GetUpTime (-72) read the virtual clock; a 2.5 s TR_ADDREQUEST moves
 * it. Written for this repo (MIT). */
#include "rt.h"
#define IO ((volatile u8 *)0x1F400)
#define TV ((volatile u32 *)0x1F300)
void _start(void) {
  for (int i = 0; i < 40; i++) IO[i] = 0;
  u32 err = LVO(SYSBASE, LVO_exec_OpenDevice, 0, 0, 0, 0, (u32)"timer.device", (u32)IO);   /* unit 0 */
  u32 dev = *(volatile u32 *)(IO + 20);
  RES[0] = err; RES[1] = dev != 0;
  LVO(dev, -66, 0, 0, 0, 0, (u32)TV, 0);
  RES[2] = TV[0]; RES[3] = TV[1];                                  /* 0 s 0 us at the start */
  *(volatile u16 *)(IO + 28) = 9; *(volatile u32 *)(IO + 32) = 2; *(volatile u32 *)(IO + 36) = 500000; /* TR_ADDREQUEST 2.5 s */
  LVO(SYSBASE, LVO_exec_DoIO, 0, 0, 0, 0, 0, (u32)IO);
  LVO(dev, -66, 0, 0, 0, 0, (u32)TV, 0);
  RES[4] = TV[0]; RES[5] = TV[1];                                  /* 2 s 500000 us */
  LVO(dev, -72, 0, 0, 0, 0, (u32)TV, 0);
  RES[6] = TV[0]; RES[7] = TV[1];                                  /* GetUpTime: the same clock */
  RES[8] = 1;
}
