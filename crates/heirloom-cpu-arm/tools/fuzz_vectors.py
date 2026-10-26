"""Differential test vectors: random valid ARMv6 encodings with random operands, flags and FPSCR, with the
expected outcome from Unicorn's ARM1176 model. Seeded, so the output is reproducible.
Usage: fuzz_vectors.py <out dir> [cases per template]
"""
import os
import random
import struct
import sys

sys.path.insert(0, os.path.dirname(__file__))
import uc_oracle  # noqa: E402

R = random.Random(0x41524D36)  # "ARM6"
DATA = 0x8000  # a 256-byte random data window; base registers point into its middle
BASE = DATA + 0x80

EDGE = [0, 1, 2, 0x7F, 0x80, 0xFF, 0x7FFF, 0x8000, 0xFFFF, 0x7FFF_FFFF, 0x8000_0000, 0x8000_0001, 0xFFFF_FFFF,
        0xFFFF_FFFE, 0x0000_8000, 0x7F7F_7F7F, 0x8080_8080, 0xFF00_FF00, 0x00FF_00FF]
F32 = [0, 0x8000_0000, 0x3F80_0000, 0xBF80_0000, 0x4000_0000, 0x7F80_0000, 0xFF80_0000, 0x7FC0_0000, 0x7FA0_0000,
       0xFFC0_0001, 0x0000_0001, 0x807F_FFFF, 0x0080_0000, 0x7F7F_FFFF, 0x3F00_0000, 0x4F00_0000, 0xCF00_0000,
       0x4F80_0000, 0x3EAA_AAAB, 0x0040_0000]
F64 = [0, 0x8000_0000_0000_0000, 0x3FF0_0000_0000_0000, 0xBFF0_0000_0000_0000, 0x7FF0_0000_0000_0000,
       0xFFF0_0000_0000_0000, 0x7FF8_0000_0000_0000, 0x7FF4_0000_0000_0000, 0x0000_0000_0000_0001,
       0x000F_FFFF_FFFF_FFFF, 0x0010_0000_0000_0000, 0x7FEF_FFFF_FFFF_FFFF, 0x41DF_FFFF_FFC0_0000,
       0xC1E0_0000_0000_0000, 0x41F0_0000_0000_0000, 0x3FE0_0000_0000_0000, 0x3810_0000_0000_0000,
       0x47EF_FFFF_E000_0000, 0x3690_0000_0000_0000]


def word():
    k = R.random()
    if k < 0.3:
        return R.choice(EDGE)
    if k < 0.45:
        return R.randrange(0, 64)
    return R.getrandbits(32)


def regs(lo=0, hi=13):
    return R.sample(range(lo, hi), 4)


def flags():
    return R.getrandbits(5) << 27 | R.getrandbits(4) << 16


def cond():
    return 0xE if R.random() < 0.75 else R.randrange(0, 15)


def init_state(mem=False):
    r = [word() for _ in range(13)] + [BASE + 0x40, word(), 0]
    st = {"r": r, "cpsr": flags()}
    if mem:
        st["mem"] = {DATA: bytes(R.getrandbits(8) for _ in range(256))}
    return st


def f32():
    return R.choice(F32) if R.random() < 0.4 else R.getrandbits(32)


def f64():
    return R.choice(F64) if R.random() < 0.4 else R.getrandbits(64)


def fpscr(vector=False):
    v = R.getrandbits(4) << 28 | R.getrandbits(2) << 22 | (R.random() < 0.3) << 24 | (R.random() < 0.3) << 25
    v |= R.getrandbits(5) & R.choice([0, 0x1F])  # some cumulative flags already set
    return v


def vfp_init(st, vector=False):
    s = []
    for _ in range(16):
        if R.random() < 0.5:
            s += [f32(), f32()]
        else:
            d = f64()
            s += [d & 0xFFFF_FFFF, d >> 32]
    st["s"] = s
    st["fpscr"] = fpscr(vector)
    return st


# ---------------------------------------------------------------- A32 templates: () -> (words, init)

