"""Port HexRaysSA/rax's generated AArch32 tests (commit 526acfa, tests/generated/arm/{a32,t16}) to vectors.

Each rax test sets registers, writes one instruction at address 0, steps once and asserts on the exit,
registers and NZCV. Many of those assertions were generated from AArch64 semantics and do not hold for the
A32/T16 encoding they test, so every test is first run on Unicorn's ARM1176 model: a test is ported only
when all of its rax assertions hold there, and the ported vector then checks the full Unicorn state, not
just rax's assertions. Tests without assertions (rax's UNPREDICTABLE cases) are ported as no-panic smoke
cases. Usage: port_rax_tests.py <rax checkout> <out dir>
"""
import collections
import os
import re
import struct
import subprocess
import sys

sys.path.insert(0, os.path.dirname(__file__))
import uc_oracle  # noqa: E402

GROUPS = {
    "data_processing": ["a32/data_processing"],
    "multiply": ["a32/multiply"],
    "memory": ["a32/memory"],
    "branch": ["a32/branch"],
    "misc": ["a32/misc"],
    "system": ["a32/system"],
    "coprocessor": ["a32/coprocessor"],
    "vfp": ["a32/simd"],
    "thumb": ["t16"],
}

FN = re.compile(r"#\[test\]\s*fn (\w+)\(\) \{(.*?)\n\}", re.S)
SET_W = re.compile(r"set_(w|sp|lr)\(&mut cpu, (?:(\d+), )?(0x[0-9A-Fa-f]+|\d+)\)")
ENC = re.compile(r"let encoding: u(16|32) = (0x[0-9A-Fa-f]+);")
GET_W = re.compile(r"assert_eq!\(\s*get_w\(&cpu, (\d+)\),\s*(0x[0-9A-Fa-f]+|\d+)")
FLAG = re.compile(r"assert_eq!\(cpu\.get_pstate\(\)\.([nzcv]), (true|false)")


def llvm_softfail(words):
    """Indices of A32 words llvm-mc (armv6k + vfp2) calls potentially undefined or invalid."""
    text = "\n".join(" ".join("0x%02x" % b for b in struct.pack("<I", w)) for w in words) + "\n"
    p = subprocess.run(["llvm-mc", "--disassemble", "-triple=armv6k-none-eabi", "-mattr=+vfp2"],
                       input=text, capture_output=True, text=True)
    bad = set()
    for line in p.stderr.splitlines():
        m = re.match(r"<stdin>:(\d+):\d+: warning: (potentially undefined|invalid)", line)
        if m:
            bad.add(int(m.group(1)) - 1)
    return bad


def parse(body):
    enc = ENC.search(body)
    if not enc:
        return None
    iset = re.search(r"ISET: (\w+)", body)
    iset = iset.group(1) if iset else "A32"
    thumb = iset in ("T16", "T32") or "create_thumb_cpu" in body
    regs = [0] * 16
    for kind, n, v in SET_W.findall(body):
        idx = {"w": None, "sp": 13, "lr": 14}[kind]
        regs[int(n) if idx is None else idx] = int(v, 0) & 0xFFFF_FFFF
    value = int(enc.group(2), 0)
    code = struct.pack("<I", value & 0xFFFF_FFFF)
    asserts = {
        "continue": re.search(r"assert_eq!\(\s*exit,\s*CpuExit::Continue", body) is not None,
        "not_continue": "!matches!(exit.unwrap(), CpuExit::Continue)" in body,
        "regs": [(int(r), int(v, 0)) for r, v in GET_W.findall(body)],
        "flags": [(f, v == "true") for f, v in FLAG.findall(body)],
    }
    smoke = not (asserts["continue"] or asserts["not_continue"] or asserts["regs"] or asserts["flags"])
    return thumb, code, {"r": regs}, asserts, smoke, iset


def holds(out, asserts):
    if asserts["continue"] and out.exit != "ok":
        return False, "exit"
    if asserts["not_continue"] and out.exit == "ok":
        return False, "exit"
    for r, v in asserts["regs"]:
        if out.regs[r] != v & 0xFFFF_FFFF:
            return False, "reg"
    bits = {"n": 31, "z": 30, "c": 29, "v": 28}
    for f, v in asserts["flags"]:
        if bool(out.cpsr >> bits[f] & 1) != v:
            return False, "flag"
    return True, ""


def is_vfp(thumb, code):
    w = struct.unpack("<I", code)[0]
    return not thumb and (w >> 25) & 7 in (6, 7) and (w >> 8) & 0xE == 0xA


def main():
    root, out_dir = sys.argv[1], sys.argv[2]
    base = os.path.join(root, "tests/generated/arm")
    summary = []
    for group, dirs in GROUPS.items():
        stats = collections.Counter()
        seen = set()
        lines, smoke_lines, words = [], [], []
        for d in dirs:
            for dirpath, _, files in os.walk(os.path.join(base, d)):
                for fname in sorted(files):
                    if fname == "mod.rs":
                        continue
                    src = open(os.path.join(dirpath, fname)).read()
                    for name, body in FN.findall(src):
                        stats["rax tests"] += 1
                        parsed = parse(body)
                        if not parsed:
                            stats["skipped: no single encoding"] += 1
                            continue
                        thumb, code, init, asserts, smoke, iset = parsed
                        key = (thumb, code, tuple(init["r"]), smoke)
                        if key in seen:
                            stats["duplicate (same encoding and state)"] += 1
                            continue
                        seen.add(key)
                        out = uc_oracle.run(thumb, code, init)
                        tag = "%s_%s" % (iset.lower(), name[5:] if name.startswith("test_") else name)
                        vfp = is_vfp(thumb, code)
                        if smoke:
                            stats["ported: smoke (rax asserts nothing)"] += 1
                            smoke_lines.append(uc_oracle.fmt_vector(tag, thumb, code, init, out, vfp))
                            continue
                        ok, why = holds(out, asserts)
                        if not ok:
                            stats["dropped: rax assertion false on ARM1176 (%s)" % why] += 1
                            continue
                        stats["ported: full-state check"] += 1
                        lines.append(uc_oracle.fmt_vector(tag, thumb, code, init, out, vfp))
                        if not thumb:
                            words.append((len(lines) - 1, struct.unpack("<I", code)[0]))
        soft = llvm_softfail([w for _, w in words])
        for k, (i, _) in enumerate(words):
            if k in soft:
                lines[i] += " unp=1"
        with open(os.path.join(out_dir, "rax_%s.vec" % group), "w") as f:
            f.write("# Ported from HexRaysSA/rax tests/generated/arm (MIT, commit 526acfa); expectations from\n")
            f.write("# Unicorn ARM1176 (only tests whose rax assertions hold there). Generated by tools/port_rax_tests.py.\n")
            for line in lines:
                f.write(line + "\n")
            for line in smoke_lines:
                f.write(line.replace(";exit=", ";smoke=1 exit=", 1) + "\n")
        summary.append("[%s]" % group)
        summary += ["  %-50s %d" % (k, v) for k, v in sorted(stats.items())]
    text = "\n".join(summary) + "\n"
    open(os.path.join(out_dir, "rax_port_summary.txt"), "w").write(text)
    print(text)


if __name__ == "__main__":
    main()
