/* dos process creation: CreateProc and CreateNewProc(NP_Seglist) over a LoadSeg'd program, NP_ExitCode/NP_ExitData and
 * dos Exit(), NP_Output with NP_CloseOutput, a duplicated current directory. Written for this repo (MIT). */
#include "rt.h"
#define NP(n) (0x80000000u + 1000 + (n))
#define NP_Seglist NP(1)
#define NP_Entry NP(3)
#define NP_Output NP(5)
#define NP_Name NP(12)
#define NP_ExitCode NP(24)
#define NP_ExitData NP(25)
#define S(p) ((const char *)(p))
#define V(i) (*(volatile u32 *)&RES[i])
#define EXEC(off, d0, a0, a1) LVO(SYSBASE, off, d0, 0, 0, 0, a0, a1)
#define DOS1(off, d1, d2, d3) LVO5(dos, off, 0, d1, d2, d3, 0, 0, 0)
u32 rc_child(void); void sleeper(void); void timer_child(void); void exit_child(void); void writer(void); void exit_stub(void);
asm(".section .text.zz,\"ax\"\n.align 2\n.globl exit_stub\nexit_stub:\n\tmove.l %d0,0x1F1F0\n\tmove.l %d1,0x1F1F4\n\trts\n");   /* RES[124], RES[125] */

void _start(void) {
  u32 dos = OPENLIB("dos.library");
  u32 me = EXEC(LVO_exec_FindTask, 0, 0, 0);
  u32 bit = EXEC(LVO_exec_AllocSignal, -1, 0, 0);
  V(100) = me; V(101) = 1u << bit;
  /* CreateProc over a LoadSeg'd program. */
  u32 seg = DOS1(LVO_dos_LoadSeg, (u32)"SYS:kid", 0, 0);
  V(0) = seg != 0;
  u32 port = LVO5(dos, LVO_dos_CreateProc, 0, (u32)"kid", 0, seg, 4096, 0, 0);
  V(1) = port != 0;
  V(2) = strcmp(S(rd32((void *)(port - 92 + 10))), "kid") == 0;
  V(3) = EXEC(LVO_exec_FindTask, 0, 0, (u32)"kid") == port - 92;
  EXEC(LVO_exec_Wait, V(101), 0, 0);
  V(4) = V(126) == 0xBEEF;
  V(5) = DOS1(LVO_dos_UnLoadSeg, seg, 0, 0);               /* CreateProc kept the seglist (AROS createproc.c): still ours */
  V(6) = EXEC(LVO_exec_FindTask, 0, 0, (u32)"kid") == 0;    /* the kid has ended */
  /* NP_Seglist: freed at exit by default (NP_FreeSeglist TRUE). */
  u32 seg2 = DOS1(LVO_dos_LoadSeg, (u32)"SYS:kid", 0, 0);
  V(126) = 0;
  u32 t1[] = { NP_Seglist, seg2, NP_Name, (u32)"kid2", 0, 0 };
  u32 p2 = LVO5(dos, LVO_dos_CreateNewProc, 0, (u32)t1, 0, 0, 0, 0, 0);
  V(7) = p2 != 0 && rd32((void *)(rd32((void *)(p2 + 128)) * 4 + 12)) == seg2;   /* pr_SegList -> [4, -1, -2, seglist] */
  EXEC(LVO_exec_Wait, V(101), 0, 0);
  V(8) = V(126) == 0xBEEF;
  /* NP_ExitCode gets the return code in D0 and NP_ExitData in D1. */
  u32 t2[] = { NP_Entry, (u32)rc_child, NP_Name, (u32)"rc", NP_ExitCode, (u32)exit_stub, NP_ExitData, 0xFEED, 0, 0 };
  LVO5(dos, LVO_dos_CreateNewProc, 0, (u32)t2, 0, 0, 0, 0, 0);
  EXEC(LVO_exec_Wait, V(101), 0, 0);
  V(9) = V(124); V(10) = V(125);                           /* 42, 0xFEED */
  /* dos Exit(7) ends the process on the spot; the exit code still runs. */
  V(124) = 0;
  u32 t3[] = { NP_Entry, (u32)exit_child, NP_Name, (u32)"ex", NP_ExitCode, (u32)exit_stub, NP_ExitData, 0xFACE, 0, 0 };
  LVO5(dos, LVO_dos_CreateNewProc, 0, (u32)t3, 0, 0, 0, 0, 0);
  EXEC(LVO_exec_Wait, V(101), 0, 0);
  V(11) = V(124); V(12) = V(125); V(13) = V(127);          /* 7, 0xFACE, 0: the code after Exit() never ran */
  /* NP_Output: the child writes to it and, with NP_CloseOutput (default TRUE), the process closes it at the end. */
  u32 out = DOS1(LVO_dos_Open, (u32)"RAM:out", 1006, 0);
  u32 lock = DOS1(LVO_dos_Lock, (u32)"RAM:", (u32)-2, 0);
  u32 old = DOS1(LVO_dos_CurrentDir, lock, 0, 0);
  V(122) = 0;
  u32 t4[] = { NP_Entry, (u32)writer, NP_Name, (u32)"w", NP_Output, out, 0, 0 };
  LVO5(dos, LVO_dos_CreateNewProc, 0, (u32)t4, 0, 0, 0, 0, 0);
  EXEC(LVO_exec_Wait, V(101), 0, 0);
  DOS1(LVO_dos_CurrentDir, old, 0, 0);
  V(14) = V(122) != 0 && V(122) != lock;                   /* the child's current directory is a duplicate lock */
  DOS1(LVO_dos_UnLock, lock, 0, 0);
  u32 in = DOS1(LVO_dos_Open, (u32)"RAM:out", 1005, 0);
  char buf[4] = { 0, 0, 0, 0 };
  V(15) = DOS1(LVO_dos_Read, in, (u32)buf, 4);
  V(16) = strcmp(buf, "hi") == 0;
  DOS1(LVO_dos_Close, in, 0, 0);
  /* A process in DoIO(timer.device) sleeps on the virtual clock while the parent waits. */
  u32 t8[] = { NP_Entry, (u32)timer_child, NP_Name, (u32)"timer", 0, 0 };
  LVO5(dos, LVO_dos_CreateNewProc, 0, (u32)t8, 0, 0, 0, 0, 0);
  EXEC(LVO_exec_Wait, V(101), 0, 0);
  V(21) = V(102);                                                       /* DoIO's io_Error: 0 */
  V(22) = V(103);                                                       /* it did wake */
  /* A process killed with RemTask gives everything back: the heap is exactly as before. */
  u32 before = LVO(SYSBASE, LVO_exec_AvailMem, 0, 0, 0, 0, 0, 0);
  u32 t5[] = { NP_Entry, (u32)sleeper, NP_Name, (u32)"sleeper", 0, 0 };
  u32 sp = LVO5(dos, LVO_dos_CreateNewProc, 0, (u32)t5, 0, 0, 0, 0, 0);
  EXEC(LVO_exec_Wait, V(101), 0, 0);                       /* it ran up to its Wait */
  EXEC(LVO_exec_RemTask, 0, 0, sp);
  V(17) = LVO(SYSBASE, LVO_exec_AvailMem, 0, 0, 0, 0, 0, 0) == before;
  V(18) = EXEC(LVO_exec_FindTask, 0, 0, (u32)"sleeper") == 0 && V(127) == 0;
  /* Negative controls: a seglist LoadSeg never returned, and a parent that wants to wait for its child. */
  u32 t6[] = { NP_Seglist, 0x1234, NP_Name, (u32)"bogus", 0, 0 };
  V(19) = LVO5(dos, LVO_dos_CreateNewProc, 0, (u32)t6, 0, 0, 0, 0, 0);
  u32 t7[] = { NP_Entry, (u32)sleeper, NP_Name, (u32)"sync", NP(23), 1, 0, 0 };
  V(20) = LVO5(dos, LVO_dos_CreateNewProc, 0, (u32)t7, 0, 0, 0, 0, 0);
  CLOSELIB(dos);
}

