"""compact.py RAW... > out : merge raw tracer logs, order by call sequence, keep DOpus's own calls (tasks dopus*/DO_*/main),
drop calls made from ROM code (internal AROS calls), drop high-frequency noise (counted in a footer)."""
import re, sys, collections
NOISE = {"GetMsg", "Signal", "SetSignal", "FindName", "WaitBlit", "TextLength", "GetBitMapAttr", "SetFont", "NextObject", "GetString", "GetSemaphore",
         "FreeSemaphore", "CheckTimer", "StartTimer", "CheckIO", "SendIO", "AbortIO", "WaitIO", "LockIBase", "UnlockIBase", "LockPubScreenList", "ClipBlit",
         "AllocRaster", "FreeRaster", "InstallClipRegion", "AskSoftStyle", "SetSoftStyle", "DateStamp", "FormatString", "IsAlNum", "ItoaU64", "GetWindowID",
         "InitRastPort", "InitBitMap", "LockDosList", "UnLockDosList", "FindDosEntry", "GetDeviceProc", "FreeDeviceProc", "WaitPort", "Wait", "GetIconFlags",
         "GetScreenDrawInfo", "FreeScreenDrawInfo", "SetAttrsA", "GetAttr", "RefreshGList", "Delay", "CurrentTime", "SetWindowTitles", "AskFont", "CloseFont", "ObtainBestPenA", "ReleasePen", "AllocMemH", "FreeMemH"}
KEEP_FONT = set()
TASK_OK = re.compile(r"^(dopus|DO_|RunProg:launch)")
LINE = re.compile(r"^S(\d+)\s+([\d.]+)\s+(.+?) (\w+)\.(\w+)\((.*)\) <(0x[0-9a-f]+)>(?: = (.*))?$")
rows = []
for f in sys.argv[1:]:
    for l in open(f, errors="replace"):
        m = LINE.match(l.rstrip("\n"))
        if m: rows.append(m.groups())
rows.sort(key=lambda r: int(r[0]))
seen = set(); dropped = collections.Counter(); n = 0
out = []
for seq, t, task, lib, fn, args, ret, rv in rows:
    if (seq) in seen: continue
    seen.add(seq)
    if not TASK_OK.match(task): continue
    r = int(ret, 16)
    if 0xE00000 <= r < 0x1000000: dropped["(ROM-internal)"] += 1; continue
    if fn in NOISE and fn not in KEEP_FONT: dropped[f"{lib}.{fn}"] += 1; continue
    if fn == "Wait" or fn == "WaitPort": continue
    args = re.sub(r"\s+", " ", args)
    out.append(f"{float(t):7.2f} {task} {lib}.{fn}({args})" + (f" = {rv}" if rv and rv != "<NO RETURN YET>" else (" = ..." if rv else "")))
print("# t(s) task lib.Func(args) = result   [DOpus-originated calls only; ROM-internal and noisy calls elided, see footer]")
print("\n".join(out))
print("# elided (count):", ", ".join(f"{k} x{v}" for k, v in dropped.most_common()))
