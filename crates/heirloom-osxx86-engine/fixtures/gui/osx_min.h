/* Own minimal declarations of the Cocoa surface OsxGuiHello uses (no SDK on the build host, and none is shipped): only
   what the app calls. The classes are provided by the engine's os-osx.library at bind time. */
#ifndef OSX_MIN_H
#define OSX_MIN_H
typedef signed char BOOL;
#define YES ((BOOL)1)
#define NO ((BOOL)0)
#define nil ((id)0)
typedef unsigned long NSUInteger;
typedef long NSInteger;
typedef double CGFloat;
typedef struct { CGFloat x, y; } NSPoint;
typedef struct { CGFloat width, height; } NSSize;
typedef struct { NSPoint origin; NSSize size; } NSRect;
static inline NSRect NSMakeRect(CGFloat x, CGFloat y, CGFloat w, CGFloat h) { NSRect r = {{x, y}, {w, h}}; return r; }

__attribute__((objc_root_class)) @interface NSObject { Class isa; }
+ (id)alloc;
- (id)init;
@end

@interface NSString : NSObject
+ (NSString *)stringWithUTF8String:(const char *)s;
- (const char *)UTF8String;
@end

@interface NSView : NSObject
- (void)addSubview:(NSView *)v;
@end

@interface NSControl : NSView
- (void)setTarget:(id)target;
- (void)setAction:(SEL)action;
- (NSString *)stringValue;
- (void)setStringValue:(NSString *)s;
@end

@interface NSButton : NSControl
- (id)initWithFrame:(NSRect)frame;
- (void)setTitle:(NSString *)title;
@end

@interface NSTextField : NSControl
- (id)initWithFrame:(NSRect)frame;
@end

@interface NSWindow : NSObject
- (id)initWithContentRect:(NSRect)r styleMask:(NSUInteger)style backing:(NSUInteger)backing defer:(BOOL)flag;
- (void)setTitle:(NSString *)title;
- (NSView *)contentView;
- (void)makeKeyAndOrderFront:(id)sender;
@end

@interface NSApplication : NSObject
+ (NSApplication *)sharedApplication;
- (void)run;
- (void)terminate:(id)sender;
@end
#endif
