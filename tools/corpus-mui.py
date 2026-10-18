#!/usr/bin/env python3
"""MUI census: run every MUI program of tools/corpus-apps.tsv once (`amiga68k-run --census`) and report, per app, the first
NEXT-NEEDED call, the first named MUI/dos refusals, and what its windows became on the seam (the route census: native
controls by kind, content-plane frames). Reads the engine's own log, so it is only as deep as the run got.
    AMIGA68K_RUN=target/release/amiga68k-run tools/corpus-mui.py [APPS.tsv] [--json out.json] [--only a,b]
"""
import json, os, re, subprocess, sys
ROOT = "/data/build/corpus-bin"
RUN = os.environ.get("AMIGA68K_RUN", "target/release/amiga68k-run")

def run(app):
    cmd = [RUN, "--census", "--max-insns", "50_000_000"]
    if app["sys"] != "-": cmd += ["--sys", os.path.join(ROOT, app["sys"]), "--assign", "LIBS=SYS:Libs"]
    if app["progdir"] != "-": cmd += ["--mount", "PD=" + os.path.join(ROOT, app["progdir"]), "--progdir", "PD:", "--assign-add", "LIBS=PD:Libs"]
    if app["sys"] == "-" and os.path.isdir(os.path.join(ROOT, "mccs")): cmd += ["--mount", "MCC=" + os.path.join(ROOT, "mccs"), "--assign-add", "LIBS=MCC:Libs"]
    cmd.append(os.path.join(ROOT, app["path"]))
    p = subprocess.run(cmd, capture_output=True, text=True, errors="replace", timeout=900)
    return p.stdout.splitlines(), p.stderr.strip().splitlines()

def summarize(app, lines, err):
    if any("load failed" in e for e in err): return dict(app=app["name"], next="LOAD-FAILED", windows=[], refused=[], ran="")
    ran = next((l for l in lines if l.startswith("ran ")), "")
    nxt = next((l.split(" ", 1)[1] for l in lines if l.startswith("NEXT-NEEDED")), "?")
    refused = []
    for l in lines:
        if "REFUSED:" in l and l.startswith("["):
            r = l.split("REFUSED:", 1)[1].strip()[:110]
            if r not in refused: refused.append(r)
    wins, cur = [], None
    for l in lines:
        if l.startswith("ui: OpenWindow"):
            m = re.search(r'title: "((?:[^"\\]|\\.)*)"', l)
            cur = dict(title=m.group(1) if m else "?", kinds={}, planes=0); wins.append(cur)
        elif l.startswith("ui: AddControl") and wins:
            k = re.search(r"kind: (\w+)", l); k = k.group(1) if k else "?"
            wins[-1]["kinds"][k] = wins[-1]["kinds"].get(k, 0) + 1
        elif l.startswith("ui: PresentPlane") and wins: wins[-1]["planes"] += 1
    return dict(app=app["name"], next=nxt, ran=ran, windows=wins, refused=refused)

if __name__ == "__main__":
    a = sys.argv[1:]
    tsv = a[0] if a and not a[0].startswith("--") else os.path.join(os.path.dirname(__file__), "corpus-apps.tsv")
    only = set(a[a.index("--only") + 1].split(",")) if "--only" in a else None
    out = a[a.index("--json") + 1] if "--json" in a else None
    res = []
    for l in open(tsv):
        f = l.rstrip("\n").split("\t")
        if len(f) < 5 or l.startswith("#") or not f[1].startswith("MUI"): continue
        app = dict(name=f[0], path=f[2], sys=f[3], progdir=f[4])
        if only and app["name"] not in only: continue
        if not os.path.exists(os.path.join(ROOT, app["path"])): print("SKIP (absent):", app["path"], file=sys.stderr); continue
        try: r = summarize(app, *run(app))
        except subprocess.TimeoutExpired: r = dict(app=app["name"], next="TIMEOUT", windows=[], refused=[], ran="")
        res.append(r); print(json.dumps(r), flush=True)
    if out: json.dump(res, open(out, "w"), indent=1)