def dp_imm():
    op, s = R.randrange(16), R.getrandbits(1)
    if 8 <= op <= 11:
        s = 1
    n, d = R.randrange(16), R.randrange(15)
    if op in (13, 15) and R.random() < 0.9:
        n = 0  # MOV/MVN: Rn is a should-be-zero field
    if 8 <= op <= 11 and R.random() < 0.9:
        d = 0  # TST..CMN: Rd is a should-be-zero field
    if not s and R.random() < 0.1:
        d = 15  # a data-processing branch
    return [cond() << 28 | 1 << 25 | op << 21 | s << 20 | n << 16 | d << 12 | R.getrandbits(12)], init_state()


def dp_shift_imm():
    op, s = R.randrange(16), R.getrandbits(1)
    if 8 <= op <= 11:
        s = 1
    n, d, m = R.randrange(16), R.randrange(15), R.randrange(16)
    if op in (13, 15) and R.random() < 0.9:
        n = 0
    if 8 <= op <= 11 and R.random() < 0.9:
        d = 0
    if not s and R.random() < 0.1:
        d = 15
    imm5 = R.choice([0, 1, 31, R.randrange(32)])
    return [cond() << 28 | op << 21 | s << 20 | n << 16 | d << 12 | imm5 << 7 | R.randrange(4) << 5 | m], init_state()


def dp_shift_reg():
    op, s = R.randrange(16), R.getrandbits(1)
    if 8 <= op <= 11:
        s = 1
    n, d, m, rs = regs()
    st = init_state()
    st["r"][rs] = R.choice([0, 1, 31, 32, 33, 255, 256, R.randrange(64), R.getrandbits(32)])
    return [cond() << 28 | op << 21 | s << 20 | n << 16 | d << 12 | rs << 8 | R.randrange(4) << 5 | 1 << 4 | m], st


def multiply():
    d, a, m, n = regs()
    op = R.choice([0, 1, 2, 4, 5, 6, 7])
    s = 0 if op == 2 else R.getrandbits(1)
    return [cond() << 28 | op << 21 | s << 20 | d << 16 | a << 12 | m << 8 | 0x90 | n], init_state()


def halfword_multiply():
    d, a, m, n = regs()
    op = R.randrange(4)
    return [cond() << 28 | 0x0100_0000 | op << 21 | d << 16 | a << 12 | m << 8 | 1 << 7 | R.getrandbits(2) << 5 | n], \
        init_state()


def misc_arith():
    n, d, m, _ = regs()
    if R.random() < 0.25:
        return [cond() << 28 | 0x016F_0F10 | d << 12 | m], init_state()  # CLZ
    return [cond() << 28 | 0x0100_0050 | R.randrange(4) << 21 | n << 16 | d << 12 | m], init_state()  # QADD..QDSUB


def parallel():
    n, d, m, _ = regs()
    prefix = R.choice([1, 2, 3, 5, 6, 7])
    op = R.choice([0, 1, 2, 3, 4, 7])
    return [cond() << 28 | 0x0600_0010 | prefix << 20 | n << 16 | d << 12 | 0xF00 | op << 5 | m], init_state()


