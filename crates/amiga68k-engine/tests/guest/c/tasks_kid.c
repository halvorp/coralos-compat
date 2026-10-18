/* A tiny program LoadSeg'd and run as a process (CreateProc / NP_Seglist): marks RES[126] and signals its parent. */
#include "rt.h"
void _start(void) {
  RES[126] = 0xBEEF;
  LVO(SYSBASE, LVO_exec_Signal, RES[101], 0, 0, 0, 0, RES[100]);
}
