/* exec RawDoFmt with the classic string-copy PutChProc (`move.b d0,(a3)+`), and dos VPrintf/VFPrintf. Written for this repo (MIT). */
#include "rt.h"
#define S(p) ((const char *)(p))
#define V(i) (*(volatile u32 *)&RES[i])
void copy_proc(void);
asm(".section .text.zz,\"ax\"\n.align 2\n.globl copy_proc\ncopy_proc:\n\tmove.b %d0,(%a3)+\n\trts\n");
/* RawDoFmt(a0 = template, a1 = data, a2 = proc, a3 = proc data) -> end of the arguments */
#define RAWDOFMT(tpl, data, proc, pdata) ({ \
  register u32 _d0 asm("d0"); register u32 _a0 asm("a0") = (u32)(tpl); register u32 _a1 asm("a1") = (u32)(data); \
  register u32 _a2 asm("a2") = (u32)(proc); register u32 _a3 asm("a3") = (u32)(pdata); \
  asm volatile("move.l %%a6,-(%%sp)\n\tmove.l %[b],%%a6\n\tjsr %c[o](%%a6)\n\tmove.l (%%sp)+,%%a6" \
    : "=r"(_d0), "+r"(_a0), "+r"(_a1) : "r"(_a2), "r"(_a3), [b] "r"(SYSBASE), [o] "i"(LVO_exec_RawDoFmt) : "cc", "memory", "d1"); _d0; })
void _start(void) {
  char out[64];
  memset(out, 'x', sizeof out);
  struct __attribute__((packed)) { u16 w; u32 l; u32 x; u32 s; u16 c; u32 n1; u32 n2; u32 n3; u32 s2; } args = { 65530, 305419896, 305419896, (u32)"str", 'Z', (u32)-42, (u32)-42, (u32)-42, (u32)"string" };
  u32 end = RAWDOFMT("[%d|%ld|%lx|%s|%c|%5ld|%-5ld|%05ld|%.2s]", &args, copy_proc, out);
  V(0) = strcmp(out, "[-6|305419896|12345678|str|Z|  -42|-42  |00-42|st]") == 0;   /* NUL terminated by the proc's last call */
  V(1) = end - (u32)&args;                                            /* 2+4+4+4+2+4+4+4+4 = 32 */
  /* Negative control: a template with no directives reads no arguments. */
  V(2) = RAWDOFMT("plain", &args, copy_proc, out) == (u32)&args;
  V(3) = strcmp(out, "plain") == 0;
  u32 dos = OPENLIB("dos.library");
  u32 vargs[2] = { 7, (u32)"seven" };
  V(4) = LVO5(dos, LVO_dos_VPrintf, 0, (u32)"%ld=%s\n", (u32)vargs, 0, 0, 0, 0);   /* returns 8 */
  /* NULL PutChProc: refused (no serial port). */
  RAWDOFMT("x", &args, 0, out);
  CLOSELIB(dos);
}