def media_misc():
    n, d, m, _ = regs()
    st = init_state()
    st["cpsr"] = flags()
    k = R.randrange(9)
    c = cond() << 28
    if k == 0:  # PKHBT/PKHTB
        return [c | 0x0680_0010 | n << 16 | d << 12 | R.randrange(32) << 7 | R.getrandbits(1) << 6 | m], st
    if k == 1:  # SSAT / USAT
        return [c | 0x06A0_0010 | R.getrandbits(1) << 22 | R.randrange(32) << 16 | d << 12 | R.randrange(32) << 7
                | R.getrandbits(1) << 6 | m], st
    if k == 2:  # SSAT16 / USAT16
        return [c | 0x06A0_0F30 | R.getrandbits(1) << 22 | R.randrange(16) << 16 | d << 12 | m], st
    if k == 3:  # extends, with and without the add
        op1 = R.choice([0, 2, 3, 4, 6, 7])
        rn = 15 if R.random() < 0.5 else n
        return [c | 0x0680_0070 | op1 << 20 | rn << 16 | d << 12 | R.randrange(4) << 10 | m], st
    if k == 4:  # REV, REV16, REVSH
        return [c | R.choice([0x06BF_0F30, 0x06BF_0FB0, 0x06FF_0FB0]) | d << 12 | m], st
    if k == 5:  # SEL
        return [c | 0x0680_0FB0 | n << 16 | d << 12 | m], st
    if k == 6:  # USAD8 / USADA8
        a = 15 if R.random() < 0.5 else R.randrange(13)
        return [c | 0x0780_0010 | d << 16 | a << 12 | n << 8 | m], st
    if k == 7:  # SMLAD/SMLSD/SMUAD/SMUSD, SMLALD/SMLSLD
        a = 15 if R.random() < 0.3 else R.choice([x for x in range(13) if x != d])
        op1 = R.choice([0, 4]) if a != 15 else 0
        return [c | 0x0700_0010 | op1 << 20 | d << 16 | a << 12 | n << 8 | R.getrandbits(2) << 5 | m], st
    a = 15 if R.random() < 0.3 else R.randrange(13)  # SMMLA/SMMUL/SMMLS
    op2 = R.choice([0, 1]) if a == 15 else R.choice([0, 1, 6, 7])
    return [c | 0x0750_0010 | d << 16 | a << 12 | n << 8 | op2 << 5 | m], st


def load_store():
    t, n, m, _ = regs()
    st = init_state(mem=True)
    st["r"][n] = BASE + R.randrange(-32, 32)
    c = cond() << 28
    k = R.randrange(4)
    p, u, w = R.getrandbits(1), R.getrandbits(1), R.getrandbits(1)
    if not p:
        w = 0
    if k == 0:  # word/byte immediate
        return [c | 0x0400_0000 | p << 24 | u << 23 | R.getrandbits(1) << 22 | w << 21 | R.getrandbits(1) << 20
                | n << 16 | t << 12 | R.randrange(48)], st
    if k == 1:  # word/byte scaled register
        st["r"][m] = R.randrange(0, 12)
        return [c | 0x0600_0000 | p << 24 | u << 23 | R.getrandbits(1) << 22 | w << 21 | R.getrandbits(1) << 20
                | n << 16 | t << 12 | R.randrange(3) << 7 | R.choice([0, 1, 2]) << 5 | m], st
    if k == 2:  # halfword / signed
        op2, load = R.choice([(1, 0), (1, 1), (2, 1), (3, 1)])
        imm = R.getrandbits(1)
        if not imm:
            st["r"][m] = R.randrange(0, 16)
        off = R.randrange(3) << 8 | R.randrange(16) if imm else m
        return [c | p << 24 | u << 23 | imm << 22 | w << 21 | load << 20 | n << 16 | t << 12 | 1 << 7 | op2 << 5
                | 1 << 4 | off], st
    # LDRD / STRD (word-aligned)
    t = R.randrange(0, 12, 2)
    n = R.choice([x for x in range(13) if x not in (t, t + 1)])
    m = R.choice([x for x in range(13) if x not in (t, t + 1, n)])
    st["r"][n] = BASE + 4 * R.randrange(-6, 6)
    st["r"][m] = 4 * R.randrange(0, 4)
    imm = R.getrandbits(1)
    off = (R.randrange(2) << 8 | 4 * R.randrange(4)) if imm else m
    return [c | p << 24 | u << 23 | imm << 22 | w << 21 | n << 16 | t << 12 | 1 << 7 | R.choice([2, 3]) << 5 | 1 << 4
            | off], st


def load_store_multiple():
    st = init_state(mem=True)
    n = R.randrange(14)
    st["r"][n] = BASE + 4 * R.randrange(-4, 4)
    lst = R.getrandbits(16) or 1
    if R.random() < 0.7:
        lst &= 0x7FFF
    lst = lst or 2
    load, w = R.getrandbits(1), R.getrandbits(1)
    if w and lst >> n & 1:
        lst &= ~(1 << n)
        lst = lst or (1 << ((n + 1) % 15))
    pu = R.getrandbits(2)
    if load and lst >> 15 & 1:
        cnt = bin(lst).count("1")
        start = {0b01: 0, 0b11: 4, 0b00: -4 * cnt + 4, 0b10: -4 * cnt}[pu] + st["r"][n]
        fix_word(st, start + 4 * (cnt - 1))
    return [cond() << 28 | 0x0800_0000 | pu << 23 | w << 21 | load << 20 | n << 16 | lst], st


