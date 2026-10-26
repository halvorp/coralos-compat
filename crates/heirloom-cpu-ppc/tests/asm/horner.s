# Horner evaluation of 3x^3 + 2x^2 + x + 5 with fmadd. In: fr2 = x, r10 -> doubles 3, 2, 1, 5. Out: fr1 and the double at 32(r10).
    .text
    lfd 1,0(10)
    lfd 3,8(10)
    fmadd 1,1,2,3
    lfd 3,16(10)
    fmadd 1,1,2,3
    lfd 3,24(10)
    fmadd 1,1,2,3
    stfd 1,32(10)
