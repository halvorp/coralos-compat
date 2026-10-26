"""Names + register signatures for AmigaOS libraries (from AROS .conf files + lvo_names.rs + DOpus .fd files) and tag names (AROS headers)."""
import re, os, glob
AROS = "/data/refs/heirloom/AROS"
CONF = {"exec.library": "rom/exec/exec.conf", "dos.library": "rom/dos/dos.conf", "intuition.library": "rom/intuition/intuition.conf",
        "graphics.library": "rom/graphics/graphics.conf", "utility.library": "rom/utility/utility.conf", "layers.library": "rom/hyperlayers/layers.conf",
        "keymap.library": "rom/keymap/keymap.conf", "amigaguide.library": "workbench/libs/amigaguide/amigaguide.conf", "asl.library": "workbench/libs/asl/asl.conf",
        "commodities.library": "workbench/libs/commodities/commodities.conf", "datatypes.library": "workbench/libs/datatypes/datatypes.conf",
        "diskfont.library": "workbench/libs/diskfont/diskfont.conf", "gadtools.library": "workbench/libs/gadtools/gadtools.conf",
        "icon.library": "workbench/libs/icon/icon.conf", "iffparse.library": "workbench/libs/iffparse/iffparse.conf",
        "locale.library": "workbench/libs/locale/locale.conf", "rexxsyslib.library": "workbench/libs/rexxsyslib/rexxsyslib.conf",
        "workbench.library": "workbench/libs/workbench/workbench.conf"}
REGS = {"D0": 0, "D1": 1, "D2": 2, "D3": 3, "D4": 4, "D5": 5, "D6": 6, "D7": 7}
SIG_RE = re.compile(r"^(?P<ret>[\w\s\*]+?)\s*\**\s*(?P<name>\w+)\((?P<args>.*)\)\s*\((?P<regs>[A-Za-z0-9_,\s/]*)\)\s*(#.*)?$")
def load_conf(path):
    sigs = {}; inlist = False
    for line in open(os.path.join(AROS, path), errors="replace"):
        line = line.rstrip("\n")
        if line.startswith("##begin functionlist"): inlist = True; continue
        if line.startswith("##end functionlist"): break
        if not inlist or not line or line[0] in "#.": continue
        m = SIG_RE.match(line.split("#")[0].rstrip())
        if not m: continue
        args = [a.strip() for a in m["args"].split(",")] if m["args"].strip() else []
        regs = [r.strip().upper() for r in re.split(r"[,/]", m["regs"]) if r.strip()]
        full = line.split("(")[0]
        sigs[m["name"]] = {"ret": m["ret"].strip() + ("*" if "*" in full else ""), "args": list(zip(args, regs))}
    return sigs
def load_all_conf():
    return {lib: load_conf(p) for lib, p in CONF.items() if os.path.exists(os.path.join(AROS, p))}
def load_lvo_names(rs="/data/wt/compat-oracle/crates/heirloom-ami68k-engine/src/lvo_names.rs"):
    out = {}
    for m in re.finditer(r'\("([\w\.]+)", &\[(.*?)\]\)', open(rs).read()):
        out[m[1]] = {int(a): b for a, b in re.findall(r'\((-?\d+), "(\w+)"\)', m[2])}
    return out
def load_fd(path):
    """-> (names {lvo: name}, sigs {name: {'args': [(argname, reg)]}}) ; bias from ##bias, ##private/##public respected (all count)."""
    names, sigs = {}, {}; bias = 30
    for line in open(path, errors="replace"):
        line = line.strip()
        if line.startswith("##bias"): bias = int(line.split()[1]); continue
        if line.startswith("##") or line.startswith("*") or not line: continue
        m = re.match(r"(\w+)\((.*?)\)\((.*?)\)", line)
        if not m: continue
        a = [x for x in m[2].split(",") if x]; r = [x.upper() for x in re.split(r"[,/]", m[3]) if x]
        names[-bias] = m[1]; sigs[m[1]] = {"ret": "LONG", "args": list(zip(a, r))}; bias += 6
    return names, sigs
# ---- tag names
def load_tags():
    defs = {}
    hdrs = []
    for d in ("intuition", "dos", "utility", "graphics", "libraries", "workbench", "exec", "datatypes", "diskfont", "gadtools", "utility", "devices"):
        hdrs += glob.glob(f"{AROS}/compiler/include/{d}/*.h")
    for h in hdrs:
        for line in open(h, errors="replace"):
            m = re.match(r"\s*#\s*define\s+(\w+)\s+\(?\s*(\w+|0x[0-9a-fA-F]+)\s*(?:\+\s*(0x[0-9a-fA-F]+|\d+))?\s*\)?\s*(?:/[/*].*)?$", line)
            if m: defs.setdefault(m[1], (m[2], m[3]))
    val = {"TAG_USER": 0x80000000, "TAG_DONE": 0, "TAG_END": 0, "TAG_IGNORE": 1, "TAG_MORE": 2, "TAG_SKIP": 3}
    def ev(n, depth=0):
        if n in val: return val[n]
        if re.fullmatch(r"0x[0-9a-fA-F]+|\d+", n): return int(n, 0)
        if n in defs and depth < 8:
            a, b = defs[n]; x = ev(a, depth + 1)
            if x is None: return None
            return (x + (int(b, 0) if b else 0)) & 0xFFFFFFFF
        return None
    byval = {}
    for n in defs:
        if re.match(r"^(WA|SA|GA|NP|SYS|ASL|ASLFR|GT|GTCB|GTLV|GTST|GTSL|GTSC|GTTX|GTNM|GTIN|GTMX|GTPA|GTBB|BBOX|IA|IDCMP|PGA|STRINGA|LAYOUTA|AO|OBP|ENV|CT|DTA|PDTA|SDTA|ADO|AVT|CBT|ESA|ICA|MA|WBA|WBAPP|AIA|AMA|FNT|PSTAG|DTST|DTM)_", n) and not n.endswith("Dummy") or n.endswith("_Dummy"):
            v = ev(n)
            if v is not None and v >= 0x80000000: byval.setdefault(v, []).append(n)
    return byval
