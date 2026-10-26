# CRC-32 (IEEE 802.3, reflected, poly 0xEDB88320), bit at a time. In: r3 = pointer, r4 = length. Out: r5.
    .text
    lis 5,-1
    ori 5,5,0xFFFF          # crc = 0xFFFFFFFF
    lis 6,0xEDB8
    ori 6,6,0x8320          # reflected polynomial
    cmpwi 0,4,0
    beq done
    mtctr 4
bytel:
    lbz 7,0(3)
    addi 3,3,1
    xor 5,5,7
    li 9,8
bitl:
    andi. 10,5,1
    srwi 5,5,1
    beq nox
    xor 5,5,6
nox:
    addic. 9,9,-1
    bne bitl
    bdnz bytel
done:
    not 5,5
