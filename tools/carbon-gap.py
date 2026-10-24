#!/usr/bin/env python3
"""Rank the CarbonLib gap: which of CarbonLib 1.4's exports (the Mac OS 9.2.1 CD export list) this engine does not bind, weighted by how many measured programs import them.
Usage: carbon-gap.py [--exports FILE] [--bin DIR] [--top N]   (reads the ppc rows of tools/corpus-apps-mac*.tsv; rows whose file is absent are skipped)
A program counts for its STATIC import census (macppc-run --census without --run), so a name used only on a path the run has not reached is still counted.
Weights: a Warcraft III family program (warcraft3 TSV) counts 3, any other PPC program 1. Output: per-family totals of unbound exports, then the top unbound names by weight."""
import collections, os, re, subprocess, sys
ROOT = os.environ.get("MAC_CORPUS_ROOT", "/data/build/corpus-bin")
HERE = os.path.dirname(os.path.abspath(__file__))
args = sys.argv[1:]
def opt(name, default):
    return args[args.index(name) + 1] if name in args else default
EXPORTS = opt("--exports", os.path.join(ROOT, "mac/macos921/exports/CarbonLib_4508d0fa.txt"))
BIN = opt("--bin", "target/release")
TOP = int(opt("--top", "60"))
if not os.path.exists(EXPORTS): sys.exit(f"SKIPPED: {EXPORTS} is not present")
run = BIN + "/macppc-run"
bound = collections.OrderedDict()
for l in subprocess.run([run, "--bind-list", EXPORTS, "CarbonLib"], capture_output=True, text=True).stdout.splitlines():
    st, cls, name = l.split("\t"); bound[name] = st == "BOUND"
# family = the Toolbox manager a name belongs to, by prefix (deliberately coarse)
FAMILIES = [("CoreFoundation", r"^(CF|__CF|kCF)"), ("OpenGL/AGL", r"^(agl|gl[A-Z])"), ("HIToolbox views/controls", r"^(HI|Control|.*Control$|GetControl|SetControl|Embed|Root)"),
    ("Windows", r"Window"), ("Menus", r"Menu"), ("Navigation Services", r"^Nav"), ("Apple Events", r"^(AE|Create.*Desc|Get.*Desc)"), ("QuickDraw/Regions/Pixmaps", r"^(Qd|QD|Get.*Port|Set.*Port|NewRgn|.*Rgn|.*PixMap|.*GWorld|.*CIcon|.*Pict)"),
    ("Files/Aliases/FSRef", r"^(FS|PB|Alias|.*Alias|File|.*Volume|.*Folder)"), ("Printing", r"^PM"), ("Text/Unicode/TEC", r"^(TEC|Text|UC|Convert|.*Unicode|.*Encoding|ATS|Font|.*Font)"),
    ("Sound/QuickTime/Components", r"^(Snd|Sound|SP|QT|Movie|Component|Open.*Component|Close.*Component)"), ("Drag Manager", r"^(Drag|.*Drag)"), ("Carbon Events", r"^(.*EventHandler|.*EventTarget|.*Event(Parameter|Class|Kind)|Create.*Event|Send.*Event|Receive.*Event|Run.*Loop)"),
    ("Threads/MP/Timer", r"^(MP|Thread|.*Timer|Delay)"), ("Memory/Resources", r"^(New|Dispose|Get|Set|Release|Load|Detach|Add|Remove)?(Ptr|Handle|Resource|Res|Zone|Mem|1.*Resource)")]
def family(n):
    for f, rx in FAMILIES:
        if re.search(rx, n): return f
    return "other"
unbound = [n for n, ok in bound.items() if not ok]
fam = collections.Counter(family(n) for n in unbound)
# measured programs
progs = []
for tsv, w in (("corpus-apps-mac-warcraft3.tsv", 3), ("corpus-apps-mac-garden.tsv", 1), ("corpus-apps-mac.tsv", 1)):
    for l in open(os.path.join(HERE, tsv)):
        if l.startswith("#") or not l.strip(): continue
        c = l.rstrip("\n").split("\t")
        if len(c) >= 3 and c[1] == "ppc":
            p = c[2] if c[2].startswith("/") else os.path.join(ROOT, c[2])
            if os.path.exists(p): progs.append((c[0], p, w))
weight = collections.Counter(); who = collections.defaultdict(list)
libdir = os.path.join(ROOT, "mac/warcraft3/libs")
for name, path, w in progs:
    cmd = [run, "--census"] + (["--libdir", libdir] if os.path.isdir(libdir) else []) + [path]
    out = subprocess.run(cmd, capture_output=True, text=True, timeout=600).stdout
    seen = set()
    for l in out.splitlines():
        m = re.match(r"NEXT-NEEDED\s+(\S+)/(\S+) \(", l)
        if m and m.group(1) in ("CarbonLib", "InterfaceLib") and m.group(2) not in seen: seen.add(m.group(2))
    for n in seen: weight[n] += w; who[n].append(name)
print(f"CarbonLib 1.4 exports: {len(bound)}; bound by this engine: {sum(bound.values())}; unbound: {len(unbound)}; measured PPC programs: {len(progs)}")
print("\n== unbound exports by family (all of CarbonLib) ==")
for f, n in fam.most_common(): print(f"{n:6d}  {f}")
print(f"\n== unbound names the measured programs import, by weight (top {TOP}) ==")
ranked = sorted(weight.items(), key=lambda kv: (-kv[1], kv[0]))
for n, w in ranked[:TOP]: print(f"{w:4d}  {n:40s} {family(n):28s} {len(who[n])} program(s)")
print(f"\n{len(ranked)} distinct unbound names are imported by the measured programs")
