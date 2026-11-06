/* osxx86 rung 4 fixture: a window with a text field and a button; the button's action writes into the field. Built from our own
   source against our own declarations. With -DECHO the action instead copies the field's text into the window title and
   retitles the button (a retitled control is a new host control, so the old reference goes stale). */
#include "osx_min.h"

@interface HelloController : NSObject {
@public
    NSTextField *field;
    NSButton *button;
    NSWindow *win;
}
- (void)clicked:(id)sender;
@end

@implementation HelloController
- (void)clicked:(id)sender {
#ifdef ECHO
    [win setTitle:[field stringValue]];
    [button setTitle:[NSString stringWithUTF8String:"Done"]];
#else
    [field setStringValue:[NSString stringWithUTF8String:(sender == (id)button ? "clicked" : "wrong sender")]];
#endif
}
@end

int main(int argc, char **argv) {
    NSApplication *app = [NSApplication sharedApplication];
    NSWindow *win = [[NSWindow alloc] initWithContentRect:NSMakeRect(100, 100, 320, 120) styleMask:15 backing:2 defer:NO];
    [win setTitle:[NSString stringWithUTF8String:"Hello"]];
    HelloController *ctl = [[HelloController alloc] init];
    /* the button is created first and added last: creation order and publication order differ on purpose */
    NSButton *button = [[NSButton alloc] initWithFrame:NSMakeRect(20, 20, 100, 30)];
    NSTextField *field = [[NSTextField alloc] initWithFrame:NSMakeRect(20, 70, 280, 24)];
    ctl->field = field;
    ctl->button = button;
    ctl->win = win;
    [button setTitle:[NSString stringWithUTF8String:"Click"]];
    [button setTarget:ctl];
    [button setAction:@selector(clicked:)];
    [[win contentView] addSubview:field];
    [[win contentView] addSubview:button];
    [win makeKeyAndOrderFront:nil];
    [app run];
    write(1, "run returned\n", 13);
    return 0;
}