def fix_word(st, addr):
    """Make the data word at addr a valid BX target (bits [1:0] != 10)."""
    mem = bytearray(st["mem"][DATA])
    off = addr - DATA
    if 0 <= off <= len(mem) - 4:
        mem[off] = mem[off] & ~2 if mem[off] & 1 == 0 else mem[off]
        st["mem"][DATA] = bytes(mem)


def swp_exclusive():
    t, n, m, d = regs()
    st = init_state(mem=True)
    st["r"][n] = BASE + 4 * R.randrange(-8, 8)
    k = R.randrange(3)
    if k == 0:
        return [cond() << 28 | 0x0100_0090 | R.getrandbits(1) << 22 | n << 16 | t << 12 | m], st
    size = R.choice([0, 2, 3])  # word, byte, halfword (doubleword below)
    ldrex = 0xE190_0F9F | size << 21 | n << 16 | t << 12
    strex = 0xE180_0F90 | size << 21 | n << 16 | d << 12 | m
    if k == 2 and R.random() < 0.5:
        t = R.randrange(0, 12, 2)
        n = R.choice([x for x in range(13) if x not in (t, t + 1)])
        d = R.choice([x for x in range(13) if x not in (t, t + 1, n)])
        st["r"][n] = BASE + 8 * R.randrange(-4, 4)
        ldrex = 0xE1B0_0F9F | n << 16 | t << 12
        strex = 0xE1A0_0F90 | n << 16 | d << 12 | t
    st["n"] = 2
    middle = [0xF57F_F01F] if k == 2 and R.random() < 0.3 else []  # CLREX makes the STREX fail
    st["n"] += len(middle)
    return [ldrex] + middle + [strex], st


def branch():
    st = init_state()
    k = R.randrange(4)
    if k == 0:
        return [cond() << 28 | 0x0A00_0000 | R.getrandbits(1) << 24 | R.getrandbits(24)], st
    if k == 1:
        return [0xFA00_0000 | R.getrandbits(1) << 24 | R.getrandbits(24)], st  # BLX imm
    m = R.randrange(13)
    st["r"][m] = R.getrandbits(32) & ~2 if R.random() < 0.5 else R.getrandbits(32) | 1
    return [cond() << 28 | (0x012F_FF30 if k == 2 else 0x012F_FF10) | m], st  # BLX / BX


def status():
    st = init_state()
    d, m = R.randrange(15), R.randrange(15)
    k = R.randrange(3)
    if k == 0:
        return [cond() << 28 | 0x010F_0000 | d << 12], st  # MRS
    mask = R.choice([8, 4, 12, 1, 9, 15])
    if k == 1:
        return [cond() << 28 | 0x0120_F000 | mask << 16 | m], st
    return [cond() << 28 | 0x0320_F000 | mask << 16 | R.getrandbits(12)], st


def cp15():
    st = init_state()
    t = R.randrange(13)
    k = R.randrange(4)
    if k == 0:
        st["n"] = 2
        u = R.randrange(13)
        return [0xEE0D_0F50 | t << 12, 0xEE1D_0F50 | u << 12], st  # MCR then MRC TPIDRURW
    if k == 1:
        return [0xEE1D_0F70 | t << 12], st  # MRC TPIDRURO
    return [R.choice([0xEE07_0F9A, 0xEE07_0FBA, 0xEE07_0F95]) | t << 12], st  # barriers


# VFP -------------------------------------------------------------------------------------------------

def vfp_regfield(double, r):
    """Encode register r into (V, bit): single = V:bit, double = bit:V."""
    return (r & 0xF, r >> 4) if double else (r >> 1, r & 1)


