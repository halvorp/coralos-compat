/* The app that links libgreet through @rpath and checks everything the second image provides. */
#include "../objc/osx_objc_min.h"

extern int greet_data, greet_inits, greet_loads;
long greet_add(long a, long b);

@interface Greeter : NSObject
+ (long)answer;
@end

static int failures;
static void out(const char *s) { unsigned long n = 0; while (s[n]) n++; write(1, s, n); }
static void check(int cond, const char *label) { if (!cond) failures++; out(cond ? "ok " : "FAIL "); out(label); out("\n"); }

int main(int argc, char **argv) {
    check(greet_add(2, 3) == 5, "a function of the library");
    check(greet_data == 42, "a data symbol of the library");
    check(greet_inits == 1, "the library's constructor ran once before main");
    check(greet_loads == 1, "the library's +load ran once before main");
    check([Greeter answer] == 7, "a class of the library");
    out("done failures="); out(failures ? "some" : "0"); out("\n");
    return failures;
}
