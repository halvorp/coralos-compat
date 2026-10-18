#!/usr/bin/env python3
"""Render the data tables of docs/CORPUS.md from the census JSON files (corpus-census.py, corpus-strings.py, corpus-census-mac.py).
Usage: corpus-report.py AMIGA_CENSUS.json STRINGS.json [MAC_CENSUS.json] > tables.md"""
import collections, json, re, sys
CORE = {"exec", "dos", "intuition", "graphics", "utility", "locale", "icon", "workbench", "stdc", "stdcio", "posixc", "layers", "gadtools",
        "muimaster", "asl", "iffparse", "diskfont", "commodities", "datatypes", "keymap", "cybergraphics", "rexxsyslib", "mathieeedoubbas",
        "mathieeedoubtrans", "mathieeesingbas", "mathieeesingtrans"}
def clean(lib): return re.sub(r"^(?:[A-Za-z_]*Nu|[0-9a-zA-Z]{0,3}Nu)", "", lib).removesuffix(".library")
def short(x, n=60): return x if len(x) <= n else x[: n - 1] + "~"
def md(rows, head):
    out = ["| " + " | ".join(head) + " |", "|" + "|".join("---" for _ in head) + "|"]
    out += ["| " + " | ".join(str(c) for c in r) + " |" for r in rows]
    return "\n".join(out)

amiga = {d["name"]: d for d in map(json.loads, open(sys.argv[1])) } if sys.argv[1].endswith(".log") else {d["name"]: d for d in json.load(open(sys.argv[1]))}
strs = json.load(open(sys.argv[2]))
print("### Amiga census, one row per binary\n")
rows = []
for n, d in amiga.items():
    m = re.match(r"ran (\d+) instructions, halted=(\w+)", d["ran"])
    ran = f"{int(m.group(1)):,} insns" + ("" if m.group(2) == "true" else " (budget, still running)") if m else d["ran"]
    first = d["first"] if d["first"].startswith(("LOAD", "none")) is False else d["first"].replace("none (no unimplemented call reached)", "none")
    ui = ", ".join(f"{k}x{v}" for k, v in d["ui"].items() if k in ("OpenWindow", "AddControl", "SetMenuStrip", "PresentContent")) or "-"
    refused = short("; ".join(sorted({re.sub(r"^\[([\w.]+)\] (\w+) REFUSED:? ?", r"\1/\2 ", r) for r in d["refused"]})), 90) or "-"
    rows.append((n, d["toolkit"], ran, first, ui, refused))
print(md(rows, ["app", "toolkit", "ran", "NEXT-NEEDED (first)", "window UI calls", "refused"]))
print("\n### Toolkit usage matrix, Amiga (name evidence from the hunk + what the census run created)\n")
rows = []
for n, d in amiga.items():
    s = strs.get(n, {"libs": [], "mui": [], "boopsi": [], "custom": []})
    libs = sorted({clean(l) for l in s["libs"]} - CORE)
    core = sorted({clean(l) for l in s["libs"]} & {"gadtools", "muimaster", "asl", "datatypes", "commodities", "iffparse", "locale", "workbench", "cybergraphics"})
    det = d.get("details", {})
    rows.append((n, "GT" if "gadtools" in core else "", ",".join(sorted(set(det.get("gt_kinds", [])) | set(s.get("gt_kinds_static", {})))) or "-", len(s["mui"]) or "-",
                 ", ".join(s["boopsi"]) or "-", ", ".join(s["custom"][:3]) or "-", "asl" if "asl" in core else "", ", ".join(core and [c for c in core if c in ("datatypes", "commodities", "iffparse", "workbench", "cybergraphics")]) or "-",
                 ", ".join(libs[:5]) or "-"))
print(md(rows, ["app", "GadTools", "GadTools kinds (run time + static heuristic)", "MUI classes named", "BOOPSI system classes named", "custom classes named", "ASL", "other system libs", "disk/3rd-party libs"]))

lib_n, mui_n, bo_n, kind_n, call_n, mkc = (collections.Counter() for _ in range(6))
for n, s in strs.items():
    for l in {clean(x) for x in s["libs"]}: lib_n[l] += 1
    for m in set(s["mui"]): mui_n[re.sub(r"^Nu", "", m)] += 1
    for b in set(s["boopsi"]): bo_n[b] += 1
for n, d in amiga.items():
    for k in set(d.get("details", {}).get("gt_kinds", [])) | set(strs.get(n, {}).get("gt_kinds_static", {})): kind_n[k] += 1
    for k in set(d.get("details", {}).get("mui_classes", [])): call_n["MUI " + k] += 1
    for k in d.get("details", {}).get("boopsi_made", []): mkc[k] += 1
    for k, v in d.get("calls", {}).items():
        if k.split("/")[0] in ("gadtools", "intuition", "muimaster", "asl", "graphics") and v: call_n[k] += 1
