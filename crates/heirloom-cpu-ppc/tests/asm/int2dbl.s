# The classic Mac/PPC int -> double idiom through memory, then x*1.5, then fctiwz back. In: r3 = int, r10 = scratch (>= 16 bytes,
# bytes 8..15 pre-set by the caller to 0x4330000080000000). Out: fr1 = (double)r3, fr3 = trunc(1.5 * r3) in the low word at 16(r10).
    .text
    lis 4,0x4330
    xoris 5,3,0x8000
    stw 4,0(10)
    stw 5,4(10)
    lfd 1,0(10)
    lfd 2,8(10)
    fsub 1,1,2
    lfd 4,24(10)           # 1.5
    fmul 3,1,4
    fctiwz 3,3
    stfd 3,16(10)
