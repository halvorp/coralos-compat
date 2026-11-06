/* osxx86 rung 2 fixture: an Objective-C app in our own source with no framework. It exercises what the runtime must do before any
   Cocoa class can exist: class and metaclass realization, instance and class methods, super calls, categories, +initialize,
   structure and floating-point returns, synthesized object properties, autorelease pools, performSelector and introspection.
   Every check writes "ok <label>" or "FAIL <label>"; the exit status is the number of failures. */
#include "osx_objc_min.h"

static int failures;
static void out(const char *s) {
    unsigned long n = 0;
    while (s[n]) n++;
    write(1, s, n);
}
static void number(long v) {
    char b[24]; int i = 24; int neg = v < 0; unsigned long u = neg ? -v : v;
    b[--i] = 0;
    do { b[--i] = '0' + u % 10; u /= 10; } while (u);
    if (neg) b[--i] = '-';
    out(b + i);
}
static void check(int cond, const char *label) {
    if (!cond) failures++;
    out(cond ? "ok " : "FAIL ");
    out(label);
    out("\n");
}

typedef struct { double x, y, w, h; } Rect;

@interface Counter : NSObject {
@public
    long n;
    double scale;
}
@property (retain) NSObject *child;
+ (Counter *)counterWithStart:(long)start;
- (void)increment;
- (long)total;
- (double)scaled:(double)factor plus:(double)extra;
- (Rect)frameWithOrigin:(double)x;
- (long)sumA:(long)a b:(long)b c:(long)c d:(long)d e:(long)e f:(long)f g:(long)g h:(long)h;
@end

static int initialized;
static int deallocs;

@implementation Counter
@synthesize child;
+ (void)initialize { if (self == [Counter class]) initialized++; }
+ (Counter *)counterWithStart:(long)start {
    Counter *c = [[[self alloc] init] autorelease];
    c->n = start;
    c->scale = 1.5;
    return c;
}
- (void)increment { n++; }
- (long)total { return n; }
- (double)scaled:(double)factor plus:(double)extra { return scale * factor + extra; }
- (Rect)frameWithOrigin:(double)x { Rect r = { x, x + 1, x + 2, x + 3 }; return r; }
- (long)sumA:(long)a b:(long)b c:(long)c d:(long)d e:(long)e f:(long)f g:(long)g h:(long)h { return a + 10*b + 100*c + 1000*d + 10000*e + 100000*f + 1000000*g + 10000000*h; }
- (void)dealloc { deallocs++; [child release]; [super dealloc]; }
@end

@interface SubCounter : Counter
- (void)increment;
@end
@implementation SubCounter
- (void)increment { [super increment]; [super increment]; n += 10; }
@end

@interface Counter (Extra)
- (long)doubled;
@end
@implementation Counter (Extra)
- (long)doubled { return n * 2; }
@end

int main(int argc, char **argv) {
    check(initialized == 0, "initialize has not run before the first message");
    void *pool = objc_autoreleasePoolPush();
    {
        Counter *c = [Counter counterWithStart:5];
        check(initialized == 1, "initialize ran once on the first message");
        [c increment];
        check([c total] == 6, "instance method and ivar");
        check([c doubled] == 12, "category method");
        check([c scaled:2.0 plus:0.25] == 3.25, "double argument and return");
        Rect r = [c frameWithOrigin:10];
        check(r.x == 10 && r.y == 11 && r.w == 12 && r.h == 13, "structure return through objc_msgSend_stret");
        check([c sumA:1 b:2 c:3 d:4 e:5 f:6 g:7 h:8] == 87654321, "eight arguments, two on the stack");
        check([c isKindOfClass:[Counter class]] && [c isKindOfClass:[NSObject class]], "isKindOfClass");
        check(![c isMemberOfClass:[NSObject class]] && [c isMemberOfClass:[Counter class]], "isMemberOfClass");
        check([c respondsToSelector:@selector(increment)] && ![c respondsToSelector:@selector(nothing)], "respondsToSelector");
        [c performSelector:@selector(increment)];
        check([c total] == 7, "performSelector");

        SubCounter *s = [[SubCounter alloc] init];
        [s increment];
        check([s total] == 12, "super calls and a subclass override");
        check([s isKindOfClass:[Counter class]], "subclass is a kind of its superclass");
        [s release];

        NSObject *kid = [[NSObject alloc] init];
        c.child = kid;
        check(c.child == kid, "synthesized retain property");
        check([kid retainCount] == 3, "the property retains and the getter autoreleases");
        [kid release];
    }
    objc_autoreleasePoolPop(pool);
    check(deallocs >= 2, "autorelease pool pop deallocated the counter and the subclass instance");
    out("done failures=");
    number(failures);
    out("\n");
    return failures;
}