ref_n = collections.Counter()
for n, d in amiga.items():
    seen = set()
    for r in d["refused"] + ([d["first"]] if d["first"] and not d["first"].startswith(("none", "LOAD")) else []):
        k = re.sub(r"^\[([\w.]+)\] (\w+) REFUSED:? ?", r"\1/\2: ", r)
        k = re.sub(r"(CloseLibrary|DupLockFromFH|SetAttrsA|DisposeObject|Close|Write on): 0x[0-9a-f]+", r"\1: <addr>", k)[:150]
        if k.startswith(("muimaster.library/MUI_CreateCustomClass", "dos.library/SetVBuf")): k = "NEXT-NEEDED " + k
        seen.add(k)
    for k in seen: ref_n[k] += 1
print("### What the engine refuses most (count of apps; last run of each, first 6 refusals + the NEXT-NEEDED)\n")
print(md([(v, k) for k, v in ref_n.most_common(26)], ["apps", "refusal"]))
print(f"\n### Aggregates over {len(strs)} Amiga binaries\n")
print("Libraries named (count of apps): " + ", ".join(f"{k} {v}" for k, v in lib_n.most_common(24)) + "\n")
print("MUI classes named (count of apps): " + ", ".join(f"{k} {v}" for k, v in mui_n.most_common(30)) + "\n")
print("BOOPSI system classes named: " + ", ".join(f"{k} {v}" for k, v in bo_n.most_common()) + "\n")
print("GadTools kinds (apps using each; run time + static heuristic): " + (", ".join(f"{k} {v}" for k, v in kind_n.most_common()) or "-") + "\n")
print("Custom BOOPSI classes made at run time (MakeClass name : super): " + (", ".join(f"{k}" for k in mkc) or "-") + "\n")
print("Library functions called at run time by the binaries that reached them, by app count: " + ", ".join(f"{k} {v}" for k, v in call_n.most_common(24)) + "\n")
ROWS = [  # (translation-table row, role it feeds, predicate over (named MUI classes, named BOOPSI system classes, libs, custom, census))
    ("MUI Application + Window (shell, title, close, menus)", "window", lambda m, b, l, c, d: "Application.mui" in m or "Window.mui" in m),
    ("MUI Group (horizontal/vertical/virtual, frame, title)", "group", lambda m, b, l, c, d: "Group.mui" in m),
    ("MUI Text (labels, text buttons via MUIO_Label/Button)", "label/button", lambda m, b, l, c, d: "Text.mui" in m),
    ("MUI String (text field)", "field", lambda m, b, l, c, d: "String.mui" in m or "BetterString.mcc" in m or "HotkeyString.mcc" in m),
    ("MUI List + Listview (rows, selection) incl. NList/NListtree", "list", lambda m, b, l, c, d: bool(m & {"List.mui", "Listview.mui", "NList.mcc", "NListview.mcc", "NListtree.mcc", "Listtree.mcc", "IconList.mui"})),
    ("GadTools gadget kinds (CreateGadgetA)", "button/checkbox/choice/field/slider/list", lambda m, b, l, c, d: "gadtools" in l),
    ("ASL requesters (file/font/screenmode)", "modal dialog", lambda m, b, l, c, d: "asl" in l or "Popasl.mui" in m),
    ("BOOPSI system gadget/image classes (propg, strg, buttong, sysi, frame)", "scrollbar/field/button/image", lambda m, b, l, c, d: bool(b - {"rootclass"})),
    ("MUI Register (tabs)", "choice (tabs)", lambda m, b, l, c, d: "Register.mui" in m),
    ("MUI Slider / Gauge / Scale", "slider/gauge", lambda m, b, l, c, d: bool(m & {"Slider.mui", "Gauge.mui", "Scale.mui", "Knob.mui"})),
    ("MUI Menustrip / Menu / Menuitem", "menu", lambda m, b, l, c, d: bool(m & {"Menustrip.mui", "Menu.mui", "Menuitem.mui"})),
    ("MUI Cycle / Radio (choice)", "choice", lambda m, b, l, c, d: bool(m & {"Cycle.mui", "Radio.mui"})),
    ("MUI Image / Dtpic / IconImage", "image", lambda m, b, l, c, d: bool(m & {"Image.mui", "Dtpic.mui", "IconImage.mcc", "Pixmap.mui"})),
    ("MUI Scrollgroup / Virtgroup / Scrollbar", "scrollbar", lambda m, b, l, c, d: bool(m & {"Scrollgroup.mui", "Virtgroup.mui", "Scrollbar.mui"})),
    ("MUI prefs window family (PrefsEditor/SystemPrefsWindow MCCs)", "window + groups", lambda m, b, l, c, d: bool(m & {"PrefsEditor.mcc", "SystemPrefsWindow.mcc"})),
    ("MUI TextEditor / TextEditor.mcp", "multiline field", lambda m, b, l, c, d: "TextEditor.mcc" in m),
    ("MUI Balance / Rectangle spacing", "(layout only)", lambda m, b, l, c, d: bool(m & {"Balance.mui", "Rectangle.mui"})),
    ("Custom BOOPSI classes (MakeClass; DOpus-style)", "plane unless ancestor used", lambda m, b, l, c, d: bool(d.get("details", {}).get("boopsi_made")) or any(x.startswith("dopus") for x in c)),
    ("EasyRequest (system alert/confirm)", "modal dialog", lambda m, b, l, c, d: any("EasyRequest" in x for x in d.get("chain", []))),
    ("Commodities (Exchange/hotkeys; no UI)", "(none)", lambda m, b, l, c, d: "commodities" in l),
    ("datatypes (picture/text/sound viewers)", "image/plane", lambda m, b, l, c, d: "datatypes" in l),
]
rank = []
for label, role, pred in ROWS:
    users = []
    for n, d in amiga.items():
        s_ = strs.get(n)
        if not s_: continue
        m = {re.sub(r"^Nu", "", x) for x in s_["mui"]}
        if pred(m, set(s_["boopsi"]), {clean(x) for x in s_["libs"]}, s_["custom"], d): users.append(n)
    rank.append((len(users), label, role, users))
