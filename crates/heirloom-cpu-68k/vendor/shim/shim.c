/* Freestanding stubs for the libc symbols Musashi's FPU/PMMU debug paths reference. The 68020 integer
   core the Amiga engine runs never reaches them (no FPU/MMU opcode is in a 68020 Amiga program we run);
   each stub fails closed (traps / returns 0) rather than pretending. memcpy comes from compiler-builtins. */
#include <stdarg.h>
typedef struct _F FILE;
FILE *stderr = 0;
void exit(int c) { (void)c; for (;;) __builtin_trap(); }
void longjmp(long *b, int v) { (void)b; (void)v; for (;;) __builtin_trap(); }
int vfprintf(FILE *f, const char *s, va_list a) { (void)f; (void)s; (void)a; return 0; }
int fprintf(FILE *f, const char *s, ...) { (void)f; (void)s; return 0; }
int sprintf(char *d, const char *s, ...) { (void)s; if (d) *d = 0; return 0; }
int sscanf(const char *d, const char *s, ...) { (void)d; (void)s; return 0; }
double sin(double x) { (void)x; return 0.0; }
double cos(double x) { (void)x; return 1.0; }
