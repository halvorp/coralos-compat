# Bench workload (self-modifying code): every iteration rewrites the instruction at `site` (addi r5,r5,2 <-> addi
# r5,r5,1) with stw + dcbst/sync/icbi/isync and then executes it, inside the same straight-line run as the store.
# In: r3 = n (> 0). Out: r5 = 2+1+2+1... (3n/2 for an even n).
    .text
    mflr 29
    bl here
here:
    mflr 12
    addi 12,12,site-here
    lis 8,0x38A5
    ori 8,8,1
    li 5,0
    mtctr 3
sloop:
    xori 8,8,3
    stw 8,0(12)
    dcbst 0,12
    sync
    icbi 0,12
    isync
site:
    nop
    bdnz sloop
    mtlr 29
