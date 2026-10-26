#!/usr/bin/env python3
"""Census of the classic Mac corpus (binaries live outside git). APPS.tsv columns: name, arch (68k|ppc), path, source-note.
For each app runs mac68k-run --census or macppc-run --census --run and records NEXT-NEEDED, the instruction count and the
Toolbox managers/traps it called, taken from the engine's own log tags ("[toolbox] InitGraf", "[menu] InitMenus", ...).
Usage: corpus-census-mac.py APPS.tsv [--out FILE]"""
import json, os, re, subprocess, sys
ROOT = os.environ.get("MAC_CORPUS_ROOT", "/data/build/corpus-bin")
BIN = os.environ.get("HEIRLOOM_BIN", "/data/wt/compat-corpus/target/release")
def run(arch, path, machine=""):
    cmd = [BIN + ("/mac68k-run" if arch == "68k" else "/macppc-run"), "--census"]
    if arch == "68k":
        cmd += ["--insns", os.environ.get("MAC_INSNS", "50000000")]   # compressed resources run a guest decompressor: 5M is not enough
        if machine or os.environ.get("MAC_MACHINE"): cmd += ["--machine", machine or os.environ["MAC_MACHINE"]]   # the TSV's 5th column (a 68000-era app: mac-plus) wins
        if os.environ.get("MAC_IDLE", "0") != "0": cmd += ["--idle", os.environ["MAC_IDLE"]]   # virtual 60 Hz ticks passed while the app waits for an event (null events)   # a machine profile (mac-ii default, mac-plus, mac-se)
    if arch == "ppc":
        cmd += ["--run", "--insns", "50000000", "--idle", os.environ.get("MAC_IDLE", "6")]
        if os.environ.get("MAC_LIBDIR"): cmd += ["--libdir", os.environ["MAC_LIBDIR"]]   # PEF libraries that sit beside the program (Warcraft III: Miles Carbon Library ...)
    p = subprocess.run(cmd + [path], capture_output=True, text=True, timeout=600)
    return p.stdout.splitlines(), p.stderr.strip().splitlines()
def load_groups():
    """Group Toolbox routine names by manager with the same table as corpus-traps-mac.py."""
    import importlib.util
    spec = importlib.util.spec_from_file_location("traps", os.path.join(os.path.dirname(os.path.abspath(__file__)), "corpus-traps-mac.py"))
    m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
    return m.MGR

def census(app):
    path = app["path"] if app["path"].startswith("/") else os.path.join(ROOT, app["path"])
    lines, err = run(app["arch"], path, app.get("machine", ""))
    nxt = next((l.split(" ", 1)[1] for l in lines if l.startswith("NEXT-NEEDED")), "?")
    ran = next((l for l in lines if l.startswith("ran ")), (err[:1] or [""])[0])
    traps = {}
    for l in lines:
        m = re.match(r"\[(\w+)\] (\w+)", l)
        if m and m.group(1) not in ("mem",): traps.setdefault(m.group(1), {}).setdefault(m.group(2), 0); traps[m.group(1)][m.group(2)] += 1
    ui = {}
    for l in lines:
        if l.startswith("ui: "):
            k = re.match(r"ui: (\w+)", l).group(1); ui[k] = ui.get(k, 0) + 1
    imports = {}
    unbound = 0
    if app["arch"] == "ppc":
        # the RUN's stop (the REFUSED line) is what matters; the static census lists every unbound import, not the one reached
        run_stop = next((m.group(1) for m in (re.match(r"^\s*\|?\s*REFUSED (.*)$", l) for l in lines) if m), None)
        unbound = sum(1 for l in lines if l.startswith("NEXT-NEEDED"))
        if run_stop: nxt = run_stop   # the static import census: "BOUND  InterfaceLib/Name (tvector) <- ..." lines
        groups = load_groups()
        for l in lines:
            m = re.match(r"^(\w[\w-]*)\s+(\w+)/(\w+)\s+\(", l)
            if not m: continue
            name = m.group(3)
            g = next((g for g, rx in groups if re.search(rx, name)), "Other")
            imports.setdefault(g, []).append(name + ("" if m.group(1) == "BOUND" else "[" + m.group(1) + "]"))
        traps = {g: {n: 1 for n in v} for g, v in imports.items()} if not traps else traps
    return dict(app, next=nxt, unbound_imports=unbound, imports={g: sorted(set(v)) for g, v in imports.items()}, ran=ran, managers={k: sorted(v) for k, v in traps.items()}, ui=ui, stderr=err[:3])
if __name__ == "__main__":
    a = sys.argv[1:]; out = a[a.index("--out") + 1] if "--out" in a else None
    res = []
    for l in open(a[0]):
        f = l.rstrip("\n").split("\t")
        if len(f) < 3 or l.startswith("#"): continue
        app = dict(name=f[0], arch=f[1], path=f[2], note=f[3] if len(f) > 3 else "", machine=f[4] if len(f) > 4 else "")
        full = app["path"] if app["path"].startswith("/") else os.path.join(ROOT, app["path"])
        if not os.path.exists(full): print("SKIP (binary absent): " + full, file=sys.stderr); continue
        r = census(app); res.append(r); print(json.dumps(r), flush=True)
    if out: json.dump(res, open(out, "w"), indent=1)
