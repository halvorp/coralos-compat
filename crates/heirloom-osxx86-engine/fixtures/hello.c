/* osxx86 rung 1 fixture: the smallest app that talks to libSystem. Built on the host by the engine's tests with clang and ld64.lld. */
long write(int, const void *, unsigned long);

int main(int argc, char **argv) {
    static const char msg[] = "hello from macho\n";
    write(1, msg, sizeof msg - 1);
    return 7;
}
