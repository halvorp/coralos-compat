"""Scripted oracle session: startup trace, open a lister, select a file, file operations. usage: scenario.py OUTDIR"""
import sys, os, time
sys.path.insert(0, "/data/build/oracle/dopus/tools")
from cc import CC
from trace import Tracer
from ui import UI
import sigs
out = sys.argv[1]; os.makedirs(out, exist_ok=True)
c = CC("/data/build/oracle/dopus/ccp.json")
print(c.call("continue").get("detail"), flush=True)     # stops at the launcher's LoadSeg
fdn, fds = sigs.load_fd("/data/refs/retro/corpus/dopus5/source/Include/fd/dopus_lib.fd")
t = Tracer(c, f"{out}/p1-startup.raw", extra_libs={"dopus5.library": (fdn, fds, None)})
print("bps", t.setup(), flush=True)
u = UI(c, t)
def phase(name):
    t.out.flush(); t.out = open(f"{out}/{name}.raw", "w")
t.run(28.0); u.shot(f"{out}/01-startup.png"); print("p1 done", t.stops, flush=True)
phase("p2-open-lister")
u.move(222, 54); u.click(2, 0.1); u.run(8); u.shot(f"{out}/02-lister-open.png"); u.run(25); u.shot(f"{out}/03-lister-later.png"); print("p2 done", flush=True)
phase("p3-type-path")
u.move(400, 252); u.click(1, 0.1); u.type("DOpus5:\n"); u.run(10); u.shot(f"{out}/04-path-typed.png"); print("p3 done", flush=True)
phase("p4-select-file")
u.move(300, 85); u.click(1, 0.1); u.run(3); u.shot(f"{out}/05-entry-clicked.png"); print("p4 done", flush=True)