u32 rc_child(void) { EXEC(LVO_exec_Signal, V(101), 0, V(100)); return 42; }
void sleeper(void) { u32 b = EXEC(LVO_exec_AllocSignal, -1, 0, 0); EXEC(LVO_exec_Signal, V(101), 0, V(100)); EXEC(LVO_exec_Wait, 1u << b, 0, 0); V(127) = 2; }
void exit_child(void) {
  u32 dos = OPENLIB("dos.library");
  EXEC(LVO_exec_Signal, V(101), 0, V(100));
  LVO5(dos, LVO_dos_Exit, 0, 7, 0, 0, 0, 0, 0);
  V(127) = 1;
}
void writer(void) {
  u32 dos = OPENLIB("dos.library");
  u32 me = EXEC(LVO_exec_FindTask, 0, 0, 0);
  V(122) = rd32((void *)(me + 152));
  u32 o = LVO5(dos, LVO_dos_Output, 0, 0, 0, 0, 0, 0, 0);
  LVO5(dos, LVO_dos_Write, 0, o, (u32)"hi", 2, 0, 0, 0);
  EXEC(LVO_exec_Signal, V(101), 0, V(100));
}
void timer_child(void) {
  u32 port = EXEC(LVO_exec_CreateMsgPort, 0, 0, 0);
  u32 io = LVO(SYSBASE, LVO_exec_CreateIORequest, 40, 0, 0, 0, port, 0);
  V(102) = 99;
  if (LVO(SYSBASE, LVO_exec_OpenDevice, 1, 0, 0, 0, (u32)"timer.device", io) != 0) { V(103) = 0; return; }
  *(u16 *)(io + 28) = 9;                                                /* TR_ADDREQUEST */
  *(u32 *)(io + 32) = 2; *(u32 *)(io + 36) = 0;                         /* 2 s */
  V(102) = LVO(SYSBASE, LVO_exec_DoIO, 0, 0, 0, 0, 0, io);
  V(103) = 1;
  EXEC(LVO_exec_Signal, V(101), 0, V(100));
}
