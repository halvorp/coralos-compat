# Bench workload (floating point): a damped fused-multiply-add recurrence through memory, single and double.
# In: r3 = n (> 0), r4 = buffer with doubles a, b, c at 0, 8, 16. Out: f4, and 32(r4).
    .text
    lfd 1,0(4)
    lfd 2,8(4)
    lfd 3,16(4)
    fmr 4,1
    mtctr 3
floop:
    fmadd 4,4,2,3
    fmul 5,4,1
    fsub 6,5,3
    fadds 7,6,2
    fabs 7,7
    stfd 7,24(4)
    lfd 8,24(4)
    fadd 4,8,1
    fmul 4,4,2
    bdnz floop
    stfd 4,32(4)
