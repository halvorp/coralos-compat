/* dos.library variables: local (pr_LocalVars), alias, global ENV:, flags (written for this repo, MIT). */
#include "rt.h"
#define GVF_GLOBAL_ONLY 0x100
#define GVF_LOCAL_ONLY 0x200
#define GVF_BINARY_VAR 0x400
#define GVF_SAVE_VAR 0x1000
#define LV_ALIAS 1
#define DOS(off, d1, d2, d3, d4) LVO5(dos, off, 0, d1, d2, d3, d4, 0, 0)
#define SETVAR(n, v, f) DOS(-900, n, v, -1, f)
#define GETVAR(n, buf, sz, f) DOS(-906, n, buf, sz, f)
#define IOERR() DOS(-132, 0, 0, 0, 0)
#define S(p) ((const char *)(p))
void _start(void) {
  u32 dos = OPENLIB("dos.library");
  char buf[32];
  RES[0] = SETVAR("Foo", "bar\nbaz", 0);
  memset(buf, 'x', sizeof buf);
  RES[1] = GETVAR("foo", buf, 32, 0);                      /* text var: stops at the newline -> 3 */
  RES[2] = strcmp(buf, "bar") == 0;
  RES[3] = IOERR();                                        /* IoErr = the full length, 7 */
  RES[4] = GETVAR("FOO", buf, 32, GVF_BINARY_VAR);         /* binary: all 7 bytes */
  RES[5] = strcmp(buf, "bar\nbaz") == 0;
  RES[6] = GETVAR("foo", buf, 3, GVF_BINARY_VAR);          /* does not fit: -1, truncated + terminated */
  RES[7] = IOERR();                                        /* ERROR_BUFFER_OVERFLOW 303 */
  RES[8] = strcmp(buf, "ba") == 0;
  RES[9] = GETVAR("nope", buf, 32, 0);                     /* -1 */
  RES[10] = IOERR();                                       /* 205 */
  RES[11] = SETVAR("ll", "list", LV_ALIAS);
  RES[12] = GETVAR("ll", buf, 32, LV_ALIAS);               /* 4 */
  RES[13] = strcmp(buf, "list") == 0;
  RES[14] = GETVAR("ll", buf, 32, 0);                      /* an alias is not a variable: -1 */
  RES[15] = SETVAR("G", "glob", GVF_GLOBAL_ONLY | GVF_SAVE_VAR);
  RES[16] = GETVAR("g", buf, 32, 0);                       /* found in ENV: */
  RES[17] = strcmp(buf, "glob") == 0;
  RES[18] = GETVAR("G", buf, 32, GVF_LOCAL_ONLY);          /* local only: not found */
  RES[19] = SETVAR("G", "loc", 0);                         /* a local that shadows it */
  GETVAR("G", buf, 32, 0);
  RES[20] = strcmp(buf, "loc") == 0;
  GETVAR("G", buf, 32, GVF_GLOBAL_ONLY);
  RES[21] = strcmp(buf, "glob") == 0;
  u32 node = DOS(-918, "foo", 0, 0, 0);                    /* FindVar */
  RES[22] = node != 0;
  RES[23] = strcmp(S(rd32((void *)(node + 16))), "bar\nbaz") == 0 && rd32((void *)(node + 20)) == 7 && *(u8 *)(node + 8) == 0;
  RES[24] = strcmp(S(*(u32 *)(node + 10)), "Foo") == 0;    /* ln_Name keeps the spelling */
  RES[25] = DOS(-918, "foo", 1, 0, 0);                     /* no alias foo: NULL */
  u32 proc = LVO(SYSBASE, -294, 0, 0, 0, 0, 0, 0);         /* FindTask(NULL); pr_LocalVars at +208, alphabetical */
  u32 n = rd32((void *)(proc + 208)), i = 0;
  while (rd32((void *)n) != 0 && i < 8) { RES[26 + i++] = *(u8 *)rd32((void *)(n + 10)); n = rd32((void *)n); }
  RES[34] = i;
  RES[35] = DOS(-912, "foo", 0, 0, 0);                     /* DeleteVar */
  RES[36] = GETVAR("foo", buf, 32, GVF_LOCAL_ONLY);        /* gone */
  RES[37] = DOS(-912, "foo", 0, 0, 0);                     /* twice: FALSE */
  RES[38] = IOERR();                                       /* 205 */
  RES[39] = DOS(-912, "G", GVF_GLOBAL_ONLY, 0, 0);         /* removes ENV:G */
  RES[40] = GETVAR("G", buf, 32, GVF_GLOBAL_ONLY);         /* -1 */
  RES[41] = GETVAR("G", buf, 0, 0);                        /* size 0 is a bad number */
  RES[42] = IOERR();                                       /* 115 */
  CLOSELIB(dos);
}
