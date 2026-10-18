/* exec/dos multitasking, part 1: a parent CreateNewProc's a child and does Directory Opus's IPC startup handshake with
 * it (Library/ipc.c of the DOpus 5 source): PutMsg(&proc->pr_MsgPort, startup) with a reply port, WaitPort(reply); the
 * child FindTask(0)s itself, WaitPort(&pr_MsgPort), GetMsg, CreateMsgPort, ReplyMsg. Then the child Delay()s while the
 * parent sleeps, and signals it. Results in RES[]; RES[100..] is the parent/child mailbox (written for this repo, MIT). */
#include "rt.h"
#define NP(n) (0x80000000u + 1000 + (n))
#define NP_Entry NP(3)
#define NP_StackSize NP(11)
#define NP_Name NP(12)
#define NP_Priority NP(13)
#define NP_WindowPtr NP(15)
#define NP_Cli NP(18)
#define NP_Arguments NP(21)
#define S(p) ((const char *)(p))
#define EXEC(off, d0, a0, a1) LVO(SYSBASE, off, d0, 0, 0, 0, a0, a1)
#define DOS(off, d1) LVO5(dos, off, 0, d1, 0, 0, 0, 0, 0)
#define IPC_STARTUP 1
void child(void);
void arg_child(void);
void arg_stub(void);
void _start(void) {
  u32 dos = OPENLIB("dos.library");
  u32 me = EXEC(LVO_exec_FindTask, 0, 0, 0);
  u32 port = EXEC(LVO_exec_CreateMsgPort, 0, 0, 0);
  u32 bit = EXEC(LVO_exec_AllocSignal, -1, 0, 0);
  RES[0] = port != 0;
  RES[100] = me; RES[101] = 1u << bit;
  u32 tags[] = { NP_Entry, (u32)child, NP_Name, (u32)"child", NP_StackSize, 8192, NP_Priority, 0, NP_WindowPtr, (u32)-1, 0, 0 };
  u32 proc = DOS(LVO_dos_CreateNewProc, tags);
  RES[1] = proc != 0;
  RES[2] = rd32((void *)(proc + 132));                     /* pr_StackSize */
  RES[3] = strcmp(S(rd32((void *)(proc + 10))), "child") == 0;
  RES[4] = EXEC(LVO_exec_FindTask, 0, 0, (u32)"child") == proc;
  RES[5] = *(u8 *)(proc + 92 + 15);                        /* pr_MsgPort.mp_SigBit = SIGB_DOS */
  RES[6] = rd32((void *)(proc + 92 + 16)) == proc;         /* mp_SigTask */
  RES[7] = *(u8 *)(proc + 8);                              /* NT_PROCESS */
  RES[8] = rd32((void *)(proc + 184));                     /* pr_WindowPtr = -1 */
  u32 msg[8];
  memset(msg, 0, sizeof msg);
  *(u32 *)((u8 *)msg + 14) = port;                         /* mn_ReplyPort */
  msg[5] = IPC_STARTUP; msg[7] = 0xD00D;
  EXEC(LVO_exec_PutMsg, 0, proc + 92, (u32)msg);
  RES[9] = RES[110] == 0;                                  /* the child has not run yet: same priority, the parent still runs */
  EXEC(LVO_exec_WaitPort, 0, port, 0);
  EXEC(LVO_exec_GetMsg, 0, port, 0);
  RES[10] = msg[5];                                        /* command unchanged = the child reports success */
  RES[11] = msg[7];                                        /* the child's answer */
  RES[12] = RES[110] == proc;                              /* the child's own FindTask(0) */
  RES[13] = rd32((void *)(proc + 88));                     /* tc_UserData set by the child */
  RES[14] = RES[111];                                      /* the child's first CreateMsgPort signal */
  RES[15] = bit;                                           /* the parent's: 17 (16 is the reply port) */
  EXEC(LVO_exec_Wait, 1u << bit, 0, 0);                    /* the child Delay()s 3 ticks, then signals */
  RES[16] = RES[112];
  RES[17] = EXEC(LVO_exec_FindTask, 0, 0, (u32)"child") == 0;   /* it returned: reaped */
  /* A second process: NP_Cli + NP_Arguments; A0/D0 carry the argument line. */
  u32 t2[] = { NP_Entry, (u32)arg_stub, NP_Name, (u32)"argchild", NP_Cli, 1, NP_Arguments, (u32)"hello\n", 0, 0 };
  RES[18] = DOS(LVO_dos_CreateNewProc, t2) != 0;
  /* Negative controls: all refused by name or failing with an error code. */
  EXEC(LVO_exec_Wait, 1u << 25, 0, 0);                     /* a signal this task never allocated */
  EXEC(LVO_exec_RemTask, 0, 0, 0x00DEAD00);                /* not a task */
  EXEC(LVO_exec_Signal, 1, 0, 0x00DEAD00);                 /* not a task */
  u32 noent[] = { NP_Name, (u32)"nothing to run", 0, 0 };
  RES[19] = DOS(LVO_dos_CreateNewProc, noent);             /* neither NP_Entry nor NP_Seglist: NULL */
  RES[20] = DOS(LVO_dos_IoErr, 0);                         /* ERROR_REQUIRED_ARG_MISSING 116 */
  /* Let argchild run to its end (equal priority: when this task sleeps). */
  EXEC(LVO_exec_Wait, 1u << bit, 0, 0);
  RES[21] = RES[120]; RES[22] = RES[121]; RES[23] = RES[122]; RES[24] = RES[123];
  CLOSELIB(dos);
}

void child(void) {
  u32 me = EXEC(LVO_exec_FindTask, 0, 0, 0);
  RES[110] = me;
  EXEC(LVO_exec_WaitPort, 0, me + 92, 0);
  u32 msg = EXEC(LVO_exec_GetMsg, 0, me + 92, 0);
  *(u32 *)(me + 88) = 0x1234;                              /* tc_UserData, as IPC_ProcStartup stores its IPCData */
  u32 cp = EXEC(LVO_exec_CreateMsgPort, 0, 0, 0);
  RES[111] = *(u8 *)(cp + 15);
  *(u32 *)(msg + 28) = 0xC0DE;
  EXEC(LVO_exec_ReplyMsg, 0, 0, msg);
  u32 dos = OPENLIB("dos.library");
  DOS(LVO_dos_Delay, 3);
  RES[112] = 1;
  EXEC(LVO_exec_Signal, RES[101], 0, RES[100]);
  CLOSELIB(dos);
}

/* Entered with A0 = the argument string, D0 = its length (CreateNewProc, NP_Arguments): a tiny assembler stub saves both
 * (C cannot see registers at its entry), then runs the C part. */
asm(".section .text.zz,\"ax\"\n.align 2\n.globl arg_stub\narg_stub:\n\tmove.l %d0,0x1F1E0\n\tmove.l %a0,0x1F1E4\n\tjmp arg_child(%pc)\n");
void arg_child(void) {
  u32 a0 = RES[121];
  RES[121] = a0 != 0 && strcmp(S(a0), "hello\n") == 0;
  u32 me = EXEC(LVO_exec_FindTask, 0, 0, 0);
  RES[122] = rd32((void *)(me + 172)) != 0;                /* pr_CLI: NP_Cli gave it one */
  RES[123] = rd32((void *)(me + 204)) == a0;               /* pr_Arguments is the same string */
  EXEC(LVO_exec_Signal, RES[101], 0, RES[100]);
}
