#!/usr/bin/env python3
"""Cut the fat Toolbox Showcase application (68k CODE in the resource fork + a PowerPC PEF in the data fork) out of systemless's
STORED (uncompressed) StuffIt 5 archive. The same trick as crates/heirloom-macppc-engine/tests/real_app.rs. systemless is GPL-3: the
output is test data under /data/build/corpus-bin, never committed. Usage: corpus-extract-showcase.py ARCHIVE.sit OUTDIR"""
import os, struct, sys
d = open(sys.argv[1], "rb").read(); out = sys.argv[2]
be32 = lambda at: struct.unpack(">I", d[at:at + 4])[0]
o = d.find(b"Joy!peff")
n = struct.unpack(">H", d[o + 32:o + 34])[0]
end = max(be32(o + 40 + 28 * i + 20) + be32(o + 40 + 28 * i + 16) for i in range(n))
start = next(s for s in range(100, 2000) if be32(s) == 0x100 and be32(s) + be32(s + 8) == be32(s + 4) and be32(s + 4) + be32(s + 12) == o - s)
os.makedirs(out, exist_ok=True)
open(os.path.join(out, "showcase.pef"), "wb").write(d[o:o + end])
open(os.path.join(out, "showcase.rsrc"), "wb").write(d[start:o])
print("pef", end, "rsrc", o - start)
