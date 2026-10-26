# Recursive fib(n) with a real PowerPC EABI stack frame (stwu, LR save at 4(old r1), callee-saved r30/r31). In: r3 = n. Out: r3.
    .text
    b main
fib:
    cmpwi 0,3,2
    blt base
    mflr 0
    stwu 1,-16(1)
    stw 0,20(1)
    stw 31,12(1)
    stw 30,8(1)
    mr 31,3
    addi 3,31,-1
    bl fib
    mr 30,3
    addi 3,31,-2
    bl fib
    add 3,3,30
    lwz 0,20(1)
    lwz 30,8(1)
    lwz 31,12(1)
    addi 1,1,16
    mtlr 0
    blr
base:
    blr
main:
    mflr 29
    bl fib
    mtlr 29
