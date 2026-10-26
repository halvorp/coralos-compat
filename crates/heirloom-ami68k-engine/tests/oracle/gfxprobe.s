| Oracle program (run on real AROS m68k in Copperline AND in this engine): draw on a private 64x32x3 bitmap with
| InitRastPort defaults, then copy the RastPort struct (100 bytes) and every pixel (ReadPixel) into `out` and spin.
| PC-relative only. out: rp (100 bytes) then 64*32 pixel bytes.
	.text
	bra.w	start
out:	.space	100+2048
gname:	.asciz	"graphics.library"
txt:	.asciz	"AB"
	.even
start:	move.l	4.w,%a6
	lea	gname(%pc),%a1
	moveq	#0,%d0
	jsr	-552(%a6)		| OpenLibrary graphics
	move.l	%d0,%a6
	move.l	%a6,%d7
	moveq	#64,%d0
	moveq	#32,%d1
	moveq	#3,%d2
	moveq	#1,%d3			| BMF_CLEAR
	sub.l	%a0,%a0
	jsr	-918(%a6)		| AllocBitMap
	move.l	%d0,%d6
	lea	out(%pc),%a5		| the RastPort lives in `out`
	move.l	%a5,%a1
	jsr	-198(%a6)		| InitRastPort
	move.l	%d6,4(%a5)		| rp->BitMap
	move.l	%a5,%a1
	moveq	#2,%d0
	jsr	-342(%a6)		| SetAPen 2
	move.l	%a5,%a1
	moveq	#2,%d0
	moveq	#2,%d1
	moveq	#9,%d2
	moveq	#5,%d3
	jsr	-306(%a6)		| RectFill(2,2,9,5)
	move.l	%a5,%a1
	moveq	#1,%d0
	jsr	-342(%a6)		| SetAPen 1
	move.l	%a5,%a1
	moveq	#3,%d0
	jsr	-348(%a6)		| SetBPen 3
	move.l	%a5,%a1
	moveq	#4,%d0
	moveq	#20,%d1
	jsr	-240(%a6)		| Move(4,20)
	move.l	%a5,%a1
	lea	txt(%pc),%a0
	moveq	#2,%d0
	jsr	-60(%a6)		| Text("AB")
	move.l	%a5,%a1
	moveq	#4,%d0
	jsr	-342(%a6)		| SetAPen 4
	move.l	%a5,%a1
	moveq	#1,%d0
	moveq	#28,%d1
	jsr	-240(%a6)		| Move(1,28)
	move.l	%a5,%a1
	moveq	#30,%d0
	moveq	#26,%d1
	jsr	-246(%a6)		| Draw(30,26)
	move.l	%a5,%a1
	moveq	#8,%d0
	moveq	#31,%d1
	jsr	-246(%a6)		| Draw(8,31)
	lea	100(%a5),%a4		| pixel dump
	moveq	#0,%d5			| y
yl:	moveq	#0,%d4			| x
xl:	move.l	%a5,%a1
	move.l	%d4,%d0
	move.l	%d5,%d1
	jsr	-318(%a6)		| ReadPixel
	move.b	%d0,(%a4)+
	addq.l	#1,%d4
	cmp.l	#64,%d4
	blt.s	xl
	addq.l	#1,%d5
	cmp.l	#32,%d5
	blt.s	yl
spin:	bra.s	spin
