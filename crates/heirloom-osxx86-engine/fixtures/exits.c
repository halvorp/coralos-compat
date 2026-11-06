/* An exit() deep in the call stack: nothing after it may run. */
void exit(int);
long write(int, const void *, unsigned long);

static void __attribute__((noinline)) leave(int code) {
    exit(code);
}

int main(int argc, char **argv) {
    static const char msg[] = "before exit\n";
    write(2, msg, sizeof msg - 1);
    leave(3);
    write(1, "after exit\n", 11);
    return 99;
}