def vfp_arith(vector=False):
    double = R.getrandbits(1)
    st = vfp_init(init_state(), vector)
    if vector:
        # Stride 1 only, and no double-precision VMOV/VABS/VNEG/VSQRT: Unicorn's QEMU 5.0 base advances a
        # stride-2 vector by 4 and does not vectorise those double ops. tests/vfp.rs covers both by hand.
        length = R.randrange(2, (4 if double else 8) + 1)
        st["fpscr"] |= (length - 1) << 16
    lim = 16 if double else 32
    d, n, m = R.randrange(lim), R.randrange(lim), R.randrange(lim)
    ops = [(0, 0), (0, 1), (1, 0), (1, 1), (2, 0), (2, 1), (3, 0), (3, 1), (8, 0)]
    if R.random() < 0.3 and not (vector and double):
        ext = R.choice([(0, 0), (0, 1), (1, 0), (1, 1)])  # VMOV VABS VNEG VSQRT
        vd, bd = vfp_regfield(double, d)
        vm, bm = vfp_regfield(double, m)
        return [cond() << 28 | 0x0EB0_0A40 | double << 8 | bd << 22 | ext[0] << 16 | vd << 12 | ext[1] << 7
                | bm << 5 | vm], st
    opc, op = R.choice(ops)
    vd, bd = vfp_regfield(double, d)
    vn, bn = vfp_regfield(double, n)
    vm, bm = vfp_regfield(double, m)
    p, q, r = opc >> 3 & 1, opc >> 1 & 1, opc & 1
    return [cond() << 28 | 0x0E00_0A00 | p << 23 | bd << 22 | q << 21 | r << 20 | vn << 16 | vd << 12 | double << 8
            | bn << 7 | op << 6 | bm << 5 | vm], st


def vfp_vector():
    return vfp_arith(vector=True)


def vfp_compare_convert():
    double = R.getrandbits(1)
    st = vfp_init(init_state())
    lim = 16 if double else 32
    k = R.randrange(5)
    c = cond() << 28 | 0x0EB0_0A40 | double << 8
    if k == 0:  # VCMP{E}, with zero
        d, m = R.randrange(lim), R.randrange(lim)
        vd, bd = vfp_regfield(double, d)
        vm, bm = vfp_regfield(double, m)
        z = R.getrandbits(1)
        if z:
            vm = bm = 0
        return [c | bd << 22 | (4 | z) << 16 | vd << 12 | R.getrandbits(1) << 7 | bm << 5 | vm], st
    if k == 1:  # VCVT between precisions
        d = R.randrange(32 if double else 16)
        m = R.randrange(lim)
        vd, bd = vfp_regfield(not double, d)
        vm, bm = vfp_regfield(double, m)
        return [c | bd << 22 | 7 << 16 | vd << 12 | 1 << 7 | bm << 5 | vm], st
    if k == 2:  # int -> float
        d, m = R.randrange(lim), R.randrange(32)
        st["s"][m] = word()
        vd, bd = vfp_regfield(double, d)
        vm, bm = vfp_regfield(False, m)
        return [c | bd << 22 | 8 << 16 | vd << 12 | R.getrandbits(1) << 7 | bm << 5 | vm], st
    d, m = R.randrange(32), R.randrange(lim)  # float -> int, VCVT and VCVTR
    vd, bd = vfp_regfield(False, d)
    vm, bm = vfp_regfield(double, m)
    return [c | bd << 22 | (12 | R.getrandbits(1)) << 16 | vd << 12 | R.getrandbits(1) << 7 | bm << 5 | vm], st


