# Self-modifying code WITHOUT icbi: every iteration calls the native bound at r6 with r3 = the address of `site`; the
# host rewrites that instruction (nop -> addi r5,r5,2, then toggling addi r5,r5,2 <-> addi r5,r5,1) through guest
# memory, as a Toolbox call loading code would, and the program then executes it. In: r3 = n (> 0). Out: r5 = 3n/2.
    .text
    mflr 29
    bl here
here:
    mflr 12
    addi 12,12,site-here
    li 5,0
    mr 7,3
nloop:
    mr 3,12
    mtctr 6
    bctrl
site:
    nop
    addic. 7,7,-1
    bne nloop
    mtlr 29
