import sys, time
sys.path.insert(0, "/data/build/oracle/dopus/tools")
from cc import CC
class UI:
    def __init__(s, c, tr=None): s.c = c; s.tr = tr
    def sec(s): return s.c.call("status")["seconds"]
    def run(s, dt):
        if s.tr: return s.tr.run(s.sec() + dt)
        return s.c.call("run_until", {"seconds": s.sec() + dt})
    def shot(s, path): return s.c.call("capture.screenshot", {"path": path})
    def rel(s, dx, dy):
        while dx or dy:
            sx = max(-100, min(100, dx)); sy = max(-100, min(100, dy))
            s.c.call("input.mouse", {"dx": sx, "dy": sy}); s.run(0.05); dx -= sx; dy -= sy
    def move(s, x, y):
        """absolute move in screenshot pixels: pin the pointer in the top-left corner (tip at (64,16)), then move relatively"""
        s.rel(-1000, -600); s.rel(x - 64, y - 16); s.run(0.3)
    def click(s, n=1, hold=0.12):
        for i in range(n):
            s.c.call("input.mouse", {"left": True}); s.run(hold)
            s.c.call("input.mouse", {"left": False}); s.run(hold)
        s.run(0.5)
    def key(s, raw, hold_ms=80): s.c.call("input.key", {"rawkey": raw, "action": "tap", "hold_ms": hold_ms}); s.run(0.4)
    def type(s, text): s.c.call("input.type", {"text": text}); s.run(0.1 * len(text) + 0.6)
