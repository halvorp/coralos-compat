# Bench workload (native callouts): every iteration stores a word, calls the native bound at r6 with (r3 = buffer,
# r4 = 32) through bctrl -- the harness's native zeroes r4 bytes at r3, as BlockZero does -- and reads the word back.
# In: r3 = n (> 0), r4 = buffer, r6 = native address. Out: r5 = n.
    .text
    mflr 29
    mr 7,3
    mr 11,4
    li 5,0
cloop:
    stw 7,0(11)
    mr 3,11
    li 4,32
    mtctr 6
    bctrl
    lwz 8,0(11)
    add 5,5,8
    addi 5,5,1
    addic. 7,7,-1
    bne cloop
    mtlr 29
