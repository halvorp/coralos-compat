#!/usr/bin/env python3
"""Static toolkit evidence for each corpus hunk binary: the libraries it names, the MUI classes/MCCs, the BOOPSI classes and
the GadTools/ASL/reqtools markers found as strings in the executable. NAME EVIDENCE ONLY: a class named here may be unused
and an indirect class is missed; the census log (corpus-census.py) is the runtime evidence. Usage: corpus-strings.py APPS.tsv"""
import json, os, re, sys
ROOT = "/data/build/corpus-bin"
LIB = re.compile(rb"[A-Za-z0-9_\-]{2,24}\.library")
MUI = re.compile(rb"\b([A-Z][A-Za-z0-9_]{2,24})\.(mui|mcc)\b")
SYS_BOOPSI = {b"rootclass", b"imageclass", b"frameiclass", b"sysiclass", b"fillrectclass", b"gadgetclass", b"propgclass", b"strgclass",
              b"buttongclass", b"frbuttonclass", b"groupgclass", b"icclass", b"modelclass", b"itexticlass", b"pointerclass",
              b"gradientsliderclass", b"tbiclass", b"colorwheelclass", b"windowclass", b"layoutclass", b"listbrowserclass",
              b"chooserclass", b"clicktabclass", b"checkboxclass", b"integerclass", b"getfileclass", b"buttonclass", b"scrollerclass"}
GT = {0: "GENERIC", 1: "BUTTON", 2: "CHECKBOX", 3: "INTEGER", 4: "LISTVIEW", 5: "MX", 6: "NUMBER", 7: "CYCLE", 8: "PALETTE", 9: "SCROLLER", 11: "SLIDER", 12: "STRING", 13: "TEXT"}
def gt_kinds(d):
    """HEURISTIC: `moveq #K,d0` within 14 bytes before `jsr -30(a6)` (CreateGadgetA is gadtools LVO -30). Only meaningful for a
    binary that names gadtools.library; other libraries' first vector is also -30, so a hit is a candidate, not proof."""
    found = {}
    i = d.find(b"\x4e\xae\xff\xe2")
    while i != -1:
        for back in range(2, 15, 2):
            w = int.from_bytes(d[i - back: i - back + 2], "big") if i - back >= 0 else 0
            if w & 0xFF00 == 0x7000 and w & 0xFF in GT: found[GT[w & 0xFF]] = found.get(GT[w & 0xFF], 0) + 1; break
        i = d.find(b"\x4e\xae\xff\xe2", i + 1)
    return found
def scan(path):
    d = open(path, "rb").read()
    libs = sorted({m.group().decode() for m in LIB.finditer(d)})
    mui = sorted({m.group(1).decode() + "." + m.group(2).decode() for m in MUI.finditer(d)})
    boopsi = sorted(c.decode() for c in SYS_BOOPSI if c in d)
    custom = sorted({m.group().decode() for m in re.finditer(rb"[A-Za-z]{3,24}class\b", d)} - {c.decode() for c in SYS_BOOPSI})[:12]
    gtk = gt_kinds(d) if b"gadtools.library" in d else {}
    return dict(libs=libs, mui=mui, boopsi=boopsi, custom=custom, gt_kinds_static=gtk)
if __name__ == "__main__":
    out = {}
    for l in open(sys.argv[1]):
        f = l.rstrip("\n").split("\t")
        if len(f) < 5 or l.startswith("#"): continue
        p = os.path.join(ROOT, f[2])
        if os.path.exists(p): out[f[0]] = scan(p)
    json.dump(out, sys.stdout, indent=1)
