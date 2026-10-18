# QuickDraw checked against systemless

`mac68k-engine`'s QuickDraw (`src/qd/`) is compared with an independent classic-Mac runtime, **systemless**
(benletchford/systemless, GPL-3), run as an external binary. Nothing of systemless is copied or linked into this MIT repo; only
its pixel output is read, and only our own rendering, the differences and per-cell hashes are committed here.

## The application

`fixtures/testapp/oracle.s` is a hand-assembled 68k Mac application (CODE 0/1 + WIND 128, built with `fixtures/testapp/build.sh`).
On its first update event it draws 56 primitives into a 320x280 window, **one per 40x40 cell**, so a difference localises:

| row | cells |
|-----|-------|
| 0 | a pen-3x2 and a pen-1 line over 50% gray in modes patCopy, patOr, patXor, patBic, notPatCopy, notPatOr, notPatXor, notPatBic |
| 1 | FrameRect, PaintRect, InvertRect, EraseRect, FillRect ltGray / dkGray, FrameRect with a 3x3 pen, PaintRect with PenPat gray |
| 2 | FrameOval, PaintOval, InvertOval, EraseOval, FrameRoundRect, PaintRoundRect, a capsule, FillOval |
| 3 | regions built with OpenRgn/CloseRgn: A, B, Union, Diff, Sect, Xor, FrameRgn(Union), InvertRgn(Union) |
| 4 | CopyBits from an offscreen 32x32 BitMap (NewPtr + OpenPort + SetPortBits): srcCopy, srcOr, srcXor, srcBic, notSrcCopy, srcCopy through an oval mask region, shrunk 32x32 to 24x36, doubled |
| 5 | four DrawString slots in the system font, each followed by a tick under the pen's end point |
| 6 | line geometry on white: slopes, pen sizes, directions |

The window content sits at screen (61, 27), not a multiple of 8, so pattern and bit-shift alignment are exercised.
(systemless misplaces a window whose bottom is past about y=307, which is why the window is 280 high at top 27.)

## Reproduce

```sh
# 1. build systemless once, OUTSIDE this repo (it is GPL-3)
git clone https://github.com/benletchford/systemless /data/build/systemless/src && cd /data/build/systemless/src && cargo build --release
# 2. the application as a MacBinary file, and our rendering of it
cargo run -p mac68k-engine --bin mac68k-oracle -- app oracle.bin
cargo run -p mac68k-engine --bin mac68k-oracle -- dump ours.txt
# 3. systemless draws the same file headlessly and saves a screenshot
echo '{"version":1,"clock":"frontend_ticks","max_ticks":120,"actions":[{"type":"run","ticks":60},{"type":"screenshot","name":"shot"}]}' > play.json
/data/build/systemless/src/target/release/systemless --headless --play-script play.json --play-output out oracle.bin   # out/shot.png
# 4. compare (needs python3 + Pillow); writes ours.png, diff.png, triple.png, cells.txt into res/
mkdir res && python3 compare.py ours.txt out/shot.png res
```

`cells.txt` is the committed result of step 4 (hashes are FNV-1a over a cell's 1-bit pixels). `tests/oracle_systemless.rs` re-runs
our side and fails if any cell changes from its recorded hash or if a cell that agreed with systemless stops agreeing.

## Result (systemless 0.98.1)

41 of 56 cells are **pixel-identical**: all rects, all ovals / round rects / capsules, all region operations including FrameRgn, 
CopyBits in every mode including masked and 2x stretched, the line pen modes patCopy / patOr / patBic, and every line geometry except slope 1.0.
Text is compared by placement only (the font differs by design).

What the oracle found wrong in mac68k-engine, now fixed with the source cited in the code and a test:

* **Lines**: our Bresenham differed from QuickDraw's. Now `DrawLine.a`: one slab per scanline, 16.16 edges, clipped to the pen-travel rectangle.
* **Ovals and round rects**: our sampled ellipse differed by a pixel on curves. Now `DrawArc.a INITOVAL/BUMPOVAL` driven by `PutOval.a` (the right edge carries a +1/2 bias, so ovals are not always left-right symmetric).
* **CopyBits scaling** was refused; now `Stretch.a` (two-axis DDA, shrinking ORs source pixels).
* **FrameRgn / InsetRgn of a general region** were refused; now `Regions.a FrRgn` (DiffRgn(rgn, InsetRgn(rgn, pnSize))).
* **Text baseline** was one row low: the pen is a baseline LINE, the last row of a glyph without descender is `v - 1`; `GetFontInfo` ascent/descent are 7/1 for the stand-in topaz8.

Where we side with Atkinson's source against systemless (`DIVERGENCES` in the test, one reason each):
patXor lines (systemless XORs overlapping pen stamps twice), notPat modes 12..15 on lines (systemless draws them as patCopy),
a StretchBits shrink (systemless samples instead of ORing), and a 45-degree line (systemless is one pixel right at slope exactly 1.0).
