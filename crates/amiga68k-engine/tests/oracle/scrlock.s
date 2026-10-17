| Oracle 2: LockPubScreen(NULL) (the Workbench screen), then a breakpoint-able NOP spot, then UnlockPubScreen.
	.text
_start:	move.l	4.w,%a6
	lea	iname(%pc),%a1
	moveq	#0,%d0
	jsr	-552(%a6)		| OpenLibrary
	move.l	%d0,%d7
	beq.s	out
	move.l	%d7,%a6
	sub.l	%a0,%a0
	jsr	-510(%a6)		| LockPubScreen(NULL)
	move.l	%d0,%d6
stop:	nop				| <- breakpoint here: d6 = struct Screen *
	sub.l	%a0,%a0
	move.l	%d6,%a1
	jsr	-516(%a6)		| UnlockPubScreen(NULL, screen)
	move.l	4.w,%a6
	move.l	%d7,%a1
	jsr	-414(%a6)		| CloseLibrary
out:	moveq	#0,%d0
	rts
iname:	.asciz	"intuition.library"
	.even