rank.sort(key=lambda r: -r[0])
print("### Translation-table rows ranked by app count (Amiga, name evidence + census)\n")
print(md([(i + 1, c, l, r, ", ".join(u[:4]) + (" ..." if len(u) > 4 else "")) for i, (c, l, r, u) in enumerate(rank)], ["#", "apps", "table row", "role it feeds", "e.g."]))
print()
if len(sys.argv) > 3:
    mac = {d["name"]: d for d in json.load(open(sys.argv[3]))}
    print("### Mac census, one row per binary\n")
    rows = []
    for n, d in mac.items():
        mg = "; ".join(f"{k}: {','.join(v[:6])}" for k, v in d["managers"].items() if k not in ("segment", "res"))
        rows.append((n, d["arch"], short(d["ran"].replace("ran ", ""), 40), d["next"], short(mg, 110)))
    print(md(rows, ["app", "arch", "ran", "NEXT-NEEDED", "Toolbox managers called"]))

if len(sys.argv) > 4:
    ms = json.load(open(sys.argv[4]))
    macc = {d["name"]: d for d in json.load(open(sys.argv[3]))}
    print("\n### Mac matrix: resource types (68k: static) and distinct Toolbox routines per manager\n")
    print("68k rows count A-line traps found in CODE resources (an upper bound: data that looks like a trap word counts, and the Retro68 runtime's own\nlibc file I/O inflates every Retro68 sample); PPC rows count the PEF's imports from InterfaceLib (exact). `Init*` and `TEInit` do not count.\n")
    show = ["WIND", "ALRT", "DLOG", "DITL", "CNTL", "MENU", "MBAR", "CDEF", "WDEF", "LDEF", "STR#", "PICT", "ICN#", "TEXT"]
    mgrs = ["Window", "Control", "Dialog", "Menu", "List", "TextEdit", "StandardFile", "QuickDraw", "Color QD", "Font/Text", "Event", "Resource", "File/Device", "Memory", "Package/Misc"]
    def real(names): return [x for x in names if not x.startswith("Init") and x != "TEInit"]
    rows, per_app = [], {}
    import os
    for n, d in macc.items():
        fn = os.path.basename(d["path"])
        if d["arch"] == "68k" and fn in ms and "error" not in ms[fn]:
            v = ms[fn]; res = [v["resource_types"].get(t, "") for t in show]; mg = {m: len(real(v["managers"].get(m, []))) for m in mgrs}
        elif d["arch"] == "ppc":
            sv = ms.get("ppc/" + fn, {}).get("resource_types", {}); res = [sv.get(t, "") for t in show]; mg = {m: len(real([re.sub(r"\[.*", "", x) for x in d.get("imports", {}).get(m, [])])) for m in mgrs}
        else: continue
        per_app[n] = mg
        rows.append([n, d["arch"]] + res + [mg[m] or "" for m in mgrs])
    print(md(rows, ["app", "arch"] + show + ["Window", "Control", "Dialog", "Menu", "List", "TE", "StdFile", "QD", "ColorQD", "Font/Text", "Event", "Res", "File", "Mem", "Misc"]))
    cnt = collections.Counter()
    for n, mg in per_app.items():
        for m, c in mg.items():
            if c > 0: cnt[m] += 1
    print(f"\nManagers used beyond their Init* call, by app count (of {len(per_app)} Mac apps): " + ", ".join(f"{m} {c}" for m, c in cnt.most_common()) + "\n")
