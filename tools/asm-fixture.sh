#!/bin/sh
# Assemble a position-independent-by-relocation m68k fixture: tools/asm-fixture.sh in.s out-prefix
# Produces out-prefix.bin (the .text section, absolute-pointer addends patched in as offsets from the section start)
# and out-prefix.rel (one decimal byte offset per line: every longword the test must relocate by the hunk base).
# The tests wrap these as a one-hunk executable with retro-hunk's writer, so they need no cross toolchain to RUN.
set -e
in=$1; out=$2
obj=$(mktemp --suffix=.o)
m68k-linux-gnu-as -m68020 -o "$obj" "$in"
m68k-linux-gnu-objcopy -O binary -j .text "$obj" "$out.bin"
python3 - "$obj" "$out.bin" "$out.rel" <<'PY'
import re, subprocess, sys
obj, binf, relf = sys.argv[1:4]
txt = subprocess.check_output(["m68k-linux-gnu-objdump", "-r", obj], text=True)
data = bytearray(open(binf, "rb").read())
offs = []
for line in txt.splitlines():
    m = re.match(r"([0-9a-f]+)\s+R_68K_32\s+\.text(?:\+0x([0-9a-f]+))?", line.strip())
    if not m: continue
    off, add = int(m.group(1), 16), int(m.group(2) or "0", 16)
    data[off:off+4] = (int.from_bytes(data[off:off+4], "big") + add).to_bytes(4, "big")
    offs.append(off)
open(binf, "wb").write(data)
open(relf, "w").write("".join(f"{o}\n" for o in sorted(offs)))
PY
rm -f "$obj"
