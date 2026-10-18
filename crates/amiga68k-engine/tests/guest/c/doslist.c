/* The DOS device list: LockDosList walks, FindDosEntry, DosInfo, GetDeviceProc, Add/RemDosEntry (written for this repo, MIT). */
#include "rt.h"
#define LDF_READ 1
#define LDF_WRITE 2
#define LDF_DEVICES 4
#define LDF_VOLUMES 8
#define LDF_ASSIGNS 16
#define ALL (LDF_READ | LDF_DEVICES | LDF_VOLUMES | LDF_ASSIGNS)
#define DOS(off, d1, d2, d3, d4) LVO5(dos, off, 0, d1, d2, d3, d4, 0, 0)
#define LOCKDOSLIST(f) DOS(LVO_dos_LockDosList, f, 0, 0, 0)
#define NEXT(d, f) DOS(LVO_dos_NextDosEntry, d, f, 0, 0)
#define FIND(d, n, f) DOS(LVO_dos_FindDosEntry, d, n, f, 0)
#define IOERR() DOS(LVO_dos_IoErr, 0, 0, 0, 0)
#define W(a, off) rd32((void *)((a) + (off)))
COLD static int bstr_is(u32 bptr, const char *s) { const u8 *b = (const u8 *)(bptr << 2); return b[0] == strlen(s) && memcmp(b + 1, s, b[0]) == 0; }
void _start(void) {
  u32 dos = OPENLIB("dos.library");
  char buf[64];
  u32 head = LOCKDOSLIST(ALL);
  u32 counts[5] = {0, 0, 0, 0, 0}, e = head, total = 0;
  while ((e = NEXT(e, ALL)) != 0 && total < 64) { counts[W(e, 4)]++; total++; }
  RES[0] = counts[1];                    /* assigns: LIBS DEVS C S L ENV ENVARC T PROGDIR = 9 */
  RES[1] = counts[0];                                /* DLT_DEVICE nodes: SYS RAM DOpus5 = 3 */
  RES[2] = counts[2];                                /* DLT_VOLUME nodes: 3 */
  e = FIND(head, "ram", LDF_DEVICES);
  RES[3] = e && W(e, 4) == 0 && bstr_is(W(e, 40), "RAM") && W(e, 8) != 0 && W(e, 12) == 0;
  u32 v = FIND(head, "RAM", LDF_VOLUMES);
  RES[4] = v && W(v, 4) == 2 && W(v, 32) == 0x444F5300 && W(v, 16) != 0 && W(v, 8) == W(e, 8);  /* same handler port as the device */
  u32 as = FIND(head, "libs", LDF_ASSIGNS);
  RES[5] = as && W(as, 4) == 1 && W(as, 12) != 0;
  DOS(LVO_dos_NameFromLock, W(as, 12), buf, 64, 0);
  RES[6] = strcmp(buf, "SYS:Libs") == 0;
  u32 al = W(as, 20);                                 /* dol_assign.dol_List: the second directory of the multi-directory assign */
  RES[7] = al != 0 && W(al, 0) == 0;
  DOS(LVO_dos_NameFromLock, W(al, 4), buf, 64, 0);
  RES[8] = strcmp(buf, "DOpus5:Libs") == 0;
  RES[9] = FIND(head, "RAM", LDF_ASSIGNS);            /* RAM is a device, not an assign: NULL */
  RES[10] = FIND(head, "nope", ALL);
  RES[11] = IOERR();                                  /* 205 */
  e = head;
  u32 last = 0;
  while ((e = NEXT(e, ALL))) last = e;
  RES[13] = NEXT(last, ALL);                          /* past the end: NULL */
  RES[14] = IOERR();                                  /* ERROR_NO_MORE_ENTRIES 232 */
  RES[15] = DOS(LVO_dos_AttemptLockDosList, LDF_WRITE | LDF_ASSIGNS, 0, 0, 0);   /* AttemptLockDosList(WRITE) while read-locked: NULL */
  DOS(LVO_dos_UnLockDosList, ALL, 0, 0, 0);                            /* UnLockDosList */
  RES[16] = DOS(LVO_dos_AttemptLockDosList, LDF_WRITE | LDF_ASSIGNS, 0, 0, 0) != 0;
  DOS(LVO_dos_UnLockDosList, LDF_WRITE | LDF_ASSIGNS, 0, 0, 0);
  /* DosLibrary -> RootNode -> DosInfo -> first entry, as a program that does not lock would read it. */
  u32 first = W(W(W(dos, 34), 24) << 2, 4) << 2;
  RES[17] = first != 0 && W(first, 4) == 0 && bstr_is(W(first, 40), "SYS");
  /* GetDeviceProc */
  u32 dp = DOS(LVO_dos_GetDeviceProc, "LIBS:x", 0, 0, 0);
  RES[18] = dp ? (W(dp, 8) & 2) | (W(dp, 4) != 0) : 99;                  /* DVPF_ASSIGN, lock set */
  DOS(LVO_dos_NameFromLock, W(dp, 4), buf, 64, 0);
  RES[19] = strcmp(buf, "SYS:Libs") == 0;
  u32 dp2 = DOS(LVO_dos_GetDeviceProc, "LIBS:x", dp, 0, 0);
  DOS(LVO_dos_NameFromLock, W(dp2, 4), buf, 64, 0);
  RES[20] = dp2 == dp && strcmp(buf, "DOpus5:Libs") == 0;             /* the same DevProc, advanced to the next directory */
  RES[21] = DOS(LVO_dos_GetDeviceProc, "LIBS:x", dp2, 0, 0);                              /* no third directory: NULL, and dp is freed */
  RES[22] = IOERR();                                                      /* 232 */
  u32 rp = DOS(LVO_dos_GetDeviceProc, "RAM:foo", 0, 0, 0);
  RES[23] = rp && W(rp, 8) == 0 && W(rp, 4) == 0 && W(rp, 0) == W(FIND(head, "RAM", LDF_DEVICES), 8);
  DOS(LVO_dos_FreeDeviceProc, rp, 0, 0, 0);                                                 /* FreeDeviceProc */
  RES[24] = DOS(LVO_dos_GetDeviceProc, "Nope:x", 0, 0, 0);
  RES[25] = IOERR();                                                      /* 218 */
  RES[26] = DOS(LVO_dos_IsFileSystem, "RAM:", 0, 0, 0);                                   /* IsFileSystem */
  RES[27] = DOS(LVO_dos_IsFileSystem, "NIL:", 0, 0, 0);
  RES[28] = DOS(LVO_dos_IsFileSystem, "Nope:", 0, 0, 0);
  /* MakeDosEntry + AddDosEntry an assign, then RemDosEntry it */
  u32 nd = DOS(LVO_dos_MakeDosEntry, "TESTAS", 1, 0, 0);                                  /* MakeDosEntry(name, DLT_DIRECTORY) */
  u32 lk = DOS(LVO_dos_Lock, "RAM:", -2, 0, 0);
  *(volatile u32 *)(nd + 12) = lk;                                        /* dol_Lock */
  RES[29] = DOS(LVO_dos_AddDosEntry, nd, 0, 0, 0);                                       /* AddDosEntry */
  u32 chk = DOS(LVO_dos_Lock, "TESTAS:", -2, 0, 0);
  RES[30] = chk != 0;
  u32 nd2 = DOS(LVO_dos_MakeDosEntry, "testas", 1, 0, 0);
  *(volatile u32 *)(nd2 + 12) = lk;
  RES[31] = DOS(LVO_dos_AddDosEntry, nd2, 0, 0, 0);                                      /* a second TESTAS: FALSE */
  RES[32] = IOERR();                                                      /* ERROR_OBJECT_EXISTS 203 */
  u32 h2 = LOCKDOSLIST(LDF_WRITE | LDF_ASSIGNS);
  u32 ent = FIND(h2, "TESTAS", LDF_ASSIGNS);
  RES[33] = DOS(LVO_dos_RemDosEntry, ent, 0, 0, 0);                                      /* RemDosEntry */
  DOS(LVO_dos_UnLockDosList, LDF_WRITE | LDF_ASSIGNS, 0, 0, 0);
  RES[34] = DOS(LVO_dos_Lock, "TESTAS:", -2, 0, 0);                                /* gone */
  DOS(LVO_dos_FreeDosEntry, nd, 0, 0, 0);                                                 /* FreeDosEntry */
  DOS(LVO_dos_FreeDosEntry, nd2, 0, 0, 0);
  DOS(LVO_dos_UnLock, chk, 0, 0, 0);
  DOS(LVO_dos_UnLock, lk, 0, 0, 0);
  /* Info: id_VolumeNode is the SYS volume node */
  u32 sl = DOS(LVO_dos_Lock, "SYS:", -2, 0, 0);
  u32 id[9];
  RES[35] = DOS(LVO_dos_Info, sl, id, 0, 0);
  u32 hh = LOCKDOSLIST(LDF_VOLUMES | LDF_READ);
  RES[36] = id[7] << 2 == FIND(hh, "SYS", LDF_VOLUMES);
  DOS(LVO_dos_UnLockDosList, LDF_VOLUMES | LDF_READ, 0, 0, 0);
  DOS(LVO_dos_UnLock, sl, 0, 0, 0);
  CLOSELIB(dos);
}
