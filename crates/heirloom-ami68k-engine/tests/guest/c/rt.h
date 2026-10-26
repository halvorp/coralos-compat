/* Minimal freestanding runtime for the C guest fixtures (written for this repo, MIT). A fixture is one .c file whose
 * FIRST function is _start; build-c.sh compiles it with m68k-linux-gnu-gcc into a raw position-independent blob
 * (-mpcrel: no relocations; gcc keeps a6 as its frame pointer, so LVO() saves and restores it around the call) that the Rust tests wrap in a Hunk file. Results go to RES[] at a fixed guest address the
 * tests read back; every library call goes through LVO(), a plain `move.l base,a6; jsr off(a6)` like compiled Amiga code. */
#include "lvo.h"
typedef unsigned int u32; typedef unsigned short u16; typedef unsigned char u8; typedef int i32; typedef short i16;
#define RES ((volatile u32 *)0x1F000)
#define LVO6(base, off, D0, D1, D2, D3, D4, D5, A0, A1) ({ \
  register u32 _d0 asm("d0") = (u32)(D0); register u32 _d1 asm("d1") = (u32)(D1); \
  register u32 _d2 asm("d2") = (u32)(D2); register u32 _d3 asm("d3") = (u32)(D3); register u32 _d4 asm("d4") = (u32)(D4); \
  register u32 _d5 asm("d5") = (u32)(D5); register u32 _a0 asm("a0") = (u32)(A0); register u32 _a1 asm("a1") = (u32)(A1); \
  asm volatile("move.l %%a6,-(%%sp)\n\tmove.l %[b],%%a6\n\tjsr %c[o](%%a6)\n\tmove.l (%%sp)+,%%a6" : "+r"(_d0), "+r"(_d1), "+r"(_a0), "+r"(_a1) \
               : "r"(_d2), "r"(_d3), "r"(_d4), "r"(_d5), [b] "r"((u32)(base)), [o] "i"(off) : "cc", "memory"); _d0; })
#define LVO5(base, off, D0, D1, D2, D3, D4, A0, A1) LVO6(base, off, D0, D1, D2, D3, D4, 0, A0, A1)
#define LVO(base, off, D0, D1, D2, D3, A0, A1) LVO5(base, off, D0, D1, D2, D3, 0, A0, A1)
#define SYSBASE (*(volatile u32 *)4)
#define OPENLIB(name) LVO(SYSBASE, LVO_exec_OpenLibrary, 0, 0, 0, 0, 0, name)
#define CLOSELIB(b) LVO(SYSBASE, LVO_exec_CloseLibrary, 0, 0, 0, 0, 0, b)
#define COLD __attribute__((section(".text.zz")))
COLD void *memset(void *d, int c, unsigned long n) { u8 *p = d; while (n--) *p++ = (u8)c; return d; }
COLD void *memcpy(void *d, const void *s, unsigned long n) { u8 *p = d; const u8 *q = s; while (n--) *p++ = *q++; return d; }
COLD int memcmp(const void *a, const void *b, unsigned long n) { const u8 *p = a, *q = b; while (n--) { if (*p != *q) return *p - *q; p++; q++; } return 0; }
COLD unsigned long strlen(const char *s) { unsigned long n = 0; while (s[n]) n++; return n; }
COLD int strcmp(const char *a, const char *b) { while (*a && *a == *b) { a++; b++; } return (u8)*a - (u8)*b; }
COLD static inline u32 rd32(const void *p) { return *(const u32 *)p; }
/* A library call with every argument register: d0-d3, a0-a3 (a2/a3 and d2/d3 are inputs; d0, d1, a0, a1 may be trashed). */
#define LVOX(base, off, d0, d1, d2, d3, a0, a1, a2, a3) ({ \
  register u32 _d0 asm("d0") = (u32)(d0); register u32 _d1 asm("d1") = (u32)(d1); register u32 _d2 asm("d2") = (u32)(d2); register u32 _d3 asm("d3") = (u32)(d3); \
  register u32 _a0 asm("a0") = (u32)(a0); register u32 _a1 asm("a1") = (u32)(a1); register u32 _a2 asm("a2") = (u32)(a2); register u32 _a3 asm("a3") = (u32)(a3); \
  asm volatile("move.l %%a6,-(%%sp)\n\tmove.l %[b],%%a6\n\tjsr %c[o](%%a6)\n\tmove.l (%%sp)+,%%a6" : "+r"(_d0), "+r"(_d1), "+r"(_a0), "+r"(_a1) \
               : "r"(_d2), "r"(_d3), "r"(_a2), "r"(_a3), [b] "r"((u32)(base)), [o] "i"(off) : "cc", "memory"); _d0; })
