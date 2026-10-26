/* locale.library: the built-in English Locale, strings, character classes, StrnCmp, no catalogs, FormatDate /
 * FormatString (through a putChar hook) and ParseDate (through a getChar hook). Written for this repo (MIT). */
#include "rt.h"
#define S(p) ((const char *)(p))
#define V(i) (*(volatile u32 *)&RES[i])
/* A library call with D0/D1 and A0..A3 (LVO() only has A0/A1). */
#define CALL(base, off, D0, D1, A0, A1, A2, A3) ({ \
  register u32 _d0 asm("d0") = (u32)(D0); register u32 _d1 asm("d1") = (u32)(D1); \
  register u32 _a0 asm("a0") = (u32)(A0); register u32 _a1 asm("a1") = (u32)(A1); \
  register u32 _a2 asm("a2") = (u32)(A2); register u32 _a3 asm("a3") = (u32)(A3); \
  asm volatile("move.l %%a6,-(%%sp)\n\tmove.l %[b],%%a6\n\tjsr %c[o](%%a6)\n\tmove.l (%%sp)+,%%a6" \
               : "+r"(_d0), "+r"(_d1), "+r"(_a0), "+r"(_a1) : "r"(_a2), "r"(_a3), [b] "r"((u32)(base)), [o] "i"(off) : "cc", "memory"); _d0; })
#define L(off, D0, D1, A0, A1, A2, A3) CALL(loc_lib, off, D0, D1, A0, A1, A2, A3)
void put_stub(void); void get_stub(void);
asm(".section .text.zz,\"ax\"\n.align 2\n"
    ".globl put_stub\nput_stub:\n\tmove.l 16(%a0),%a3\n\tmove.l (%a3),%a4\n\tmove.l %a1,%d0\n\tmove.b %d0,(%a4)+\n\tmove.l %a4,(%a3)\n\trts\n"
    ".globl get_stub\nget_stub:\n\tmove.l 16(%a0),%a3\n\tmove.l (%a3),%a4\n\tmoveq #0,%d0\n\tmove.b (%a4),%d0\n\tbeq 1f\n\taddq.l #1,(%a3)\n1:\trts\n");

struct ds { u32 days, minute, tick; };
#define DAYS_2024_02_29 16860   /* days since 1978-01-01; a Thursday */

