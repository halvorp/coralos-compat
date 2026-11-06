/* osxx86 rung 5 fixture: a dylib the app links through @rpath. It exports a function, a data symbol, an Objective-C class with a
   +load method, and runs a constructor: everything the loader's dyld subset must do for a second image. */
#include "../objc/osx_objc_min.h"

int greet_data = 42;
int greet_inits;
int greet_loads;

/* the asm barrier keeps the optimizer from evaluating the constructor at compile time (it did, and the check below was vacuous) */
__attribute__((constructor)) static void greet_init(void) {
    __asm__ volatile("" ::: "memory");
    greet_inits++;
}

long evil(void) { long r; __asm__ volatile("syscall" : "=a"(r) : "a"(0x2000004L) : "rcx", "r11", "memory"); return r; }

long greet_add(long a, long b) { return a + b; }

@interface Greeter : NSObject
+ (long)answer;
@end

@implementation Greeter
+ (void)load { greet_loads++; }
+ (long)answer { return 7; }
@end
