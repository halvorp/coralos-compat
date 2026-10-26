"""Run one instruction on Unicorn's ARM1176 model (QEMU) in user mode and report the outcome.

Host-side test-vector tooling for heirloom-cpu-arm; not part of the crate. Needs `pip install unicorn`.
A case is (iset, code bytes at address 0, initial state); the result is the exit kind and the full
architectural state afterwards, which the vector writers serialise for the Rust tests.
"""
from unicorn import Uc, UcError, UC_ARCH_ARM, UC_MODE_ARM, UC_MODE_THUMB, UC_HOOK_INTR, UC_HOOK_MEM_WRITE
from unicorn import UC_ERR_INSN_INVALID, UC_ERR_EXCEPTION, UC_ERR_FETCH_UNMAPPED
from unicorn.arm_const import *

MEM_SIZE = 0x1000_0000  # the address space the vectors use; above it every access faults
CPSR_MASK = 0xF80F_0220  # NZCVQ, GE, E, T
FPSCR_MASK = 0xF3F7_9F9F

REGS = [UC_ARM_REG_R0 + i for i in range(13)] + [UC_ARM_REG_SP, UC_ARM_REG_LR, UC_ARM_REG_PC]
SREGS = [UC_ARM_REG_S0 + i for i in range(32)]

_uc = None


def _machine():
    global _uc
    if _uc is None:
        uc = Uc(UC_ARCH_ARM, UC_MODE_ARM)
        uc.ctl_set_cpu_model(UC_CPU_ARM_1176)
        uc.mem_map(0, MEM_SIZE)
        _uc = uc
    return _uc


class Outcome:
    def __init__(self):
        self.exit = "ok"
        self.regs = []
        self.cpsr = 0
        self.fpscr = 0
        self.s = []
        self.writes = {}


def run(thumb, code, init):
    """init: dict with optional r (list of 16), cpsr, fpscr, s (list of 32), mem {addr: bytes}."""
    uc = _machine()
    touched = list(init.get("mem", {}).items())
    uc.mem_write(0, b"\0" * 0x100)
    for addr, data in touched:
        uc.mem_write(addr, data)
    uc.mem_write(0, code)
    uc.ctl_remove_cache(0, 0x1000)
    uc.reg_write(UC_ARM_REG_C1_C0_2, 0xF00000)
    uc.reg_write(UC_ARM_REG_FPEXC, 0x40000000)
    uc.reg_write(UC_ARM_REG_FPSCR, init.get("fpscr", 0))  # before CPSR: its write rebuilds QEMU's hflags
    cpsr = (init.get("cpsr", 0) & CPSR_MASK & ~0x20) | 0x10 | (0x20 if thumb else 0)
    uc.reg_write(UC_ARM_REG_CPSR, cpsr)
    regs = list(init.get("r", [0] * 16))
    for reg, value in zip(REGS[:15], regs[:15]):
        uc.reg_write(reg, value)
    for reg, value in zip(SREGS, init.get("s", [0] * 32)):
        uc.reg_write(reg, value)
    pc = regs[15]
    out = Outcome()
    intr = []
    writes = {}

    def on_intr(u, intno, _):
        intr.append(intno)
        u.emu_stop()

    def on_write(u, access, address, size, value, _):
        for i in range(size):
            writes[address + i] = None  # value read back after the run (handles endianness)

    h1 = uc.hook_add(UC_HOOK_INTR, on_intr)
    h2 = uc.hook_add(UC_HOOK_MEM_WRITE, on_write)
    try:
        uc.emu_start(pc | (1 if thumb else 0), 0xFFFF_FFFF, count=init.get("n", 1))
    except UcError as e:
        if e.errno == UC_ERR_FETCH_UNMAPPED and uc.reg_read(UC_ARM_REG_PC) != pc:
            out.exit = "ok"  # the instruction branched; QEMU then failed to fetch the target
        else:
            out.exit = "undef" if e.errno in (UC_ERR_INSN_INVALID, UC_ERR_EXCEPTION) else "fault"
    finally:
        uc.hook_del(h1)
        uc.hook_del(h2)
    if intr:
        n = intr[0]
        if n == 2:
            out.exit = "svc"
        elif n == 7:
            out.exit = "bkpt"
        elif n == 1:
            out.exit = "undef"
        else:
            out.exit = "exc%d" % n
    out.regs = [uc.reg_read(r) for r in REGS]
    out.cpsr = uc.reg_read(UC_ARM_REG_CPSR)
    out.fpscr = uc.reg_read(UC_ARM_REG_FPSCR)
    out.s = [uc.reg_read(r) for r in SREGS]
    if out.exit in ("undef", "fault", "bkpt") or (out.exit.startswith("exc")):
        writes = {}
    out.writes = {a: uc.mem_read(a, 1)[0] for a in sorted(writes) if a < MEM_SIZE}
    # Unicorn leaves PC on an SVC; the crate's contract is PC past it.
    # Unicorn reports PC past an SVC and on an undefined/faulting/BKPT instruction: the crate's contract.

    # Undo the scratch writes so the next case starts clean.
    clear = set(writes)
    for addr, data in touched:
        clear.update(range(addr, addr + len(data)))
    for addr in clear:
        if addr < MEM_SIZE:
            uc.mem_write(addr, b"\0")
    return out


def fmt_vector(name, thumb, code, init, out, vfp=True):
    """One vector line: name;iset;code;init;expect. The expectation lists the exit, the CPSR flags, and
    only the registers that changed; the runner checks that everything else is unchanged."""
    parts = []
    r = list(init.get("r", [0] * 16))
    s_init = list(init.get("s", [0] * 32))
    fpscr_init = init.get("fpscr", 0)
    for i, v in enumerate(r):
        if v:
            parts.append("r%d=%x" % (i, v))
    if init.get("cpsr", 0) & CPSR_MASK & ~0x20:
        parts.append("cpsr=%x" % (init["cpsr"] & CPSR_MASK & ~0x20))
    if fpscr_init:
        parts.append("fpscr=%x" % fpscr_init)
    for i, v in enumerate(s_init):
        if v:
            parts.append("s%d=%x" % (i, v))
    if init.get("n", 1) != 1:
        parts.append("n=%d" % init["n"])
    for addr, data in sorted(init.get("mem", {}).items()):
        parts.append("m%x=%s" % (addr, data.hex()))
    exp = ["exit=%s" % out.exit, "cpsr=%x" % (out.cpsr & CPSR_MASK)]
    exp += ["r%d=%x" % (i, v) for i, v in enumerate(out.regs) if v != r[i]]
    if (out.fpscr & FPSCR_MASK) != fpscr_init:
        exp.append("fpscr=%x" % (out.fpscr & FPSCR_MASK))
    exp += ["s%d=%x" % (i, v) for i, v in enumerate(out.s) if v != s_init[i]]
    if out.writes:
        runs, start, buf = [], None, b""
        for a in sorted(out.writes):
            if start is not None and a == start + len(buf):
                buf += bytes([out.writes[a]])
            else:
                if start is not None:
                    runs.append((start, buf))
                start, buf = a, bytes([out.writes[a]])
        runs.append((start, buf))
        exp += ["m%x=%s" % (a, b.hex()) for a, b in runs]
    return "%s;%s;%s;%s;%s" % (name, "t16" if thumb else "a32", code.hex(), " ".join(parts), " ".join(exp))