void _start(void) {
  u32 loc_lib = OPENLIB("locale.library");
  V(0) = loc_lib != 0;
  u32 loc = L(LVO_locale_OpenLocale, 0, 0, 0, 0, 0, 0);
  V(1) = loc != 0;
  V(2) = strcmp(S(rd32((void *)loc)), "united_states.country") == 0 && strcmp(S(rd32((void *)(loc + 4))), "english.language") == 0
      && strcmp(S(rd32((void *)(loc + 8))), "english") == 0;
  V(3) = strcmp(S(rd32((void *)(loc + 76))), "%A %B %e") == 0 && strcmp(S(rd32((void *)(loc + 72))), "%A %B %e %Y %Q:%M %p") == 0;
  V(4) = strcmp(S(rd32((void *)(loc + 96))), ".") == 0 && strcmp(S(rd32((void *)(loc + 100))), ",") == 0 && strcmp(S(rd32((void *)(loc + 140))), "$") == 0;
  V(5) = *(u8 *)(loc + 68) * 100 + *(u8 *)(loc + 69);                 /* MS_AMERICAN 1, CT_7SUN 0 -> 100 */
  V(6) = rd32((void *)(loc + 56));                                    /* MAKE_ID('U','S','A',0) */
  V(7) = *(u8 *)rd32((void *)(loc + 108));                            /* loc_Grouping[0] = 3 */
  V(8) = strcmp(S(L(LVO_locale_GetLocaleStr, 1, 0, loc, 0, 0, 0)), "Sunday") == 0 && strcmp(S(L(LVO_locale_GetLocaleStr, 15, 0, loc, 0, 0, 0)), "January") == 0
      && strcmp(S(L(LVO_locale_GetLocaleStr, 41, 0, loc, 0, 0, 0)), "am") == 0 && strcmp(S(L(LVO_locale_GetLocaleStr, 36, 0, loc, 0, 0, 0)), "Oct") == 0;
  V(9) = L(LVO_locale_GetLocaleStr, 51, 0, loc, 0, 0, 0);               /* no string 51: NULL */
  /* Catalogs: none exist; the caller's own string comes back. */
  V(10) = L(LVO_locale_OpenCatalogA, 0, 0, loc, (u32)"dopus5.catalog", 0, 0);
  V(11) = L(LVO_locale_GetCatalogStr, 5, 0, 0, (u32)"built-in", 0, 0) == (u32)"built-in";
  /* Character classes (the negative controls are the zeros) and conversion. */
  V(12) = L(LVO_locale_IsAlpha, 'a', 0, loc, 0, 0, 0); V(13) = L(LVO_locale_IsAlpha, '1', 0, loc, 0, 0, 0);
  V(14) = L(LVO_locale_IsDigit, '7', 0, loc, 0, 0, 0); V(15) = L(LVO_locale_IsUpper, 'a', 0, loc, 0, 0, 0);
  V(16) = L(LVO_locale_IsAlNum, 'x', 0, 0, 0, 0, 0);                  /* locale NULL = the default */
  V(17) = L(LVO_locale_ConvToUpper, 'a', 0, loc, 0, 0, 0); V(18) = L(LVO_locale_ConvToLower, 0xC9, 0, loc, 0, 0, 0);
  V(19) = L(LVO_locale_IsSpace, ' ', 0, loc, 0, 0, 0) && !L(LVO_locale_IsSpace, 'x', 0, loc, 0, 0, 0);
  /* StrnCmp. */
  V(20) = (int)L(LVO_locale_StrnCmp, -1, 0, loc, (u32)"abc", (u32)"ABD", 0) < 0;
  V(21) = L(LVO_locale_StrnCmp, -1, 0, loc, (u32)"abc", (u32)"ABC", 0);          /* equal under SC_ASCII */
  V(22) = L(LVO_locale_StrnCmp, 2, 0, loc, (u32)"abc", (u32)"abd", 0);          /* only the first 2 compared */
  V(23) = (int)L(LVO_locale_StrnCmp, 3, 0, loc, (u32)"abc", (u32)"abd", 0) < 0;
  { char b[8]; V(24) = L(LVO_locale_StrConvert, 8, 0, loc, (u32)"abC", (u32)b, 0); V(25) = (u8)b[0] == 'A' && b[3] == 0; }
  /* FormatDate through a putChar hook. */
  {
    char out[96]; u32 cur = (u32)out; u32 hook[5] = { 0, 0, (u32)put_stub, 0, (u32)&cur };
    struct ds d = { DAYS_2024_02_29, 13 * 60 + 5, 7 * 50 };
    memset(out, 'x', sizeof out);
    V(26) = L(LVO_locale_FormatDate, 0, 0, loc, (u32)"%A %B %e %Y %Q:%M %p", (u32)&d, (u32)hook);
    V(27) = strcmp(out, "Thursday February 29 2024 1:05 pm") == 0 && cur == (u32)out + 34;   /* 33 characters + the final NUL */
    cur = (u32)out; L(LVO_locale_FormatDate, 0, 0, loc, (u32)"%c|%j %w %U|%y %m%%", (u32)&d, (u32)hook);
    V(28) = strcmp(out, "Thu Feb 29 13:05:07 2024|060 4 08|24 02%") == 0;
    struct ds mid = { DAYS_2024_02_29, 0, 0 };
    cur = (u32)out; L(LVO_locale_FormatDate, 0, 0, loc, (u32)"%I:%M %p", (u32)&mid, (u32)hook);
    V(29) = strcmp(out, "12:00 am") == 0;
    cur = (u32)out; L(LVO_locale_FormatDate, 0, 0, loc, 0, (u32)&d, (u32)hook);   /* NULL template: one NUL */
    V(30) = out[0] == 0 && cur == (u32)out + 1;
  }
  /* FormatString: %s %ld %d %lx widths, left alignment, zero fill, %lD grouping. */
  {
    char out[96]; u32 cur = (u32)out; u32 hook[5] = { 0, 0, (u32)put_stub, 0, (u32)&cur };
    struct __attribute__((packed)) { u32 s; u32 l; u16 w; u32 x; u16 a; u16 b; u16 c; u32 g; } args = { (u32)"n", 100000, (u16)-5, 100000, 123, 7, 42, 1234567 };
    u32 end = L(LVO_locale_FormatString, 0, 0, loc, (u32)"%s=%ld,%d,%lx,%5d|%-4d|%03d|%lD", (u32)&args, (u32)hook);
    V(31) = strcmp(out, "n=100000,-5,186A0,  123|7   |042|1,234,567") == 0;
    V(32) = end - (u32)&args;                                           /* 4+4+2+4+2+2+2+4 = 24 */
  }
  /* ParseDate through a getChar hook. */
  {
    u32 cur; u32 hook[5] = { 0, 0, (u32)get_stub, 0, (u32)&cur };
    struct ds d = { 0, 0, 0 };
    cur = (u32)"02/29/24 01:05 pm";
    V(33) = L(LVO_locale_ParseDate, 0, 0, loc, (u32)&d, (u32)"%m/%d/%y %I:%M %p", (u32)hook);
    V(34) = d.days; V(35) = d.minute; V(36) = d.tick;
    cur = (u32)"02/30/24 01:05 pm";                                      /* 2024-02-30 does not exist */
    V(37) = L(LVO_locale_ParseDate, 0, 0, loc, (u32)&d, (u32)"%m/%d/%y %I:%M %p", (u32)hook);
    cur = (u32)"02-29/24 01:05 pm";                                      /* wrong separator */
    V(38) = L(LVO_locale_ParseDate, 0, 0, loc, (u32)&d, (u32)"%m/%d/%y %I:%M %p", (u32)hook);
    cur = (u32)"Thursday, February 29 2024";
    V(39) = L(LVO_locale_ParseDate, 0, 0, loc, (u32)&d, (u32)"%A, %B %e %Y", (u32)hook);
    V(40) = d.days; V(41) = d.minute;
  }
  L(LVO_locale_CloseLocale, 0, 0, loc, 0, 0, 0);
  L(LVO_locale_CloseLocale, 0, 0, loc, 0, 0, 0);                        /* once too often: refused */
  L(LVO_locale_GetLocaleStr, 1, 0, 0x1234, 0, 0, 0);                    /* not a Locale: refused */
  L(LVO_locale_GetCatalogStr, 1, 0, 0x1234, 0, 0, 0);                   /* not a catalog: refused */
  L(LVO_locale_OpenLocale, 0, 0, (u32)"ENV:Sys/locale.prefs", 0, 0, 0);       /* a prefs file: refused */
  CLOSELIB(loc_lib);
}
