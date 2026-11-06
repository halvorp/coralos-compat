/* Own declarations for the Objective-C smoke app: the root class and the one libSystem call it uses. */
#ifndef OSX_OBJC_MIN_H
#define OSX_OBJC_MIN_H
typedef signed char BOOL;
#define YES ((BOOL)1)
#define NO ((BOOL)0)
#define nil ((id)0)
typedef unsigned long NSUInteger;

__attribute__((objc_root_class)) @interface NSObject { Class isa; }
+ (id)alloc;
+ (id)new;
+ (Class)class;
- (id)init;
- (id)retain;
- (oneway void)release;
- (id)autorelease;
- (NSUInteger)retainCount;
- (void)dealloc;
- (Class)class;
- (BOOL)isKindOfClass:(Class)c;
- (BOOL)isMemberOfClass:(Class)c;
- (BOOL)respondsToSelector:(SEL)s;
- (id)performSelector:(SEL)s;
- (id)performSelector:(SEL)s withObject:(id)o;
@end

void *objc_autoreleasePoolPush(void);
void objc_autoreleasePoolPop(void *);
long write(int, const void *, unsigned long);
void exit(int);
#endif
