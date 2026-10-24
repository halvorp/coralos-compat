# Bench workload (memory): per pass, copy 256 words with lwzu/stwu, then walk the copy with lwz/lbz/lhz/stw.
# In: r3 = passes (> 0), r4 = a 2 KiB buffer. Out: r5.
    .text
    li 5,0
    mr 11,3
pass:
    addi 9,4,-4
    addi 10,4,1020
    li 7,256
    mtctr 7
copy:
    lwzu 8,4(9)
    addi 8,8,1
    stwu 8,4(10)
    bdnz copy
    li 7,64
    mtctr 7
    addi 9,4,1024
walk:
    lwz 8,0(9)
    lbz 6,5(9)
    lhz 12,10(9)
    add 5,5,8
    add 5,5,6
    xor 5,5,12
    stw 5,12(9)
    addi 9,9,16
    bdnz walk
    addic. 11,11,-1
    bne pass
