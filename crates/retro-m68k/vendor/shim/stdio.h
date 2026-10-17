#include <stdarg.h>
typedef struct _F FILE; extern FILE *stderr;
int vfprintf(FILE*, const char*, va_list); int fprintf(FILE*, const char*, ...);
int sscanf(const char*, const char*, ...); int sprintf(char*, const char*, ...);
