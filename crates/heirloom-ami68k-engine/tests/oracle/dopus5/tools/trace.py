"""Library-call tracer over Copperline's control protocol: pc breakpoints on library jump-table slots.
usage: from trace import Tracer; t = Tracer(cc, out); t.setup(); t.run(until_seconds)"""
import re, time, sys, struct
sys.path.insert(0, __file__.rsplit("/", 1)[0])
import sigs

WL = {  # whitelist per library (None = everything)
 "exec.library": "OpenLibrary OldOpenLibrary CloseLibrary OpenDevice CloseDevice OpenResource FindPort AddPort RemPort CreateMsgPort DeleteMsgPort CreateIORequest DeleteIORequest PutMsg GetMsg ReplyMsg WaitPort Wait Signal AddTask RemTask AddLibrary MakeLibrary SetFunction FindSemaphore AddSemaphore AddResource AddDevice FindResident InitResident FindName SetSignal AddIntServer SetIntVector AllocSignal FreeSignal SetExcept CreatePool DoIO SendIO WaitIO AbortIO CheckIO TaggedOpenLibrary".split(),
 "dos.library": "LoadSeg NewLoadSeg UnLoadSeg CreateNewProc CreateProc RunCommand SystemTagList Execute Open Close Lock UnLock Examine ExNext ExAll ExAllEnd DeleteFile Rename CreateDir SetProtection SetComment MatchFirst MatchNext MatchEnd GetVar SetVar FindVar AssignAdd AssignLock FindDosEntry LockDosList UnLockDosList GetDeviceProc NameFromLock NameFromFH CurrentDir Delay ReadArgs Info ParentDir DupLock ExamineFH SetFileDate SetMode WaitForChar IsFileSystem SetProgramDir SetProgramName GetProgramName ChangeMode SetFileSize StartNotify EndNotify Cli FindCliProc SetCurrentDirName GetCurrentDirName DateStamp ParsePattern ParsePatternNoCase MatchPattern MatchPatternNoCase Seek".split(),
 "intuition.library": "OpenWindow OpenWindowTagList CloseWindow OpenScreen OpenScreenTagList CloseScreen LockPubScreen UnlockPubScreen LockPubScreenList PubScreenStatus MakeClass AddClass RemoveClass FreeClass NewObjectA DisposeObject SetAttrsA SetGadgetAttrsA AddGList AddGadget RemoveGList RemoveGadget RefreshGList SetMenuStrip ClearMenuStrip ResetMenuStrip ModifyIDCMP GetScreenDrawInfo FreeScreenDrawInfo WindowToFront WindowToBack ScreenToFront ScreenToBack SetWindowTitles EasyRequestArgs AutoRequest BuildEasyRequestArgs SysReqHandler ActivateWindow SizeWindow MoveWindow ChangeWindowBox WindowLimits SetPointer ClearPointer OpenWorkBench CloseWorkBench LockIBase UnlockIBase ObtainGIRPort ReleaseGIRPort ActivateGadget ScreenPosition ShowTitle BeginRefresh EndRefresh RefreshWindowFrame NextObject FindClass ZipWindow SetWindowPointerA GetAttr SetEditHook SetMouseQueue CurrentTime DisplayBeep MoveScreen OffMenu OnMenu OffGadget OnGadget ItemAddress IntuiTextLength PrintIText ClearDMRequest ViewAddress ViewPortAddress ScreenToBack IsWindowVisible".split(),
 "graphics.library": "OpenFont CloseFont AskFont SetFont AddFont RemFont AllocBitMap FreeBitMap InitRastPort InitBitMap OpenMonitor ObtainBestPenA ReleasePen GetVPModeID FindDisplayInfo GetDisplayInfoData LoadRGB4 LoadRGB32 SetRGB4 SetRGB32 GetRGB4 AllocRaster FreeRaster InitVPort MakeVPort LoadView BestModeIDA AllocSpriteDataA AddFont OpenFont ScrollRaster ClipBlit BltBitMapRastPort WritePixelArray8 ReadPixelArray8 TextLength SetSoftStyle AskSoftStyle GetBitMapAttr ObtainPen WaitTOF WaitBlit".split(),
 "gadtools.library": None, "icon.library": None, "workbench.library": None, "asl.library": None, "diskfont.library": None,
 "locale.library": None, "datatypes.library": None, "commodities.library": None, "amigaguide.library": None, "rexxsyslib.library": None,
 "layers.library": "CreateUpfrontLayer CreateBehindLayer DeleteLayer MoveLayer SizeLayer UpfrontLayer BehindLayer MoveSizeLayer ScrollLayer InstallClipRegion NewLayerInfo DisposeLayerInfo SetLayerInfoBounds".split(),
 "utility.library": "".split(),
}
VOIDISH = {"void", "VOID"}
PRINTABLE = re.compile(rb"^[\x20-\x7e\xa0-\xff\t\n]*$")

