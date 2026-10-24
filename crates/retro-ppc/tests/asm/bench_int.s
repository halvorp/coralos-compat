# Bench workload (integer ALU): an xorshift/multiply mix, r3 iterations. In: r3 = n (> 0). Out: r5, r6.
    .text
    lis 5,0x1234
    ori 5,5,0x5678
    li 6,0
    mtctr 3
iloop:
    slwi 7,5,13
    xor 5,5,7
    srwi 7,5,17
    xor 5,5,7
    slwi 7,5,5
    xor 5,5,7
    mullw 8,5,6
    add 6,6,8
    rlwinm 9,6,3,0,28
    subf 6,9,6
    addi 6,6,7
    andi. 10,5,0xFF
    or 6,6,10
    bdnz iloop
