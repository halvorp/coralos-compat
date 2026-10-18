"""wrap.py in.s out : assemble a PC-relative m68k .s into a one-hunk AmigaOS executable."""
import subprocess, struct, sys, tempfile, os
src, out = sys.argv[1:3]
o = tempfile.mktemp(suffix=".o"); b = tempfile.mktemp(suffix=".bin")
subprocess.check_call(["m68k-linux-gnu-as", "-m68020", "-o", o, src])
subprocess.check_call(["m68k-linux-gnu-objcopy", "-O", "binary", "-j", ".text", o, b])
d = open(b, "rb").read(); d += b"\0" * (-len(d) % 4); n = len(d) // 4
open(out, "wb").write(struct.pack(">IIIIII", 0x3F3, 0, 1, 0, 0, n) + struct.pack(">II", 0x3E9, n) + d + struct.pack(">I", 0x3F2))
os.unlink(o); os.unlink(b)