class Tracer:
    def __init__(self, cc, out, extra_libs=None, trace_all_dopus=True):
        self.c = cc; self.out = open(out, "w"); self.conf = sigs.load_all_conf(); self.lvo = sigs.load_lvo_names()
        self.tags = sigs.load_tags(); self.addr = {}  # slot address -> (lib, func, sig)
        self.pend = {}; self.pendbp = {}; self.taskname = {}; self.libs = set(); self.stops = 0; self.line_no = 0
        self.libbases = {}; self.extra = extra_libs or {}  # lib name -> (names{lvo:name}, sigs, whitelist-or-None)
        self.cnt = {}

    # ---- memory helpers
    def rd(self, a, n):
        if a < 4 or (a > 0x20000000 and not (0x00E00000 <= a <= 0x01000000)): return b""
        try: return bytes.fromhex(self.c.call("mem.read", {"addr": a, "len": n, "encoding": "hex"})["data"])
        except Exception: return b""
    def l32(self, a):
        b = self.rd(a, 4); return int.from_bytes(b, "big") if len(b) == 4 else 0
    def w16(self, a):
        b = self.rd(a, 2); return int.from_bytes(b, "big") if len(b) == 2 else 0
    def cstr(self, a, n=100):
        b = self.rd(a, n)
        if not b: return None
        b = b.split(b"\0")[0]
        return b.decode("latin-1") if PRINTABLE.match(b) and b else None
    def tname(self, task):
        if task in self.taskname: return self.taskname[task]
        n = self.cstr(self.l32(task + 10), 40) or f"task@{task:x}"
        self.taskname[task] = n; return n

    # ---- breakpoint plumbing
    def addbp(self, addr, lib, func, sig):
        if addr in self.addr: return
        r = self.c.call("break.add", {"kind": "pc", "addr": addr})
        if "id" in r: self.addr[addr] = (lib, func, sig)
    def setup(self):
        syms = self.c.call("symbols.rom")["symbols"]
        n = 0
        for s in syms:
            lib = s["module"]
            if lib not in WL: continue
            wl = WL[lib]
            if wl is not None and s["name"] not in wl: continue
            sig = self.conf.get(lib, {}).get(s["name"])
            self.addbp(s["vector"], lib, s["name"], sig); n += 1
            self.libs.add(lib)
        return n
    def scan_lib(self, base):
        """add breakpoints for a freshly opened disk/stub library from its jump table"""
        name = self.cstr(self.l32(base + 10), 60)
        if not name or base in self.libbases: return
        self.libbases[base] = name
        neg = self.w16(base + 16)
        if name in self.extra: names, sg, wl = self.extra[name]
        else: names, sg, wl = self.lvo.get(name, {}), self.conf.get(name, {}), WL.get(name, "ALL")
        if name in self.libs and name not in self.extra: return
        if wl == "ALL": wl = None
        tab = self.rd(base - neg, neg)
        n = 0
        for k in range(1, neg // 6 + 1):
            slot = base - 6 * k; ent = tab[neg - 6 * k: neg - 6 * k + 6]
            if ent[:2] != b"\x4e\xf9": continue
            fn = names.get(-6 * k, f"LVO{-6*k}")
            if wl is not None and fn not in wl: continue
            self.addbp(slot, name, fn, sg.get(fn)); n += 1
        self.log(f"## library {name} @{base:x} neg={neg} traced={n}")

    # ---- formatting
    def tagname(self, v, fn):
        c = self.tags.get(v)
        if not c: return f"{v:#x}"
        pre = {"OpenWindowTagList": "WA_", "OpenWindow": "WA_", "OpenScreenTagList": "SA_", "CreateNewProc": "NP_", "SystemTagList": "SYS_"}.get(fn)
        if pre:
            f = [x for x in c if x.startswith(pre)]
            if f: return f[0]
        return "|".join(c[:2])
    def tags_str(self, p, fn, lib):
        if not p: return "NULL"
        out = []
        for i in range(40):
            b = self.rd(p + 8 * i, 8)
            if len(b) < 8: break
            t, d = struct.unpack(">II", b)
            if t == 0: break
            if t == 2: out.append(f"MORE:{d:#x}"); break
            if t == 1: continue
            nm = self.tagname(t, fn); s = None
            if d > 0x1000 and d < 0x20000000 and re.search(r"Title|Name|Text|Label|File|Dir|Pattern|Font|Str|Arguments|Cmd|CurrentDir|Input|Output|PubScreenName|PubName", nm):
                s = self.cstr(d, 60)
            out.append(f"{nm}={'%r' % s if s else ('%#x' % d if d > 9 else d)}")
        return "{" + ", ".join(out) + "}"
    def val(self, typ, v, fn, lib):
        t = typ.replace("const", "").replace("CONST", "")
        if "TagItem" in t: return self.tags_str(v, fn, lib)
        if re.search(r"STRPTR|char\s*\*|UBYTE\s*\*|\bSTRPTR\b|ClassID", t) and "STRPTR *" not in t and v > 0x1000:
            s = self.cstr(v, 80)
            if s is not None: return repr(s)
        if "MsgPort" in t and v > 0x1000:
            s = self.cstr(self.l32(v + 10), 40)
            return f"port:{s!r}@{v:#x}" if s else f"{v:#x}"
        if "TextAttr" in t and v > 0x1000:
            return f"TextAttr({self.cstr(self.l32(v), 40)!r}, y={self.w16(v+4)}, style={self.rd(v+6,1).hex()}, flags={self.rd(v+7,1).hex()})"
        if "NewWindow" in t and v > 0x1000:
            b = self.rd(v, 48)
            if len(b) >= 48:
                l, tp, w, h = struct.unpack(">hhhh", b[:8]); idcmp, flags = struct.unpack(">II", b[10:18]); title = struct.unpack(">I", b[26:30])[0]
                return f"NewWindow(l={l},t={tp},w={w},h={h},idcmp={idcmp:#x},flags={flags:#x},title={self.cstr(title,60)!r})"
        if "Message *" in t and v > 0x1000 and fn in ("PutMsg", "ReplyMsg"):
            b = self.rd(v, 40)
            if len(b) >= 40:
                rp = struct.unpack(">I", b[14:18])[0]; ln = struct.unpack(">H", b[18:20])[0]
                return f"msg@{v:#x}(type={b[8]},replyport={rp:#x},len={ln},data={b[20:40].hex()})"
        return f"{v:#x}" if v > 9 or v < 0 else str(v)
    def retval(self, sig, d0, fn):
        r = sig["ret"] if sig else "?"
        if fn in ("OpenLibrary", "OldOpenLibrary", "FindPort", "FindTask", "LoadSeg", "NewLoadSeg"): return f"{d0:#x}" if d0 else "NULL"
        return f"{d0:#x}" if d0 > 9 else str(d0)
    def log(self, s):
        self.out.write(s + "\n"); self.line_no += 1
    def regval(self, g, rn):
        return g["d"][int(rn[1])] if rn[0] == "D" else g["a"][int(rn[1])]

    def handle(self, g, sec):
        pc = g["pc"]; sp = g["a"][7]
        if pc in self.addr:
            lib, fn, sig = self.addr[pc]
            ret = self.l32(sp); task = self.l32(self.l32(4) + 276)
            parts = []
            if sig:
                for (a, rn) in sig["args"]:
                    parts.append(f"{a.split()[-1].strip('*')}={self.val(a.rsplit(' ',1)[0] if ' ' in a else '', self.regval(g, rn), fn, lib)}")
            else:
                parts = [f"d0={g['d'][0]:#x}", f"a0={g['a'][0]:#x}", f"a1={g['a'][1]:#x}"]
            line = f"{sec:8.3f} {self.tname(task)} {lib[:-8] if lib.endswith('.library') else lib}.{fn}({', '.join(parts)}) <{ret:#x}>"
            self.stops += 1; self.seq = getattr(self, "seq", 0) + 1; line = f"S{self.seq:06d} " + line; key = (lib, fn); self.cnt[key] = self.cnt.get(key, 0) + 1
            noret = sig and sig["ret"] in VOIDISH
            if noret or ret < 0x100:
                self.log(line); return
            ent = {"line": line, "fn": fn, "lib": lib, "sig": sig, "a1": g["a"][1], "d0": g["d"][0]}
            k = (ret, sp + 4)
            if k not in self.pend:
                if ret not in self.pendbp:
                    r = self.c.call("break.add", {"kind": "pc", "addr": ret}); self.pendbp[ret] = [r.get("id"), 0]
                self.pendbp[ret][1] += 1
            self.pend[k] = ent
        else:
            k = (pc, sp)
            ent = self.pend.get(k)
            if not ent: return
            del self.pend[k]
            self.pendbp[pc][1] -= 1
            if self.pendbp[pc][1] <= 0:
                self.c.call("break.remove", {"id": self.pendbp[pc][0]}); del self.pendbp[pc]
            d0 = g["d"][0]
            self.log(f"{ent['line']} = {self.retval(ent['sig'], d0, ent['fn'])}")
            if ent["fn"] in ("OpenLibrary", "OldOpenLibrary") and d0:
                self.scan_lib(d0)
    def run(self, until, max_stops=10**9, wall=3000):
        t0 = time.time(); self.final = None
        while time.time() - t0 < wall and self.stops < max_stops:
            r = self.c.call("run_until", {"seconds": until})
            if r.get("reason") != "breakpoint":
                self.final = r; break
            g = self.c.call("regs.get"); self.handle(g, r["seconds"])
        for k, e in list(self.pend.items()): self.log(e['line'] + ' = <NO RETURN YET>')
        self.out.flush()
        return self.final
