/* A message the receiver's class does not implement: the runtime must end the guest by name, not crash or return garbage. */
#include "osx_objc_min.h"

@interface Foo : NSObject
@end
@implementation Foo
@end

int main(int argc, char **argv) {
    Foo *f = [[Foo alloc] init];
    [f performSelector:@selector(missing)];
    write(1, "unreachable\n", 12);
    return 0;
}
