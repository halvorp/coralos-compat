/* iffparse.library reading: PropChunk / CollectionChunk / StopChunk, ParseIFF SCAN / STEP / RAWSTEP, CurrentChunk / ParentChunk,
 * FindProp / FindCollection / FindPropContext, ReadChunkBytes, guest EntryHandler / ExitHandler hooks, SetLocalItemPurge and the
 * error returns. The test writes RAM:in.iff (and bad files) before the run (written for this repo, MIT). Results in RES[]. */
#include "rt.h"
#define V(i) (*(volatile u32 *)&RES[i])
#define ID(a, b, c, d) (((u32)(a) << 24) | ((u32)(b) << 16) | ((u32)(c) << 8) | (u32)(d))
#define TEST ID('T','E','S','T')
#define NAME ID('N','A','M','E')
#define CMNT ID('C','M','N','T')
#define BODY ID('B','O','D','Y')
#define HEAD ID('H','E','A','D')
#define FORM ID('F','O','R','M')
struct CN { u32 succ, pred; u32 id, type; i32 size, scan; };
struct SP { i32 size; const u8 *data; };
struct CI { struct CI *next; i32 size; const u8 *data; };
u32 hook_stub(void); u32 hook_c(u32 hook, u32 obj, u32 msg);
asm(".section .text.zz,\"ax\"\n.align 2\n.globl hook_stub\nhook_stub:\n\tmove.l %a1,-(%sp)\n\tmove.l %a2,-(%sp)\n\tmove.l %a0,-(%sp)\n\tjsr hook_c(%pc)\n\tlea 12(%sp),%sp\n\trts\n");
#define OPEN(name, mode) LVO5(dos, LVO_dos_Open, 0, name, mode, 0, 0, 0, 0)
#define I(off, d0, d1, d2, a0, a1, a2) LVOX(ib, off, d0, d1, d2, 0, a0, a1, a2, 0)
static u32 ib, dos;
COLD static u32 begin(const char *file) {                 /* AllocIFF + the file as a DOS stream + OpenIFF */
  u32 f = OPEN(file, 1005);
  u32 iff = LVO(ib, LVO_iffparse_AllocIFF, 0, 0, 0, 0, 0, 0);
  *(volatile u32 *)iff = f;                               /* iff_Stream = the BPTR */
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
  V(127) = ib;
  u8 buf[32];
  /* ---- A. Prop / Collection / Stop, then read the BODY chunk ---- */
  u32 iff = begin("RAM:in.iff");
  V(0) = I(LVO_iffparse_OpenIFF, 0, 0, 0, iff, 0, 0);
  V(1) = I(LVO_iffparse_PropChunk, TEST, NAME, 0, iff, 0, 0);
  V(2) = I(LVO_iffparse_CollectionChunk, TEST, CMNT, 0, iff, 0, 0);
  V(3) = I(LVO_iffparse_StopChunk, TEST, BODY, 0, iff, 0, 0);
  V(4) = I(LVO_iffparse_ParseIFF, 0, 0, 0, iff, 0, 0);                    /* SCAN: runs to the BODY stop: 0 */
  struct CN *cn = (struct CN *)I(LVO_iffparse_CurrentChunk, 0, 0, 0, iff, 0, 0);
  V(5) = cn->id == BODY; V(6) = cn->size; V(7) = cn->scan; V(8) = cn->type == TEST;
  struct CN *pc = (struct CN *)I(LVO_iffparse_ParentChunk, 0, 0, 0, (u32)cn, 0, 0);
  V(9) = pc->id == FORM; V(10) = pc->type == TEST;
  V(11) = I(LVO_iffparse_ParentChunk, 0, 0, 0, (u32)pc, 0, 0) == 0;       /* the outermost chunk has no parent */
  V(12) = I(LVO_iffparse_FindPropContext, 0, 0, 0, iff, 0, 0) == (u32)pc;
  struct SP *sp = (struct SP *)I(LVO_iffparse_FindProp, TEST, NAME, 0, iff, 0, 0);
  V(13) = sp->size; V(14) = memcmp(sp->data, "Beta", 4) == 0;             /* the later NAME replaced "Alpha" */
  struct CI *ci = (struct CI *)I(LVO_iffparse_FindCollection, TEST, CMNT, 0, iff, 0, 0);
  u32 n = 0; const u8 *first = 0, *last = 0;
  for (struct CI *c = ci; c; c = c->next) { n++; if (!first) first = c->data; last = c->data; }
  V(15) = n; V(16) = first && memcmp(first, "c3", 2) == 0; V(17) = last && memcmp(last, "c1", 2) == 0;   /* most recent first */
  V(18) = I(LVO_iffparse_FindProp, TEST, ID('X','X','X','X'), 0, iff, 0, 0) == 0;
  V(19) = I(LVO_iffparse_ReadChunkBytes, 5, 0, 0, iff, (u32)buf, 0);     /* 5 */
  V(20) = cn->scan;
  V(21) = I(LVO_iffparse_ReadChunkBytes, 10, 0, 0, iff, (u32)buf + 5, 0);  /* only 3 are left */
  V(22) = I(LVO_iffparse_ReadChunkBytes, 4, 0, 0, iff, (u32)buf + 8, 0);   /* none */
  V(23) = memcmp(buf, "01234567", 8) == 0;
  V(24) = I(LVO_iffparse_ParseIFF, 0, 0, 0, iff, 0, 0);                   /* on to the end of the file: IFFERR_EOF */
  V(25) = I(LVO_iffparse_CurrentChunk, 0, 0, 0, iff, 0, 0);
  V(26) = I(LVO_iffparse_ParseIFF, 0, 0, 0, iff, 0, 0);                   /* parsing again past the end stays at EOF */
  finish(iff);
  /* ---- B. The program's own hooks (A0 = hook, A2 = object, A1 = command) ---- */
  u32 h1[5] = { 0, 0, (u32)hook_stub, 0, 1 }, h2[5] = { 0, 0, (u32)hook_stub, 0, 2 }, h3[5] = { 0, 0, (u32)hook_stub, 0, 3 }, h4[5] = { 0, 0, (u32)hook_stub, 0, 4 }, h5[5] = { 0, 0, (u32)hook_stub, 0, 5 };
  iff = begin("RAM:in.iff");
  I(LVO_iffparse_OpenIFF, 0, 0, 0, iff, 0, 0);
  V(30) = LVOX(ib, LVO_iffparse_EntryHandler, TEST, NAME, 2, 0, iff, (u32)h1, 0xBEEF, 0);
  V(31) = LVOX(ib, LVO_iffparse_ExitHandler, TEST, CMNT, 2, 0, iff, (u32)h4, 0xCAFE, 0);
  V(32) = I(LVO_iffparse_ParseIFF, 0, 0, 0, iff, 0, 0);                   /* the hooks return 0: SCAN reaches the end */
  finish(iff);
  iff = begin("RAM:in.iff");                                              /* a hook returning IFF_RETURN2CLIENT stops the scan with 0 */
  I(LVO_iffparse_OpenIFF, 0, 0, 0, iff, 0, 0);
  LVOX(ib, LVO_iffparse_EntryHandler, TEST, BODY, 2, 0, iff, (u32)h2, 0x22, 0);
  V(33) = I(LVO_iffparse_ParseIFF, 0, 0, 0, iff, 0, 0);
  struct CN *cur = (struct CN *)I(LVO_iffparse_CurrentChunk, 0, 0, 0, iff, 0, 0);
  V(34) = cur->id == BODY;
  finish(iff);
  iff = begin("RAM:in.iff");                                              /* a hook returning an error code returns it from ParseIFF */
  I(LVO_iffparse_OpenIFF, 0, 0, 0, iff, 0, 0);
  LVOX(ib, LVO_iffparse_EntryHandler, TEST, BODY, 2, 0, iff, (u32)h5, 0x55, 0);
  V(35) = I(LVO_iffparse_ParseIFF, 0, 0, 0, iff, 0, 0);
  finish(iff);
  /* ---- C. STEP and RAWSTEP: where ParseIFF returns, and what CurrentChunk says there ---- */
  iff = begin("RAM:in.iff");
  I(LVO_iffparse_OpenIFF, 0, 0, 0, iff, 0, 0);
  u32 k = 0;
  for (;;) {
    i32 r = (i32)I(LVO_iffparse_ParseIFF, 1, 0, 0, iff, 0, 0);
    struct CN *c = (struct CN *)I(LVO_iffparse_CurrentChunk, 0, 0, 0, iff, 0, 0);
    V(84 + 2 * k) = (u32)r; V(85 + 2 * k) = c ? c->id : 0; k++;
    if (r == -1 || k >= 18) break;
  }
  V(36) = k;
  finish(iff);
  iff = begin("RAM:in.iff");
  I(LVO_iffparse_OpenIFF, 0, 0, 0, iff, 0, 0);
  LVOX(ib, LVO_iffparse_EntryHandler, TEST, NAME, 2, 0, iff, (u32)h1, 0xBEEF, 0);
  k = 0;
  for (; k < 30; k++) { if ((i32)I(LVO_iffparse_ParseIFF, 2, 0, 0, iff, 0, 0) == -1) break; }
  V(37) = k + 1;                                                          /* RAWSTEP stops at every entry and exit; handlers do not run */
  finish(iff);
  /* ---- D. Errors ---- */
  iff = begin("RAM:garbage.iff"); V(40) = I(LVO_iffparse_OpenIFF, 0, 0, 0, iff, 0, 0); V(41) = I(LVO_iffparse_ParseIFF, 0, 0, 0, iff, 0, 0); finish(iff);   /* NOTIFF twice */
  iff = begin("RAM:empty.iff"); V(42) = I(LVO_iffparse_OpenIFF, 0, 0, 0, iff, 0, 0); finish(iff);
  iff = begin("RAM:badtype.iff"); V(43) = I(LVO_iffparse_OpenIFF, 0, 0, 0, iff, 0, 0); finish(iff);
  iff = begin("RAM:cut.iff");                                             /* a chunk longer than the file */
  I(LVO_iffparse_OpenIFF, 0, 0, 0, iff, 0, 0);
  I(LVO_iffparse_PropChunk, TEST, NAME, 0, iff, 0, 0);
  V(44) = I(LVO_iffparse_ParseIFF, 0, 0, 0, iff, 0, 0);                  /* PropFunc cannot read it in full: the stream fails (IFFERR_READ) */
  finish(iff);
  iff = LVO(ib, LVO_iffparse_AllocIFF, 0, 0, 0, 0, 0, 0);
  V(45) = I(LVO_iffparse_OpenIFF, 0, 0, 0, iff, 0, 0);                    /* no InitIFF: IFFERR_NOHOOK */
  V(46) = I(LVO_iffparse_ParseIFF, 0, 0, 0, iff, 0, 0);                   /* not open: IFFERR_NOTIFF */
  V(47) = I(LVO_iffparse_PopChunk, 0, 0, 0, iff, 0, 0);                   /* no chunk open: IFFERR_NOSCOPE */
  LVO(ib, LVO_iffparse_FreeIFF, 0, 0, 0, 0, iff, 0);
  /* ---- E. Local items: a program's purge hook runs (and frees the item) when the handle is freed ---- */
  iff = LVO(ib, LVO_iffparse_AllocIFF, 0, 0, 0, 0, 0, 0);
  u32 lci = LVOX(ib, LVO_iffparse_AllocLocalItem, TEST, HEAD, ID('u','s','e','r'), 12, 0, 0, 0, 0);
  V(50) = lci != 0;
  V(51) = I(LVO_iffparse_LocalItemData, 0, 0, 0, lci, 0, 0) != 0;
  LVO(ib, LVO_iffparse_SetLocalItemPurge, 0, 0, 0, 0, lci, (u32)h3);
  V(52) = I(LVO_iffparse_StoreLocalItem, 1, 0, 0, iff, lci, 0);           /* IFFSLI_ROOT */
  V(53) = I(LVO_iffparse_FindLocalItem, TEST, HEAD, ID('u','s','e','r'), iff, 0, 0) == lci;
  V(54) = I(LVO_iffparse_FindLocalItem, TEST, HEAD, ID('n','o','p','e'), iff, 0, 0) == 0;
  V(55) = I(LVO_iffparse_StoreLocalItem, 3, 0, 0, iff, lci, 0);           /* IFFSLI_PROP with no FORM open: IFFERR_NOSCOPE */
  V(59) = lci;
  LVO(ib, LVO_iffparse_FreeIFF, 0, 0, 0, 0, iff, 0);                      /* the purge hook runs now */
  V(56) = V(60 + 3);                                                      /* it ran once */
  CLOSELIB(ib); CLOSELIB(dos);
}
u32 hook_c(u32 hook, u32 obj, u32 msg) {
  u32 id = *(volatile u32 *)(hook + 16);
  V(60 + id)++;
  V(66 + 3 * id) = hook; V(67 + 3 * id) = obj; V(68 + 3 * id) = msg;
  if (id == 2) return (u32)-12;                                           /* IFF_RETURN2CLIENT */
  if (id == 5) return (u32)-77;
  if (id == 3) LVO(V(127), LVO_iffparse_FreeLocalItem, 0, 0, 0, 0, obj, 0);   /* the purge hook frees the item itself */
  return 0;
}
