| Event Manager + Window Manager hit testing from assembly (flat program). Before running, the test pre-posts three events
| (keyDown, mouseDown message 7 at v=30 h=40, updateEvt), creates one window, and stores points at 0x3100.. :
|   0x3100 WindowPtr, 0x3104 a point in its content, 0x310C its go-away point, 0x3110 a menu-bar point, 0x3114 a desk point.
| Results are stored at 0x3000.. .
|   m68k-linux-gnu-as -m68020 evtest.s -o evtest.o && m68k-linux-gnu-objcopy -O binary -j .text evtest.o evtest.bin
        .include "traps.inc"
        .equ _EventAvail,   0xA971
        .equ _GetNextEvent, 0xA970
        .equ _TickCount,    0xA975
        .text
start:  lea     ev(%pc),%a2
        | EventAvail(mouseDownMask): TRUE and the event stays queued
        clr.w   -(%sp)
        move.w  #2,-(%sp)
        move.l  %a2,-(%sp)
        .short  _EventAvail
        move.w  (%sp)+,0x3000          | 0x0100 = TRUE
        move.w  (%a2),0x3002           | what = 1
        | GetNextEvent(mouseDownMask): removes it
        clr.w   -(%sp)
        move.w  #2,-(%sp)
        move.l  %a2,-(%sp)
        .short  _GetNextEvent
        move.w  (%sp)+,0x3004
        move.l  2(%a2),0x3008          | message = 7
        move.l  10(%a2),0x300C         | where (v, h) = (30, 40)
        | EventAvail(mouseDownMask) again: the keyDown and the update do not match the mask
        clr.w   -(%sp)
        move.w  #2,-(%sp)
        move.l  %a2,-(%sp)
        .short  _EventAvail
        move.w  (%sp)+,0x3010
        | FlushEvents(everyEvent, 0)
        move.l  #0x0000FFFF,%d0
        .short  _FlushEvents
        clr.w   -(%sp)
        move.w  #0xFFFF,-(%sp)
        move.l  %a2,-(%sp)
        .short  _EventAvail
        move.w  (%sp)+,0x3012          | nothing left
        | WaitNextEvent(sleep 0) with nothing queued: the null event, not a block
        clr.w   -(%sp)
        move.w  #0xFFFF,-(%sp)
        move.l  %a2,-(%sp)
        clr.l   -(%sp)
        clr.l   -(%sp)
        .short  _WaitNextEvent
        move.w  (%sp)+,0x3014
        move.w  (%a2),0x3016           | what = 0
        clr.l   -(%sp)
        .short  _TickCount
        move.l  (%sp)+,0x3018
        | hit testing
        clr.w   -(%sp)
        move.l  0x3104,-(%sp)
        pea     0x3108
        .short  _FindWindow
        move.w  (%sp)+,0x3020          | inContent = 3
        clr.w   -(%sp)
        move.l  0x3100,-(%sp)
        move.l  0x3104,-(%sp)
        .short  _TrackGoAway           | a content click is not in the go-away box
        move.w  (%sp)+,0x3022
        clr.w   -(%sp)
        move.l  0x3100,-(%sp)
        move.l  0x310C,-(%sp)
        .short  _TrackGoAway           | the go-away point is
        move.w  (%sp)+,0x3024
        clr.w   -(%sp)
        move.l  0x3110,-(%sp)
        pea     0x3118
        .short  _FindWindow
        move.w  (%sp)+,0x3026          | inMenuBar = 1
        clr.w   -(%sp)
        move.l  0x3114,-(%sp)
        pea     0x311C
        .short  _FindWindow
        move.w  (%sp)+,0x3028          | inDesk = 0
        | GetNextEvent(everyEvent) with nothing queued BLOCKS; the host posts an event, the call completes with TRUE
        clr.w   -(%sp)
        move.w  #0xFFFF,-(%sp)
        move.l  %a2,-(%sp)
        .short  _GetNextEvent
        move.w  (%sp)+,0x302A
        move.w  (%a2),0x302C
        .short  _ExitToShell
ev:     .space  16
