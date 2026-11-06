/* osxx86 rung 3 fixture: the smallest GUI app. One window with a title, ordered front, then the run loop. */
#include "osx_min.h"

#ifndef STYLE
#define STYLE 15
#endif

int main(int argc, char **argv) {
    NSApplication *app = [NSApplication sharedApplication];
    NSWindow *win = [[NSWindow alloc] initWithContentRect:NSMakeRect(100, 100, 320, 120) styleMask:STYLE backing:2 defer:NO];
    [win setTitle:[NSString stringWithUTF8String:"Hello"]];
    [win makeKeyAndOrderFront:nil];
    [app run];
    write(1, "run returned\n", 13);
    return 0;
}