def vfp_transfer():
    st = vfp_init(init_state(mem=True))
    t, t2 = R.sample(range(13), 2)
    k = R.randrange(7)
    c = cond() << 28
    if k == 0:  # VMOV Sn <-> Rt
        n = R.randrange(32)
        return [c | 0x0E00_0A10 | R.getrandbits(1) << 20 | (n >> 1) << 16 | t << 12 | (n & 1) << 7], st
    if k == 1:  # VMOV.32 Dn[x] <-> Rt
        return [c | 0x0E00_0B10 | R.getrandbits(1) << 21 | R.getrandbits(1) << 20 | R.randrange(16) << 16
                | t << 12], st
    if k == 2:  # VMOV two core <-> two S / one D
        double = R.getrandbits(1)
        m = R.randrange(16) if double else R.randrange(31)
        vm, bm = vfp_regfield(double, m)
        return [c | 0x0C40_0A10 | R.getrandbits(1) << 20 | t2 << 16 | t << 12 | double << 8 | bm << 5 | vm], st
    if k == 3:  # VMRS / VMSR FPSCR, VMRS APSR_nzcv, VMRS FPSID
        st["r"][t] &= 0xF3F7_009F  # leave the trap enables clear: Unicorn holds them RAZ
        return [c | R.choice([0x0EF1_0A10 | t << 12, 0x0EE1_0A10 | t << 12, 0x0EF1_FA10, 0x0EF0_0A10 | t << 12])], st
    double = R.getrandbits(1)
    n = R.randrange(13)
    st["r"][n] = BASE + 4 * R.randrange(-8, 8)
    lim = 16 if double else 32
    if k == 4:  # VLDR / VSTR
        d = R.randrange(lim)
        vd, bd = vfp_regfield(double, d)
        return [c | 0x0D00_0A00 | R.getrandbits(1) << 23 | bd << 22 | R.getrandbits(1) << 20 | n << 16 | vd << 12
                | double << 8 | R.randrange(8)], st
    # VLDM / VSTM: IA, IA!, DB!
    count = R.randrange(1, 5)
    d = R.randrange(lim - count + 1)
    vd, bd = vfp_regfield(double, d)
    p, u, w = R.choice([(0, 1, 0), (0, 1, 1), (1, 0, 1)])
    imm8 = count * (2 if double else 1)
    return [c | 0x0C00_0A00 | p << 24 | u << 23 | bd << 22 | w << 21 | R.getrandbits(1) << 20 | n << 16 | vd << 12
            | double << 8 | imm8], st


def setend_load():
    st = init_state(mem=True)
    t, n, _, _ = regs()
    st["r"][n] = BASE + R.randrange(-16, 16)
    st["n"] = 2
    load = R.choice([0xE590_0000, 0xE1D0_00B0, 0xE580_0000, 0xE1C0_00B0]) | n << 16 | t << 12 | R.randrange(8)
    return [0xF101_0200, load], st  # SETEND BE; access


# ---------------------------------------------------------------- Thumb templates: () -> (halfwords, init)

def thumb_alu():
    st = init_state()
    lo = lambda: R.randrange(8)  # noqa: E731
    k = R.randrange(6)
    if k == 0:
        return [R.randrange(3) << 11 | R.randrange(32) << 6 | lo() << 3 | lo()], st
    if k == 1:
        return [0x1800 | R.getrandbits(2) << 9 | lo() << 6 | lo() << 3 | lo()], st
    if k == 2:
        return [0x2000 | R.randrange(4) << 11 | lo() << 8 | R.getrandbits(8)], st
    if k == 3:
        st["r"][R.randrange(8)] = R.choice([0, 1, 31, 32, 33, 255, R.getrandbits(32)])
        return [0x4000 | R.randrange(16) << 6 | lo() << 3 | lo()], st
    if k == 4:  # hi-register ADD/CMP/MOV (a PC destination is a branch)
        op = R.randrange(3)
        d = R.randrange(16) if op != 1 else R.randrange(15)
        m = R.randrange(16)
        if op == 1 and d < 8 and m < 8:
            m += 8
        if op == 1 and m == 15:
            m = 14
        return [0x4400 | op << 8 | (d >> 3) << 7 | m << 3 | (d & 7)], st
    return [R.choice([0xB200, 0xB240, 0xB280, 0xB2C0, 0xBA00, 0xBA40, 0xBAC0]) | lo() << 3 | lo()], st


