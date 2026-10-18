#!/usr/bin/env python3
"""Run the amiga68k census over the corpus (binaries live in /data/build/corpus-bin, never in git).

For each app: pass 1 is the plain census (NEXT-NEEDED = the first function no lane has written); pass 2 keeps going with
`--stub lib/Func` for each NEXT-NEEDED in turn (zeroed return) so the CHAIN of unimplemented functions the app needs is
recorded, not only the first. Output: one JSON object per app on stdout (and --out FILE). Usage:
    tools/corpus-census.py APPS.tsv [--out census.json] [--only name,...]
APPS.tsv columns: name, toolkit, path (under /data/build/corpus-bin), sys-dir (or -), progdir (or -)
"""
import json, os, re, subprocess, sys
ROOT = "/data/build/corpus-bin"
RUN = os.environ.get("AMIGA68K_RUN", "/data/wt/compat-corpus/target/release/amiga68k-run")
CAP = 40

def run(app, stubs):
    cmd = [RUN, "--census", "--max-insns", "50_000_000"]
    if app["sys"] != "-":
        cmd += ["--sys", os.path.join(ROOT, app["sys"]), "--assign", "LIBS=SYS:Libs"]
    if app["progdir"] != "-":
        cmd += ["--mount", "PD=" + os.path.join(ROOT, app["progdir"]), "--progdir", "PD:", "--assign-add", "LIBS=PD:Libs"]
    if app["sys"] == "-" and os.path.isdir(os.path.join(ROOT, "mccs")):  # AmigaOS-built binaries: the MCCs YAM ships, as LIBS:mui/
        cmd += ["--mount", "MCC=" + os.path.join(ROOT, "mccs"), "--assign-add", "LIBS=MCC:Libs"]
    for s in stubs: cmd += ["--stub", s]
    cmd.append(os.path.join(ROOT, app["path"]))
    p = subprocess.run(cmd, capture_output=True, text=True, timeout=600)
    return p.stdout.splitlines(), p.stderr.strip().splitlines()

def summarize(lines):
    ran = next((l for l in lines if l.startswith("ran ")), "")
    nxt = next((l.split(" ", 1)[1] for l in lines if l.startswith("NEXT-NEEDED")), "?")
    ui = [l for l in lines if l.startswith("ui: ")]
    kinds = {}
    for l in ui:
        m = re.match(r"ui: (\w+)", l); k = m.group(1) if m else "?"
        kinds[k] = kinds.get(k, 0) + 1
    ctrl = sorted(set(re.findall(r"kind: (\w+)", " ".join(ui))))
    calls = [l for l in lines if l.startswith("[") and "REFUSED" not in l]
    return ran, nxt, kinds, ctrl, calls

GT_KINDS = {0: "GENERIC", 1: "BUTTON", 2: "CHECKBOX", 3: "INTEGER", 4: "LISTVIEW", 5: "MX", 6: "NUMBER", 7: "CYCLE", 8: "PALETTE",
            9: "SCROLLER", 11: "SLIDER", 12: "STRING", 13: "TEXT"}  # gadtools/gadtools.h *_KIND

def details(lines):
    """What the run actually created, read from the engine's own log lines: GadTools kinds, MUI classes / MUIO_ objects,
    BOOPSI classes made or instantiated, ASL requester types, menu builders."""
    d = {"gt_kinds": set(), "mui_classes": set(), "mui_make": set(), "boopsi_new": set(), "boopsi_made": set(), "asl": set(), "other": set()}
    for l in lines:
        m = re.search(r"CreateGadgetA kind=(\d+)", l)
        if m: d["gt_kinds"].add(GT_KINDS.get(int(m.group(1)), "kind" + m.group(1)))
        m = re.search(r"MUI_NewObjectA \"([^\"]+)\"", l)
        if m: d["mui_classes"].add(m.group(1))
        m = re.search(r"MUI_MakeObjectA (\w+)", l)
        if m: d["mui_make"].add(m.group(1))
        m = re.search(r"\[intuition\] NewObjectA \"?([\w.]+)\"?", l)
        if m: d["boopsi_new"].add(m.group(1))
        m = re.search(r"\[intuition\] MakeClass \"([^\"]+)\" super=(\w+)", l)
        if m: d["boopsi_made"].add(m.group(1) + " : " + m.group(2))
        m = re.search(r"AllocAslRequest (\w+)", l)
        if m: d["asl"].add(m.group(1))
        for k in ("CreateMenusA", "EasyRequestArgs", "MUI_RequestA", "MUI_CreateCustomClass", "BrokerCommand", "CxBroker"):
            if k in l and "REFUSED" not in l: d["other"].add(k)
    return {k: sorted(v) for k, v in d.items()}

def census(app):
    lines, err = run(app, [])
    if any("load failed" in e for e in err):
        return dict(app, first="LOAD-FAILED: " + next(e for e in err if "load failed" in e), chain=[], ran="", ui={}, ctrl=[])
    ran, first, kinds, ctrl, calls = summarize(lines)
    chain, stubs, last = [], [], lines
    cur = first
    while cur not in ("none (no unimplemented call reached)", "?") and len(stubs) < CAP:
        chain.append(cur); stubs.append(cur)
        last, err = run(app, stubs)
        _, nxt, _, _, _ = summarize(last)
        if nxt == cur: break
        cur = nxt
    ran2, end, kinds2, ctrl2, calls2 = summarize(last)
    if cur in ("none (no unimplemented call reached)", "?"): end = cur
    else: chain.append(cur)
    refused = sorted(set(l[:160] for l in last if "REFUSED" in l))[:6]
    # the library calls the (stubbed) run made, as "lib/Func" with counts
    cnt = {}
    for l in last:
        m = re.match(r"\[([\w.]+)\] (\w+)", l)
        if m: cnt[m.group(1) + "/" + m.group(2)] = cnt.get(m.group(1) + "/" + m.group(2), 0) + 1
    det = details(last)
    return dict(app, first=first, details=det, chain=chain, ran=ran, ran_after_stubs=ran2, ui=kinds2, ctrl=ctrl2, calls=cnt, refused=refused,
                log_tail=[l[:200] for l in last if l.startswith("[")][-6:])

if __name__ == "__main__":
    a = sys.argv[1:]
    out = a[a.index("--out") + 1] if "--out" in a else None
    only = set(a[a.index("--only") + 1].split(",")) if "--only" in a else None
    rows = []
    for l in open(a[0]):
        f = l.rstrip("\n").split("\t")
        if len(f) < 5 or l.startswith("#"): continue
        rows.append(dict(name=f[0], toolkit=f[1], path=f[2], sys=f[3], progdir=f[4]))
    res = []
    for app in rows:
        if only and app["name"] not in only: continue
        if not os.path.exists(os.path.join(ROOT, app["path"])):
            print("SKIP (binary absent): " + app["path"], file=sys.stderr); continue
        r = census(app); res.append(r)
        print(json.dumps(r), flush=True)
    if out: json.dump(res, open(out, "w"), indent=1)
