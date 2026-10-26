/* The small dos.library services: Fault/PrintFault, StrToLong, SplitName, dates, handles, streams, signals (written for this repo, MIT). */
#include "rt.h"
#define DOS(off, d1, d2, d3, d4) LVO5(dos, off, 0, d1, d2, d3, d4, 0, 0)
#define IOERR() DOS(LVO_dos_IoErr, 0, 0, 0, 0)
#define NEG1 ((u32)-1)
void _start(void) {
  u32 dos = OPENLIB("dos.library");
  char buf[64];
  /* Fault / PrintFault */
  RES[0] = DOS(LVO_dos_Fault, 205, "Copy", buf, 64);
  RES[1] = strcmp(buf, "Copy: object not found") == 0;
  RES[2] = DOS(LVO_dos_Fault, 205, 0, buf, 8);             /* truncated to 7 + NUL */
  RES[3] = strcmp(buf, "object ") == 0;
  RES[4] = DOS(LVO_dos_Fault, 0, "x", buf, 64);            /* code 0: empty */
  DOS(LVO_dos_Fault, 7777, 0, buf, 64);
  RES[5] = strcmp(buf, "Error 7777") == 0;
  RES[6] = DOS(LVO_dos_PrintFault, 214, "Tool", 0, 0);     /* prints "Tool: disk is write-protected\n" */
  RES[7] = IOERR();                                        /* PrintFault leaves the code in IoErr: 214 */
  /* StrToLong */
  i32 v = 99;
  RES[8] = DOS(LVO_dos_StrToLong, "  -123abc", &v, 0, 0);  /* 6 chars used, -123 */
  RES[9] = (u32)v;
  RES[10] = DOS(LVO_dos_StrToLong, "abc", &v, 0, 0);       /* no number: -1 and value 0 */
  RES[11] = (u32)v;
  /* SplitName */
  RES[12] = LVO6(dos, LVO_dos_SplitName, 0, "a/bc/d", '/', buf, 0, 64, 0, 0);
  RES[13] = strcmp(buf, "a") == 0;
  RES[14] = LVO6(dos, LVO_dos_SplitName, 0, "a/bc/d", '/', buf, 2, 64, 0, 0);
  RES[15] = strcmp(buf, "bc") == 0;
  RES[16] = LVO6(dos, LVO_dos_SplitName, 0, "a/bc/d", '/', buf, 5, 64, 0, 0);   /* the last piece: -1 */
  RES[17] = strcmp(buf, "d") == 0;
  /* DateToStr / StrToDate (struct DateTime: stamp[3], format, flags, then three string pointers, 26 bytes) */
  struct { u32 stamp[3]; u8 format, flags; char *day, *date, *time; } dt;
  char day[16], date[16], time[16];
  memset(&dt, 0, sizeof dt);
  dt.day = day; dt.date = date; dt.time = time;
  dt.stamp[0] = 17000; dt.stamp[1] = 13 * 60 + 5; dt.stamp[2] = 7 * 50;       /* day 17000 at 13:05:07 */
  RES[18] = DOS(LVO_dos_DateToStr, &dt, 0, 0, 0);
  RES[19] = strcmp(time, "13:05:07") == 0;
  RES[20] = strlen(day) > 3 && strlen(date) == 9;           /* "dd-mmm-yy" */
  dt.format = 2;                                            /* FORMAT_USA: mm-dd-yy */
  DOS(LVO_dos_DateToStr, &dt, 0, 0, 0);
  RES[21] = strlen(date) == 8 && date[2] == '-';
  dt.format = 0;
  DOS(LVO_dos_DateToStr, &dt, 0, 0, 0);
  dt.stamp[0] = 0; dt.stamp[1] = 0; dt.stamp[2] = 0;        /* parse our own text back */
  RES[22] = DOS(LVO_dos_StrToDate, &dt, 0, 0, 0);
  RES[23] = dt.stamp[0] == 17000 && dt.stamp[1] == 13 * 60 + 5 && dt.stamp[2] == 7 * 50;
  dt.date = "32-Jan-26";                                    /* no such day */
  RES[24] = DOS(LVO_dos_StrToDate, &dt, 0, 0, 0);
  RES[25] = IOERR();                                        /* 115 */
  /* handles */
  u32 fh = DOS(LVO_dos_Open, "SYS:Dir/f.txt", 1005, 0, 0);
  u32 pl = DOS(LVO_dos_ParentOfFH, fh, 0, 0, 0);
  DOS(LVO_dos_NameFromLock, pl, buf, 64, 0);
  RES[26] = strcmp(buf, "SYS:Dir") == 0;
  DOS(LVO_dos_UnLock, pl, 0, 0, 0);
  RES[27] = DOS(LVO_dos_WaitForChar, fh, 1000, 0, 0);       /* data waits in the file: TRUE */
  RES[28] = DOS(LVO_dos_SetMode, fh, 1, 0, 0);              /* not a console: FALSE */
  RES[29] = DOS(LVO_dos_SetMode, DOS(LVO_dos_Output, 0, 0, 0, 0), 1, 0, 0);   /* console: TRUE */
  RES[30] = DOS(LVO_dos_WaitForChar, DOS(LVO_dos_Input, 0, 0, 0, 0), 1000, 0, 0);   /* no input: FALSE */
  u32 lk = DOS(LVO_dos_Lock, "SYS:Dir/f.txt", -2, 0, 0);
  RES[31] = DOS(LVO_dos_ChangeMode, 0, lk, (u32)-1, 0);     /* CHANGE_LOCK to exclusive while the file is open: refused */
  RES[32] = IOERR();                                        /* 202 */
  DOS(LVO_dos_Close, fh, 0, 0, 0);
  RES[33] = DOS(LVO_dos_ChangeMode, 0, lk, (u32)-1, 0);     /* now alone: TRUE */
  RES[34] = DOS(LVO_dos_Lock, "SYS:Dir/f.txt", -2, 0, 0);   /* exclusive now: a second lock is refused */
  DOS(LVO_dos_UnLock, lk, 0, 0, 0);
  /* streams, args, CLI */
  u32 out = DOS(LVO_dos_Output, 0, 0, 0, 0);
  u32 f2 = DOS(LVO_dos_Open, "RAM:redir", 1006, 0, 0);
  u32 old = DOS(LVO_dos_SelectOutput, f2, 0, 0, 0);
  RES[35] = old == out && DOS(LVO_dos_Output, 0, 0, 0, 0) == f2;
  DOS(LVO_dos_SelectOutput, old, 0, 0, 0);
  RES[36] = DOS(LVO_dos_Output, 0, 0, 0, 0) == out;
  DOS(LVO_dos_Close, f2, 0, 0, 0);
  u32 args = DOS(LVO_dos_GetArgStr, 0, 0, 0, 0);
  RES[37] = *(const char *)args == '\n';                    /* the engine's empty CLI line */
  DOS(LVO_dos_SetArgStr, "new\n", 0, 0, 0);
  RES[38] = strcmp((const char *)DOS(LVO_dos_GetArgStr, 0, 0, 0, 0), "new\n") == 0;
  RES[39] = DOS(LVO_dos_MaxCli, 0, 0, 0, 0);
  RES[40] = DOS(LVO_dos_FindCliProc, 1, 0, 0, 0) != 0 && DOS(LVO_dos_FindCliProc, 2, 0, 0, 0) == 0;
  RES[41] = DOS(LVO_dos_CheckSignal, 0x1000, 0, 0, 0);      /* nothing pending */
  RES[43] = LVO6(dos, LVO_dos_ReadLink, 0, 0, 0, "SYS:Dir/f.txt", buf, 64, 0, 0);   /* exists but is not a link: -1 */
  RES[44] = IOERR();                                        /* 212 */
  RES[45] = LVO6(dos, LVO_dos_LockRecord, 0, 0, 0, 0, 0, 0, 0, 0);
  CLOSELIB(dos);
}
