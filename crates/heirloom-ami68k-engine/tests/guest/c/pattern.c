/* ParsePattern / MatchPattern / MatchFirst / MatchNext / MatchEnd over a small tree (written for this repo, MIT). */
#include "rt.h"
#define DOS(off, d1, d2, d3, d4) LVO5(dos, off, 0, d1, d2, d3, d4, 0, 0)
#define IOERR() DOS(LVO_dos_IoErr, 0, 0, 0, 0)
#define APF_ITSWILD 2
#define APF_DODIR 4
#define APF_DIDDIR 8
#define F(ap) (*(volatile u8 *)((ap) + 16))
#define STRLEN(ap) (*(volatile i16 *)((ap) + 18))
#define BUF(ap) ((const char *)((ap) + 280))
#define NAME(ap) ((const char *)((ap) + 20 + 8))
COLD static u32 first(u32 dos, const char *pat, u32 ap) { return LVO5(dos, LVO_dos_MatchFirst, 0, pat, ap, 0, 0, 0, 0); }
COLD static u32 next(u32 dos, u32 ap) { return LVO5(dos, LVO_dos_MatchNext, 0, ap, 0, 0, 0, 0, 0); }
void _start(void) {
  u32 dos = OPENLIB("dos.library");
  char tok[40];
  u32 ap_mem[80];
  u32 ap = (u32)ap_mem;
  /* ParsePattern / MatchPattern */
  RES[0] = DOS(LVO_dos_ParsePattern, "#?.txt", tok, 40, 0);              /* 1: has wildcards */
  RES[1] = DOS(LVO_dos_MatchPattern, tok, "x.txt", 0, 0);                 /* TRUE */
  RES[2] = DOS(LVO_dos_MatchPattern, tok, "x.tx", 0, 0);                  /* FALSE */
  RES[3] = DOS(LVO_dos_MatchPatternNoCase, tok, "X.TXT", 0, 0);           /* TRUE */
  RES[4] = DOS(LVO_dos_MatchPattern, tok, "X.TXT", 0, 0);                 /* case-sensitive: FALSE */
  RES[5] = DOS(LVO_dos_ParsePattern, "plain", tok, 40, 0);                /* 0 */
  RES[6] = DOS(LVO_dos_ParsePattern, "(a|b", tok, 40, 0);                 /* -1 */
  RES[7] = IOERR();                                                       /* 114 */
  RES[8] = DOS(LVO_dos_ParsePattern, "#?.txt", tok, 2, 0);                /* buffer too small: -1 */
  RES[9] = IOERR();                                                       /* 303 */
  /* wildcard in the last component */
  memset(ap_mem, 0, sizeof ap_mem);
  STRLEN(ap) = 100;
  RES[10] = first(dos, "SYS:Docs/#?.txt", ap);                            /* 0 */
  RES[11] = strcmp(BUF(ap), "SYS:Docs/a.txt") == 0 && strcmp(NAME(ap), "a.txt") == 0;
  RES[12] = (F(ap) & APF_ITSWILD) != 0;
  u32 base = rd32((void *)ap), chain = rd32((void *)(ap + 4));        /* ap_Base, ap_Last: SYS:Docs/ is two levels (SYS: then Docs) */
  RES[13] = base != 0 && chain != base && rd32((void *)base) == chain && rd32((void *)(chain + 4)) == base;
  char buf[64];
  DOS(LVO_dos_NameFromLock, rd32((void *)(chain + 8)), buf, 64, 0);
  RES[14] = strcmp(buf, "SYS:Docs") == 0;                                 /* the last AChain's lock is the directory being scanned */
  RES[15] = next(dos, ap);                                                /* b.txt */
  RES[16] = strcmp(NAME(ap), "b.txt") == 0;
  RES[17] = next(dos, ap);                                                /* no more: 232 */
  RES[18] = IOERR();
  LVO5(dos, LVO_dos_MatchEnd, 0, ap, 0, 0, 0, 0, 0);
  RES[19] = rd32((void *)ap);                                             /* ap_Base cleared by MatchEnd */
  /* a directory, entered and left (DODIR left set) */
  memset(ap_mem, 0, sizeof ap_mem);
  STRLEN(ap) = 100;
  u32 all = 0, dids = 0, err = first(dos, "SYS:Docs", ap);
  while (err == 0 && all < 40) { all++; dids += (F(ap) & APF_DIDDIR) != 0; err = next(dos, ap); }
  RES[20] = all;                                                          /* 11 */
  RES[21] = dids;                                                         /* 3: Sub, Sub2, Docs */
  RES[22] = err;                                                          /* 232 */
  LVO5(dos, LVO_dos_MatchEnd, 0, ap, 0, 0, 0, 0, 0);
  /* DODIR cleared at once: only the directory itself */
  memset(ap_mem, 0, sizeof ap_mem);
  STRLEN(ap) = 100;
  u32 n1 = 0;
  err = first(dos, "SYS:Docs", ap);
  while (err == 0 && n1 < 40) { n1++; F(ap) &= ~APF_DODIR; err = next(dos, ap); }
  RES[23] = n1;                                                           /* 1 */
  LVO5(dos, LVO_dos_MatchEnd, 0, ap, 0, 0, 0, 0, 0);
  /* one level: enter the first directory only */
  memset(ap_mem, 0, sizeof ap_mem);
  STRLEN(ap) = 100;
  u32 n2 = 0;
  err = first(dos, "SYS:Docs", ap);
  while (err == 0 && n2 < 40) { if (n2 > 0) F(ap) &= ~APF_DODIR; n2++; err = next(dos, ap); }
  RES[24] = n2;                                                           /* Docs + a b c Sub Sub2 + Docs again = 7 */
  LVO5(dos, LVO_dos_MatchEnd, 0, ap, 0, 0, 0, 0, 0);
  /* wildcards in two components */
  memset(ap_mem, 0, sizeof ap_mem);
  STRLEN(ap) = 100;
  RES[25] = first(dos, "SYS:#?/#?.info", ap);
  RES[26] = strcmp(BUF(ap), "SYS:Docs/c.info") == 0;
  RES[27] = next(dos, ap);
  RES[28] = rd32((void *)(rd32((void *)(ap + 4)) + 4)) != 0;             /* ap_Last has a parent: two chains */
  LVO5(dos, LVO_dos_MatchEnd, 0, ap, 0, 0, 0, 0, 0);
  /* misses */
  memset(ap_mem, 0, sizeof ap_mem);
  RES[29] = first(dos, "SYS:Nope/x", ap);                                 /* 205 */
  RES[30] = first(dos, "SYS:Docs/#?.zzz", ap);                            /* 232 */
  RES[31] = first(dos, "SYS:Docs/a.txt", ap);                             /* plain name: 0 */
  RES[32] = (F(ap) & APF_ITSWILD) == 0;
  RES[33] = next(dos, ap);                                                /* a file is not entered: 232 */
  LVO5(dos, LVO_dos_MatchEnd, 0, ap, 0, 0, 0, 0, 0);
  /* buffer too small for the path */
  memset(ap_mem, 0, sizeof ap_mem);
  STRLEN(ap) = 5;
  RES[34] = first(dos, "SYS:Docs/a.txt", ap);                             /* 303 */
  CLOSELIB(dos);
}
