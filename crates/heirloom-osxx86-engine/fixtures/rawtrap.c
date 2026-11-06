/* An app that issues a Darwin BSD write itself with a raw SYSCALL instruction: the loader must refuse it before it runs. */
int main(void) {
    long ret;
    __asm__ volatile("syscall" : "=a"(ret) : "a"(0x2000004L), "D"(1L), "S"("x"), "d"(1L) : "rcx", "r11", "memory");
    return (int)ret;
}
