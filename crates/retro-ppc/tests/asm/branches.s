# Branch semantics: BO/BI decoding, CTR decrement, bl/blr, bctr/bctrl, conditional blr.
# Loaded at 0x1000 by the test harness; results are left in registers the test inspects.
    .text
    b start
sub:                    # 0x1004
    li 8,77
    blr
sub2:                   # 0x100c: returns early iff CR0[EQ]
    li 16,1
    beqlr
    li 16,2
    blr
start:
    # 1. bdnz: five iterations of +2; the branch falls through when CTR reaches 0
    li 3,0
    li 4,5
    mtctr 4
loop:
    addi 3,3,2
    bdnz loop
    mfctr 20
    # 2. bdz taken when the decremented CTR is 0
    li 5,0
    li 4,1
    mtctr 4
    bdz skip1
    li 5,99
skip1:
    # 3. beq taken, bne not taken
    cmpwi 0,3,10
    li 6,0
    beq 0,iseq
    li 6,1
iseq:
    li 7,0
    bne 0,wrong
    b ok7
wrong:
    li 7,1
ok7:
    # 4. bl sets LR to the next instruction, blr returns through it
    li 8,0
    bl sub
    mflr 21
    # 5. bctr
    li 10,0
    bl get
get:
    mflr 9
    addi 9,9,20
    mtctr 9
    bctr
    li 10,1
    li 11,5
    # 6. bctrl: CTR target, LR = next instruction
    li 8,0
    li 12,0x1004
    mtctr 12
    bctrl
    mflr 22
    # 7. bdnzt / bdnzf: CTR and a CR bit together
    li 4,3
    mtctr 4
    creqv 2,2,2
    li 13,0
l2:
    addi 13,13,1
    bdnzt 2,l2
    li 4,3
    mtctr 4
    li 14,0
l3:
    addi 14,14,1
    bdnzf 2,l3
    mfctr 23
    # 8. conditional blr, taken then not taken
    li 16,0
    bl sub2
    mr 24,16
    crxor 2,2,2
    bl sub2
    mr 25,16
    # restore the harness's exit address (EXIT = 0xF000) into LR for the appended blr
    lis 26,0
    ori 26,26,0xF000
    mtlr 26
