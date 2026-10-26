typedef long jmp_buf[1];
void longjmp(jmp_buf, int);
#define setjmp(b) 0
