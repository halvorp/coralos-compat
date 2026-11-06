/* osxx86 GUI fixture (rungs 2-3): one window, a text field and a button; the button's action writes into the field.
   Our own source, built on the host with clang + ld64.lld against own declarations; no Apple binary or header is involved. */
#include "osx_min.h"

@interface HelloController : NSObject {
@public
    NSTextField *field;
}
- (void)clicked:(id)sender;
@end

@implementation HelloController
- (void)clicked:(id)sender {
    [field setStringValue:[NSString stringWithUTF8String:"clicked"]];
}
@end

int main(int argc, char **argv) {
    NSApplication *app = [NSApplication sharedApplication];
    NSWindow *win = [[NSWindow alloc] initWithContentRect:NSMakeRect(100, 100, 320, 120) styleMask:15 backing:2 defer:NO];
    [win setTitle:[NSString stringWithUTF8String:"Hello"]];
    HelloController *ctl = [[HelloController alloc] init];
    NSTextField *field = [[NSTextField alloc] initWithFrame:NSMakeRect(20, 70, 280, 24)];
    ctl->field = field;
    NSButton *button = [[NSButton alloc] initWithFrame:NSMakeRect(20, 20, 100, 30)];
    [button setTitle:[NSString stringWithUTF8String:"Click"]];
    [button setTarget:ctl];
    [button setAction:@selector(clicked:)];
    [[win contentView] addSubview:field];
    [[win contentView] addSubview:button];
    [win makeKeyAndOrderFront:nil];
    [app run];
    return 0;
}
