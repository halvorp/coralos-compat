/* iffparse.library writing: a FORM with nested chunks (known and unknown sizes, an odd-length chunk, records, an inner FORM), a LIST
 * with a PROP, the syntax checks of a write-mode PushChunk, then the file read back with ParseIFF (written for this repo, MIT). */
#include "rt.h"
#define V(i) (*(volatile u32 *)&RES[i])
#define ID(a, b, c, d) (((u32)(a) << 24) | ((u32)(b) << 16) | ((u32)(c) << 8) | (u32)(d))
#define TEST ID('T','E','S','T')
#define SUBF ID('S','U','B','F')
#define HEAD ID('H','E','A','D')
#define ODDX ID('O','D','D','X')
#define RECS ID('R','E','C','S')
#define DATA ID('D','A','T','A')
#define FORM ID('F','O','R','M')
#define LIST ID('L','I','S','T')
#define PROP ID('P','R','O','P')
struct CN { u32 succ, pred; u32 id, type; i32 size, scan; };
static u32 ib, dos;
#define I(off, d0, d1, d2, a0, a1) LVOX(ib, off, d0, d1, d2, 0, a0, a1, 0, 0)
#define PUSH(iff, ty, id, size) I(LVO_iffparse_PushChunk, ty, id, size, iff, 0)
#define POP(iff) I(LVO_iffparse_PopChunk, 0, 0, 0, iff, 0)
#define WRITE(iff, p, n) I(LVO_iffparse_WriteChunkBytes, n, 0, 0, iff, (u32)(p))
COLD static u32 begin(const char *file, u32 mode) {
  u32 f = LVO5(dos, LVO_dos_Open, 0, file, mode, 0, 0, 0, 0);
  u32 iff = LVO(ib, LVO_iffparse_AllocIFF, 0, 0, 0, 0, 0, 0);
  *(volatile u32 *)iff = f;
  LVO(ib, LVO_iffparse_InitIFFasDOS, 0, 0, 0, 0, iff, 0);
  return iff;
}
COLD static void finish(u32 iff) {
  u32 f = *(volatile u32 *)iff;
  LVO(ib, LVO_iffparse_CloseIFF, 0, 0, 0, 0, iff, 0);
  LVO(ib, LVO_iffparse_FreeIFF, 0, 0, 0, 0, iff, 0);
  LVO5(dos, LVO_dos_Close, 0, f, 0, 0, 0, 0, 0);
}
void _start(void) {
  ib = OPENLIB("iffparse.library"); dos = OPENLIB("dos.library");
  /* ---- the FORM ---- */
  u32 iff = begin("RAM:out.iff", 1006);
  V(0) = I(LVO_iffparse_OpenIFF, 1, 0, 0, iff, 0);
  V(1) = PUSH(iff, TEST, FORM, -1);
  V(2) = PUSH(iff, 0, HEAD, 6);
  V(3) = WRITE(iff, "abcdef", 6);
  struct CN *cn = (struct CN *)I(LVO_iffparse_CurrentChunk, 0, 0, 0, iff, 0);
  V(4) = cn->scan;                                               /* 6 */
  V(5) = WRITE(iff, "zzzz", 4);                                  /* the chunk's size is known and used up: 0 bytes */
  V(6) = POP(iff);
  V(7) = PUSH(iff, 0, ODDX, -1);
  V(8) = WRITE(iff, "hello", 5);                                 /* odd length: PopChunk pads */
  V(9) = POP(iff);
  V(10) = PUSH(iff, 0, RECS, -1);
  u8 recs[12] = { 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12 };
  V(11) = I(LVO_iffparse_WriteChunkRecords, 4, 3, 0, iff, (u32)recs);   /* 3 records of 4 bytes */
  V(12) = POP(iff);
  V(13) = PUSH(iff, SUBF, FORM, -1);
  V(14) = PUSH(iff, 0, DATA, 3);
  V(15) = WRITE(iff, "xyz", 3);
  V(16) = POP(iff);                                              /* DATA (odd length, pad) */
  V(17) = POP(iff);                                              /* the inner FORM: its unknown size is patched in */
  V(18) = POP(iff);                                              /* the outer FORM */
  V(19) = I(LVO_iffparse_PopChunk, 0, 0, 0, iff, 0);             /* nothing open: IFFERR_NOSCOPE */
  finish(iff);
  /* ---- a LIST with a PROP ---- */
  iff = begin("RAM:list.iff", 1006);
  I(LVO_iffparse_OpenIFF, 1, 0, 0, iff, 0);
  V(20) = PUSH(iff, TEST, LIST, -1) | PUSH(iff, TEST, PROP, -1) | PUSH(iff, 0, HEAD, 2);
  V(23) = WRITE(iff, "hi", 2);
  V(20) |= POP(iff) | POP(iff);
  V(21) = PUSH(iff, TEST, FORM, -1) | PUSH(iff, 0, DATA, 2);
  V(24) = WRITE(iff, "yo", 2);
  V(21) |= POP(iff) | POP(iff);
  V(22) = POP(iff);
  finish(iff);
  /* ---- the syntax checks of a write-mode PushChunk (a separate file each, nothing is written by the failures) ---- */
  iff = begin("RAM:bad.iff", 1006);
  I(LVO_iffparse_OpenIFF, 1, 0, 0, iff, 0);
  V(30) = PUSH(iff, 0, HEAD, 4);                                 /* the first chunk must be a FORM, LIST or CAT: IFFERR_NOTIFF */
  V(39) = PUSH(iff, TEST, FORM, -1);                             /* ... and that attempt used up IFFF_NEWFILE: nothing more may start the file (IFFERR_EOF) */
  finish(iff);
  iff = begin("RAM:bad.iff", 1006);
  I(LVO_iffparse_OpenIFF, 1, 0, 0, iff, 0);
  V(31) = PUSH(iff, TEST, FORM, -1);
  V(32) = PUSH(iff, TEST, PROP, -1);                             /* a PROP only inside a LIST: IFFERR_SYNTAX */
  V(33) = PUSH(iff, 0, ID('A', 0x01, 'C', 'D'), 4);              /* not a valid ID: IFFERR_SYNTAX */
  V(34) = PUSH(iff, ID('t', 'e', 's', 't'), FORM, -1);           /* not a valid type: IFFERR_NOTIFF */
  V(35) = PUSH(iff, 0, HEAD, 4);                                 /* fine inside the FORM */
  V(36) = PUSH(iff, 0, DATA, 4);                                 /* a plain chunk inside a plain chunk: IFFERR_SYNTAX */
  finish(iff);
  V(37) = LVO(ib, LVO_iffparse_GoodID, ID('F','O','R','M'), 0, 0, 0, 0, 0) + 2 * LVO(ib, LVO_iffparse_GoodID, ID(' ','A','B','C'), 0, 0, 0, 0, 0)
        + 4 * LVO(ib, LVO_iffparse_GoodType, ID('i','l','b','m'), 0, 0, 0, 0, 0) + 8 * LVO(ib, LVO_iffparse_GoodType, ID('I','L','B','M'), 0, 0, 0, 0, 0)
        + 16 * LVO(ib, LVO_iffparse_GoodID, ID(' ',' ',' ',' '), 0, 0, 0, 0, 0);   /* 1 + 0 + 0 + 8 + 16 */
  char s[8];
  LVO(ib, LVO_iffparse_IDtoStr, ID('B','M','H','D'), 0, 0, 0, (u32)s, 0);
  V(38) = strcmp(s, "BMHD") == 0;
  /* ---- read it back with ParseIFF(STEP) ---- */
  iff = begin("RAM:out.iff", 1005);
  V(40) = I(LVO_iffparse_OpenIFF, 0, 0, 0, iff, 0);
  u32 k = 0; char buf[16]; u32 got = 0;
  for (;;) {
    i32 r = (i32)I(LVO_iffparse_ParseIFF, 1, 0, 0, iff, 0);
    struct CN *c = (struct CN *)I(LVO_iffparse_CurrentChunk, 0, 0, 0, iff, 0);
    if (r == 0 && c && c->id == ODDX) { got = I(LVO_iffparse_ReadChunkBytes, 16, 0, 0, iff, (u32)buf); }
    V(50 + 2 * k) = (u32)r; V(51 + 2 * k) = c ? c->id : 0; k++;
    if (r == -1 || k >= 30) break;
  }
  V(41) = k; V(42) = got; V(43) = memcmp(buf, "hello", 5) == 0;
  finish(iff);
  CLOSELIB(ib); CLOSELIB(dos);
}
