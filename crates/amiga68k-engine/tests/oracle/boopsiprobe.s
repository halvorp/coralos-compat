| Oracle program (written for this repo, MIT): MakeClass("probeclass","gadgetclass",0,8,0), AddClass, NewObjectA, then copy
| into `out` the evidence the engine must match on a real AROS m68k: the new IClass (60 bytes at out+0), the object's
| memory from its _Object header (12 + 72 + 8 = 92 bytes at out+64), the built-in gadgetclass (out+156), rootclass (out+216)
| and imageclass (out+276) IClass structs. Ends in a spin loop so the dump can be read. PC-relative only.
	.text
	.globl _start
_start:	move.l	4.w,%a6
	lea	iname(%pc),%a1
	moveq	#0,%d0
	jsr	-552(%a6)		| OpenLibrary intuition
	move.l	%d0,%a6
	lea	out(%pc),%a3
	lea	cname(%pc),%a0
	lea	sname(%pc),%a1
	sub.l	%a2,%a2
	moveq	#8,%d0
	moveq	#0,%d1
	jsr	-678(%a6)		| MakeClass
	move.l	%d0,%a5
	lea	disp(%pc),%a0
	move.l	%a0,8(%a5)
	move.l	%a5,%a0
	jsr	-684(%a6)		| AddClass
	sub.l	%a0,%a0
	lea	cname(%pc),%a1
	sub.l	%a2,%a2
	jsr	-636(%a6)		| NewObjectA
	move.l	%d0,%a4
	move.l	%a5,%a0
	lea	(%a3),%a1
	moveq	#59,%d0
c1:	move.b	(%a0)+,(%a1)+
	dbf	%d0,c1
	lea	-12(%a4),%a0
	lea	64(%a3),%a1
	moveq	#91,%d0
c2:	move.b	(%a0)+,(%a1)+
	dbf	%d0,c2
	lea	gname(%pc),%a0
	jsr	-672(%a6)		| FindClass gadgetclass
	move.l	%d0,%a0
	lea	156(%a3),%a1
	moveq	#59,%d0
c3:	move.b	(%a0)+,(%a1)+
	dbf	%d0,c3
	move.l	156+24(%a3),%a0		| its cl_Super = rootclass
	lea	216(%a3),%a1
	moveq	#59,%d0
c4:	move.b	(%a0)+,(%a1)+
	dbf	%d0,c4
	lea	iname2(%pc),%a0
	jsr	-672(%a6)		| FindClass imageclass
	move.l	%d0,%a0
	lea	276(%a3),%a1
	moveq	#59,%d0
c5:	move.b	(%a0)+,(%a1)+
	dbf	%d0,c5
spin:	bra.s	spin
disp:	move.l	24(%a0),%a0
	move.l	8(%a0),%a3
	jmp	(%a3)
iname:	.asciz	"intuition.library"
cname:	.asciz	"probeclass"
sname:	.asciz	"gadgetclass"
gname:	.asciz	"gadgetclass"
iname2:	.asciz	"imageclass"
	.even
out:	.space	340
