| CODE 1 of the test application: the classic init sequence, a menu bar built by CODE 2 (through the jump table, so
| _LoadSeg runs), a window from a WIND resource, and a WaitNextEvent loop that quits on a close box or File > Quit.
|   m68k-linux-gnu-as -m68020 main.s -o main.o && m68k-linux-gnu-objcopy -O binary -j .text main.o code1.bin
| A5 world: the QuickDraw globals fill A5-206..A5 (thePort at A5-4); the application's own globals are below them.
        .include "traps.inc"
        .equ WINDOW,   -210            | WindowPtr of the main window
        .equ EVENT,    -240            | EventRecord (16 bytes): what 0, message 2, when 6, where 10, modifiers 14
        .equ WHICHWIN, -250            | FindWindow's VAR window
        .equ OTHERS,   -252            | count of menu choices other than Quit
        .equ JT_MENUS, 32+8*1+2        | jump-table entry 1 (CODE 2's SetupMenus), +2 = the word callers jsr to
        .text
main:   pea     -4(%a5)                | &qd.thePort
        .short  _InitGraf
        .short  _InitFonts
        .short  _InitWindows
        .short  _InitMenus
        .short  _TEInit
        clr.l   -(%sp)
        .short  _InitDialogs
        .short  _InitCursor
        move.l  #0x0000FFFF,%d0        | FlushEvents(everyEvent, 0)
        .short  _FlushEvents
        jsr     JT_MENUS(%a5)          | SetupMenus, in CODE 2: unloaded -> _LoadSeg -> patched -> jmp
        clr.l   -(%sp)
        move.w  #128,-(%sp)            | WIND 128
        clr.l   -(%sp)                 | wStorage = NIL
        move.l  #-1,-(%sp)             | behind = in front
        .short  _GetNewWindow
        move.l  (%sp)+,WINDOW(%a5)
loop:   clr.w   -(%sp)                 | Boolean result
        move.w  #0xFFFF,-(%sp)         | everyEvent
        pea     EVENT(%a5)
        move.l  #60,-(%sp)             | sleep
        clr.l   -(%sp)                 | mouseRgn
        .short  _WaitNextEvent
        tst.b   (%sp)+                 | pop the Boolean (a byte on A7 steps by 2)
        beq.s   loop
        cmp.w   #1,EVENT(%a5)          | what == mouseDown?
        bne.s   loop
        clr.w   -(%sp)
        move.l  EVENT+10(%a5),-(%sp)   | where
        pea     WHICHWIN(%a5)
        .short  _FindWindow
        move.w  (%sp)+,%d0
        cmp.w   #1,%d0                 | inMenuBar
        beq.s   menu
        cmp.w   #6,%d0                 | inGoAway
        bne.s   loop
        clr.w   -(%sp)
        move.l  WHICHWIN(%a5),-(%sp)
        move.l  EVENT+10(%a5),-(%sp)
        .short  _TrackGoAway
        tst.b   (%sp)+
        beq.s   loop
        bra.s   quit
menu:   clr.l   -(%sp)
        move.l  EVENT+10(%a5),-(%sp)
        .short  _MenuSelect
        move.l  (%sp)+,%d0
        beq.s   loop                   | nothing chosen
        cmp.l   #(129<<16)+2,%d0       | menu 129 (File), item 2 (Quit)
        beq.s   quit
        addq.w  #1,OTHERS(%a5)
        bra.s   loop
quit:   move.l  WINDOW(%a5),-(%sp)
        .short  _DisposeWindow
        .short  _ExitToShell
