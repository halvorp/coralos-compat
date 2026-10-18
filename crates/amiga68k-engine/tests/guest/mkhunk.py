#!/usr/bin/env python3
"""Wrap a raw 68k blob (+ its .relocs offsets) into a one-hunk AmigaOS Hunk executable: mkhunk.py NAME OUT."""
import struct, sys
name, out = sys.argv[1], sys.argv[2]
code = open(name + ".bin", "rb").read()
code += b"\0" * (-len(code) % 4)
relocs = [int(l) for l in open(name + ".relocs").read().split()]
b = struct.pack(">IIIII", 0x3F3, 0, 1, 0, 0) + struct.pack(">I", len(code) // 4)
b += struct.pack(">II", 0x3E9, len(code) // 4) + code
if relocs:
    b += struct.pack(">III", 0x3EC, len(relocs), 0) + b"".join(struct.pack(">I", r) for r in relocs) + struct.pack(">I", 0)
b += struct.pack(">I", 0x3F2)
open(out, "wb").write(b)
