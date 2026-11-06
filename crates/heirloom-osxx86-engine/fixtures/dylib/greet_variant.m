/* The same library without greet_add: a stale library the app was not linked against. */
#include "../objc/osx_objc_min.h"
int greet_data = 42;
int greet_inits;
int greet_loads;
@interface Greeter : NSObject
+ (long)answer;
@end
@implementation Greeter
+ (long)answer { return 7; }
@end
