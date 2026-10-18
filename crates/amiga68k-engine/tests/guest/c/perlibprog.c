/* OpenLibrary of a disk library whose Open vector returns a DIFFERENT base per opener (AROS rellibs: stdc.library), then
 * CloseLibrary of that base. Written for this repo (MIT). */
#include "rt.h"
void _start(void) {
  u32 lib = OPENLIB("per.library");
  RES[0] = lib != 0;
  /* the opener's base has a JMP table of its own (MakeLibrary built it): func1 at -30 adds d0 + d1 */
  RES[1] = LVO(lib, -30, 7, 5, 0, 0, 0, 0);
  CLOSELIB(lib);                       /* before the fix: a refusal, "not an open library base" */
  CLOSELIB(lib + 6);                   /* NEGATIVE CONTROL: an address inside the library that is not a base: still refused by name */
  RES[2] = 1;
}
