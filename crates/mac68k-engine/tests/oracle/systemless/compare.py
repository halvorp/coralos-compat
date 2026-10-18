#!/usr/bin/env python3
"""Compare the oracle application's window as drawn by mac68k-engine (ours.txt, from `mac68k-oracle dump`) with a
screenshot of the same application run on systemless (an EXTERNAL oracle, never linked or copied).

usage: compare.py OURS.txt SHOT.png OUTDIR
Writes OUTDIR/ours.png (our window, 1-bit), OUTDIR/diff.png (ours | oracle | differences, 3x) and OUTDIR/cells.txt (one line per
40x40 cell: row, col, primitive, differing pixels, an FNV-1a hash of the oracle's 1-bit cell and of ours).
The oracle's content origin is found by searching the screenshot for the offset where the two images differ least."""
import sys
from PIL import Image

NAMES = [
    ["line-patCopy", "line-patOr", "line-patXor", "line-patBic", "line-notPatCopy", "line-notPatOr", "line-notPatXor", "line-notPatBic"],
    ["FrameRect", "PaintRect", "InvertRect", "EraseRect", "FillRect-ltGray", "FillRect-dkGray", "FrameRect-pen3", "PaintRect-penGray"],
    ["FrameOval", "PaintOval", "InvertOval", "EraseOval", "FrameRoundRect", "PaintRoundRect", "capsule", "FillOval-gray"],
    ["rgn-A", "rgn-B", "rgn-union", "rgn-diff", "rgn-sect", "rgn-xor", "FrameRgn", "InvertRgn"],
    ["CopyBits-srcCopy", "CopyBits-srcOr", "CopyBits-srcXor", "CopyBits-srcBic", "CopyBits-notSrcCopy", "CopyBits-mask", "CopyBits-shrink", "CopyBits-grow"],
    ["text0", "text0b", "text1", "text1b", "text2", "text2b", "text3", "text3b"],
    ["geom-axis-diag", "geom-shallow", "geom-steep", "geom-pen3x2-2x3", "geom-reversed", "geom-pen5-1x4", "geom-pen4x1-dot", "geom-X"],
]

def fnv(bits):
    h = 0xCBF29CE484222325
    for b in bits:
        h = ((h ^ (1 if b else 0)) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return h

def load_ours(path):
    lines = open(path).read().split("\n")
    w, h = map(int, lines[0].split())
    return w, h, [[c == "1" for c in lines[1 + y]] for y in range(h)]

def main():
    ours_path, shot_path, out = sys.argv[1:4]
    w, h, ours = load_ours(ours_path)
    shot = Image.open(shot_path).convert("RGB")
    px = shot.load()
    black = lambda x, y: px[x, y] == (0, 0, 0)
    best = None
    for oy in range(20, 40):
        for ox in range(48, 75):
            d = sum(black(ox + x, oy + y) != ours[y][x] for y in range(0, h, 3) for x in range(0, w, 3))
            if best is None or d < best[0]:
                best = (d, ox, oy)
    _, ox, oy = best
    theirs = [[black(ox + x, oy + y) for x in range(w)] for y in range(h)]
    print(f"oracle content origin in the screenshot: left {ox} top {oy}")
    img = Image.new("RGB", (w * 3 + 8, h), (128, 128, 128))
    for y in range(h):
        for x in range(w):
            img.putpixel((x, y), (0, 0, 0) if ours[y][x] else (255, 255, 255))
            img.putpixel((w + 4 + x, y), (0, 0, 0) if theirs[y][x] else (255, 255, 255))
            d = ours[y][x] != theirs[y][x]
            img.putpixel((2 * w + 8 + x, y), (255, 0, 0) if d else ((200, 200, 200) if ours[y][x] else (255, 255, 255)))
    img.crop((0, 0, w, h)).save(f"{out}/ours.png")
    img.save(f"{out}/diff.png")
    lines = []
    for r in range(7):
        for c in range(8):
            cells = lambda m: [m[r * 40 + y][c * 40 + x] for y in range(40) for x in range(40)]
            a, b = cells(ours), cells(theirs)
            lines.append(f"{r} {c} {NAMES[r][c]} diff={sum(p != q for p, q in zip(a, b))} oracle={fnv(b):016x} ours={fnv(a):016x}")
    open(f"{out}/cells.txt", "w").write("\n".join(lines) + "\n")
    print("\n".join(lines))

main()
