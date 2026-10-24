# Bench workload (branches and calls): a loop whose body takes data-dependent conditional branches and calls a leaf
# through bl and through bctrl. In: r3 = n (> 0). Out: r5.
    .text
    b start
leaf:
    addi 5,5,3
    blr
start:
    mflr 29
    bl here
here:
    mflr 12
    addi 12,12,leaf-here
    li 5,0
    mr 7,3
bloop:
    andi. 8,7,3
    beq viabl
    cmpwi 0,8,2
    blt viactr
    addi 5,5,1
    b join
viabl:
    bl leaf
    b join
viactr:
    mtctr 12
    bctrl
join:
    addic. 7,7,-1
    bne bloop
    mtlr 29