def thumb_memory():
    st = init_state(mem=True)
    lo = lambda: R.randrange(8)  # noqa: E731
    n = lo()
    st["r"][n] = BASE + R.randrange(-32, 32)
    k = R.randrange(7)
    if k == 0:
        m = R.choice([x for x in range(8) if x != n])
        st["r"][m] = R.randrange(-16, 16) & 0xFFFF_FFFF
        return [0x5000 | R.randrange(8) << 9 | m << 6 | n << 3 | lo()], st
    if k == 1:
        return [0x6000 | R.randrange(4) << 11 | R.randrange(8) << 6 | n << 3 | lo()], st
    if k == 2:
        return [0x8000 | R.getrandbits(1) << 11 | R.randrange(16) << 6 | n << 3 | lo()], st
    if k == 3:
        return [0x9000 | R.getrandbits(1) << 11 | lo() << 8 | R.randrange(16)], st
    if k == 4:
        if R.random() < 0.4:
            return [R.choice([0xB000, 0xB080]) | R.randrange(128)], st  # ADD/SUB SP, #imm7*4
        return [R.choice([0x4800, 0xA000, 0xA800]) | lo() << 8 | R.randrange(32)], st
    if k == 5:  # PUSH / POP (POP {PC} interworks)
        op, pc, lst = R.choice([0xB400, 0xBC00]), R.getrandbits(1), R.getrandbits(8) or 1
        if op == 0xBC00 and pc:
            fix_word(st, st["r"][13] + 4 * bin(lst).count("1"))
        return [op | pc << 8 | lst], st
    st["r"][n] = BASE + 4 * R.randrange(-4, 4)
    lst = R.getrandbits(8) | 1 << R.randrange(8)
    load = R.getrandbits(1)
    if not load and lst >> n & 1 and lst & ((1 << n) - 1):
        lst &= ~(1 << n)
    return [0xC000 | load << 11 | n << 8 | (lst or (1 << ((n + 1) % 8)))], st


def thumb_branch():
    st = init_state()
    k = R.randrange(5)
    if k == 0:
        return [0xD000 | R.randrange(14) << 8 | R.getrandbits(8)], st
    if k == 1:
        return [0xE000 | R.getrandbits(11)], st
    if k == 2:  # BL / BLX pair
        suffix = R.choice([0xF800 | R.getrandbits(11), 0xE800 | (R.getrandbits(11) & ~1)])
        return [0xF000 | R.getrandbits(11), suffix], st
    m = R.randrange(15)
    st["r"][m] = R.getrandbits(32) & ~2 if R.random() < 0.5 else R.getrandbits(32) | 1
    return [0x4700 | (R.getrandbits(1) << 7 if m != 15 else 0) | m << 3], st


def thumb_misc():
    st = init_state()
    return [R.choice([0xDF00 | R.getrandbits(8), 0xBE00 | R.getrandbits(8), 0xB650, 0xB658, 0xB660, 0xDE00
                      | R.getrandbits(8), 0xBF00, 0xB100 | R.getrandbits(8)])], st


A32 = {
    "dp_imm": dp_imm, "dp_shift_imm": dp_shift_imm, "dp_shift_reg": dp_shift_reg, "multiply": multiply,
    "halfword_multiply": halfword_multiply, "misc_arith": misc_arith, "parallel": parallel,
    "media_misc": media_misc, "load_store": load_store, "load_store_multiple": load_store_multiple,
    "swp_exclusive": swp_exclusive, "branch": branch, "status": status, "cp15": cp15, "setend": setend_load,
    "vfp_arith": vfp_arith, "vfp_vector": vfp_vector, "vfp_compare_convert": vfp_compare_convert,
    "vfp_transfer": vfp_transfer,
}
THUMB = {"thumb_alu": thumb_alu, "thumb_memory": thumb_memory, "thumb_branch": thumb_branch,
         "thumb_misc": thumb_misc}


def main():
    out_dir = sys.argv[1]
    count = int(sys.argv[2]) if len(sys.argv) > 2 else 300
    for thumb, templates in ((False, A32), (True, THUMB)):
        for name, make in templates.items():
            lines = []
            for i in range(count):
                insns, st = make()
                code = b"".join(struct.pack("<H" if thumb else "<I", w) for w in insns)
                code += b"\0" * (-len(code) % 4)
                out = uc_oracle.run(thumb, code, st)
                lines.append(uc_oracle.fmt_vector("%s_%d" % (name, i), thumb, code, st, out))
            with open(os.path.join(out_dir, "fuzz_%s.vec" % name), "w") as f:
                f.write("# Random ARMv6 encodings (%s), expectations from Unicorn ARM1176.\n" % name)
                f.write("# Generated by tools/fuzz_vectors.py (seeded).\n")
                f.write("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
