#!/usr/bin/env python3
"""Static evidence for classic Mac applications: the resource types in the resource fork (CNTL/DLOG/DITL/MENU/WIND/LDEF...) and
the Toolbox traps their CODE resources contain, grouped by manager. A-line words found by scanning even offsets of CODE
resources against the engine's trap table (crates/heirloom-mac68k-engine/src/trap_names.rs): data that looks like a trap word over-counts,
so read counts as an upper bound. Usage: corpus-traps-mac.py FILE... (AppleDouble header files, MacBinary .bin, or raw forks)"""
import collections, json, os, re, struct, sys
HERE = os.path.dirname(os.path.abspath(__file__))
TRAPS = {int(m.group(1), 16): m.group(2) for m in re.finditer(r"\(0x([0-9A-F]{4}), \"(\w+)\"\)",
         open(os.path.join(HERE, "..", "crates/heirloom-mac68k-engine/src/trap_names.rs")).read())}
MGR = [("Control", r"^(New|Dispose|Get|Set|Hilite|Show|Hide|Draw|Track|Test|Move|Size|Kill|Update|Find|Drag)?(C?Control|Ctl|ControlTitle|ControlValue|ControlMinimum|ControlMaximum|ControlAction|ControlRef|Controls)\w*$|^(HiliteControl|TrackControl|TestControl|FindControl|DrawControls|UpdtControl|SetCtlValue|GetCtlValue|SetCtlMin|SetCtlMax|GetCtlMin|GetCtlMax|SetCtlAction|GetCtlAction|SetCRefCon|GetCRefCon|DragControl|MoveControl|SizeControl|Draw1Control|KillControls|ShowControl|HideControl|SetCTitle|GetCTitle|NewControl|DisposeControl)$"),
       ("Dialog", r"Dialog|Alert|ParamText|DITL|ModalDialog|GetDItem|SetDItem|SelIText|GetIText|SetIText|FindDItem|UpdtDialog|ErrorSound|InitDialogs|CouldDialog|FreeDialog|ShortenDITL|AppendDITL|CountDITL"),
       ("Menu", r"Menu|InsMenuItem|AppendMenu|AddResMenu|CheckItem|SetItem|GetItem|DisableItem|EnableItem|CalcMenuSize|FlashMenuBar|PopUpMenuSelect|DelMenuItem|InsertResMenu|HiliteMenu|SetMenuBar|GetMenuBar|ClearMenuBar|DrawMenuBar|MenuSelect|MenuKey|InitMenus|NewMenu|GetMHandle|SetMenuFlash|SetMFlash|InsertMenu|DeleteMenu|DisposeMenu|GetRMenu"),
       ("List", r"^(L[A-Z]\w+|PackBits)$|^_?Pack0$"),
       ("TextEdit", r"^TE\w*$|TextEdit|^TEInit$|^TextBox$"),
       ("StandardFile", r"^Pack3$|StandardFile|^SF\w+$|^CustomGet|^StandardPut$"),
       ("Window", r"Window|^(Front|Back|Select|Show|Hide|Hilite|Drag|Grow|Size|Move|Track|Find|Close|Update)\w*(Window|Win|GoAway|Box|Zoom)\w*$|^(DragGrayRgn|ClipAbove|PaintOne|PaintBehind|CalcVis|CheckUpdate|SaveOld|DrawNew|GrowWindow|DragWindow|FrontWindow|ShowHide|InitWindows|GetNewWindow|NewWindow|NewCWindow|GetNewCWindow|CloseWindow|DisposeWindow|SetWTitle|GetWTitle|BeginUpdate|EndUpdate|InvalRect|InvalRgn|ValidRect|ValidRgn|SelectWindow|BringToFront|SendBehind|FindWindow|TrackGoAway|TrackBox|ZoomWindow|SetWRefCon|GetWRefCon|SetWindowPic|GetWindowPic|HiliteWindow|MoveWindow|SizeWindow|DrawGrowIcon|PinRect|WindowRef)$"),
       ("Event", r"Event|GetNextEvent|WaitNextEvent|Button|StillDown|WaitMouseUp|TickCount|GetKeys|FlushEvents|SystemClick|SystemTask|SystemEvent|OSEventAvail|GetMouse|SetEventMask|PostEvent|GetDblTime|GetCaretTime"),
       ("Memory", r"Handle|Ptr|BlockMove|MaxMem|FreeMem|CompactMem|MoreMasters|TempNewHandle|TempFreeMem|SetApplLimit|MaxApplZone|StripAddress|HLock|HUnlock|HPurge|HNoPurge|MemError|SetZone|GetZone|InitZone|ReallocHandle|EmptyHandle|HGetState|HSetState|NewEmptyHandle|RecoverHandle|ApplicZone|SystemZone|HandToHand|PtrToXHand|PtrToHand|ReserveMem|PurgeMem|CompactMem|ResrvMem|MaxBlock|PurgeSpace|BlockMoveData|TopMem"),
       ("Resource", r"Resource|^(GetResource|GetNamedResource|ReleaseResource|LoadResource|DetachResource|AddResource|RemoveResource|UpdateResFile|WriteResource|SetResLoad|CountResources|GetIndResource|CountTypes|GetIndType|OpenResFile|CloseResFile|CurResFile|UseResFile|SizeResource|SetResPurge|ChangedResource|HomeResFile|ResError|GetResInfo|SetResInfo|SetResAttrs|GetResAttrs|RGetResource|Get1Resource|Get1NamedResource|Get1IndResource|Count1Resources|Count1Types|Get1IndType|OpenRFPerm|FSpOpenResFile|InsertResFile|CreateResFile|Unique1ID|UniqueID|MaxSizeRsrc|ReadPartialResource|GetResFileAttrs|SetResFileAttrs|LoadSeg|UnloadSeg)$"),
       ("File/Device", r"^(Open|Close|Read|Write|Control|Status|KillIO|GetVolInfo|Create|Delete|OpenRF|Rename|GetFileInfo|SetFileInfo|UnmountVol|MountVol|Allocate|GetEOF|SetEOF|FlushVol|GetVol|SetVol|FInitQueue|Eject|GetFPos|SetFPos|FlushFile|OpenWD|CloseWD|HOpen|HOpenRF|HCreate|HDelete|HGetFInfo|HSetFInfo|HRename|HGetVol|HSetVol|OpenDriver|CloseDriver|PBHOpen|FSOpen|FSClose|FSRead|FSWrite|GetFInfo|SetFInfo|FSDelete|SetVol|SetEOF|Rename|SetFLock|RstFLock|OpenDeskAcc|CloseDeskAcc|FSMakeFSSpec|FSpOpenDF|FSpCreate|Gestalt|SetFileType|SetFileCreator|FlushEvents)$"),
       ("Font/Text", r"Font|Text|String|CharWidth|StringWidth|TextWidth|GetFontInfo|StdText|Pascal2|NumToString|StringToNum|GetString|GetIndString|Munger|Munge|NewString|SetString|^(TextFont|TextFace|TextMode|TextSize|SpaceExtra|DrawChar|DrawString|DrawText|CharWidth|StringWidth|TextWidth|MeasureText|GetFontInfo|GetFontName|GetFNum|RealFont|SetFontLock|FMSwapFont|InitFonts|FontMetrics|CharExtra|CharByte|CharType|FindWord|FindLine|StdTxMeas|NameToIndex)$"),
       ("Color QD", r"Color|CTab|PixMap|GDevice|Palette|CIcon|CCursor|Pen(Pat)?Pix|BackPixPat|PixPat|RGB|GetPixel|SetGWorld|NewGWorld|GWorld|DisposeGWorld|CopyMask|DeviceLoop|OpColor|HiliteColor|AllocCursor|SeedCTab|Entry2Index|Index2Color|InvertColor|RealColor|SaveEntries|RestoreEntries|CTabChanged|CTabToHandle|GetCTable|GetCCursor|GetCIcon|PlotCIcon|GetMainDevice|GetDeviceList|SetDeviceAttribute|TestDeviceAttribute|MakeITable|ProtectEntry|ReserveEntry|SetEntries|SetEntryUsage|AddComp|DelComp|SetClientID|ResizePalette|PMForeColor|PMBackColor|NewPixMap|DisposePixMap|CopyPixMap|CopyPixPat|NewPixPat|DisposePixPat|MakeRGBPat|FillCRect|FillCOval|FillCRgn|FillCPoly|FillCArc|FillCRoundRect|NewCWindow|GetNewCWindow|NewCDialog|GetCPixel|SetCPixel|CalcCMask|SeedCFill|CharExtra"),
       ("QuickDraw", r"Rect|Oval|Rgn|Poly|Arc|Pen|Pat|Line|Move|Frame|Paint|Erase|Invert|Fill|Port|Clip|Bits|Pic|Cursor|Icon|Point|InitGraf|Random|Scroll|Cmp|BitMap|MapPt|MapRect|MapRgn|MapPoly|ScalePt|AddPt|SubPt|EqualPt|PtInRect|PtToAngle|Pt2Rect|Rect2|UnionRect|SectRect|OffsetRect|InsetRect|SetRect|EmptyRect|EqualRect|StdRect|BackPat|ForeColor|BackColor|ColorBit|HidePen|ShowPen|GetPen|PenSize|PenMode|PenNormal|ClosePoly|OpenPoly|KillPoly|CopyBits|ScrollRect|SetOrigin|Global|Local|Line|ShowCursor|HideCursor|ObscureCursor|InitCursor|SetCursor|GetCursor|GetIcon|PlotIcon|OpenPicture|ClosePicture|DrawPicture|KillPicture|PicComment|StuffHex|GetPixel|SeedFill|CalcMask|ZeroScrap|PutScrap|GetScrap|TextFont|StdBits|StdComment|StdGetPic|StdPutPic|StdLine|StdOval|StdPoly|StdRRect|StdRgn|StdArc|NewRgn|DisposeRgn|OpenRgn|CloseRgn|CopyRgn|SetRectRgn|RectRgn|OffsetRgn|InsetRgn|SectRgn|UnionRgn|DiffRgn|XorRgn|PtInRgn|RectInRgn|EqualRgn|EmptyRgn|SetEmptyRgn|GetMaskTable|OpenPort|InitPort|ClosePort|SetPort|GetPort|GrafDevice|SetPortBits|PortSize|MovePortTo|SetClip|GetClip|ClipRect|DisposePtr"),
       ("Package/Misc", r"Pack\d|Shutdown|Delay|Sound|SysBeep|Notification|SetTrapAddress|GetTrapAddress|Gestalt|Fix|Frac|Long|HiWord|LoWord|BitAnd|BitOr|BitXor|BitNot|BitShift|BitTst|BitSet|BitClr|ScrnBitMap|Date|Time|ReadDateTime|SecondsToDate|DateToSeconds|GetDateTime|SetDateTime|IUDate|IUTime|IUMagString|IUCompString|Zoom|Chain|Launch|ExitToShell|SysError|DebugStr|Debugger|Environs|CountADBs|ADB|SerShk|Serial|Mixed|MixedMode|InitUtil|WriteParam|ReadXPRam|CmpString|RelString|EqualString|UprString|ReadLocation|HWPriv|SlotManager|Slot|Comm|Unimplemented|Mem|CurApName|SCSI|Time|Shift|Pad|Bit|Hi|Lo")]
def split_forks(d):
    if d[:4] == b"\x00\x05\x16\x07":                      # AppleDouble / AppleSingle
        n = struct.unpack(">H", d[24:26])[0]
        for i in range(n):
            eid, off, ln = struct.unpack(">III", d[26 + 12 * i: 38 + 12 * i])
            if eid == 2: return d[off: off + ln]
        return b""
    if len(d) > 128 and d[0] == 0 and 1 <= d[1] <= 63 and d[74] == 0 and d[82] == 0:   # MacBinary
        dl, rl = struct.unpack(">II", d[83:91]); ro = 128 + ((dl + 127) // 128) * 128
        return d[ro: ro + rl]
    return d
def resources(rf):
    if len(rf) < 16: return {}
    do, mo, dl, ml = struct.unpack(">IIII", rf[:16])
    tl_off = mo + struct.unpack(">H", rf[mo + 24: mo + 26])[0]
    ntypes = struct.unpack(">H", rf[tl_off: tl_off + 2])[0] + 1
    out = {}
    for i in range(ntypes):
        t, cnt, ref = struct.unpack(">4sHH", rf[tl_off + 2 + 8 * i: tl_off + 10 + 8 * i])
        for j in range(cnt + 1):
            rid, noff, w = struct.unpack(">hHI", rf[tl_off + ref + 12 * j: tl_off + ref + 12 * j + 8])
            off = do + (w & 0xFFFFFF); ln = struct.unpack(">I", rf[off: off + 4])[0]
            out.setdefault(t.decode("latin1"), []).append((rid, rf[off + 4: off + 4 + ln]))
    return out
def trap_key(w):
    """The engine's table key: OS traps drop the flag bits (NewPtrClear and NewPtr are one key), Toolbox traps keep 10 bits."""
    return 0xA800 | (w & 0x03FF) if w & 0x0800 else 0xA000 | (w & 0x00FF)
def trap_keys(path):
    """The set of distinct trap KEYS appearing in a file's CODE resources (same over-counting caveat as scan)."""
    try: res = resources(split_forks(open(path, "rb").read()))
    except Exception: return set()
    return {trap_key(w) for rid, body in res.get("CODE", []) for i in range(0, len(body) - 1, 2)
            for w in [struct.unpack(">H", body[i: i + 2])[0]] if w & 0xF000 == 0xA000 and w in TRAPS}
def rank(implemented, paths, n=10):
    """The n traps used by the most apps that the engine does not implement: `--rank IMPLEMENTED.txt FILE...` (one registered
    name per line, from Mac::implemented()). Compared by table key, so an alias (GetOSTrapAddress for the GetTrapAddress
    handler, NewPtrClear for NewPtr) counts as implemented exactly as the engine's table sees it."""
    have = {l.strip().lstrip("_") for l in open(implemented)}
    done = {trap_key(w) for w, name in TRAPS.items() if name in have} | {0xA89F}   # $A89F is the Unimplemented marker, not a call
    c = collections.Counter()
    for p in paths: c.update(k for k in trap_keys(p) if k not in done)
    return [("%s ($%04X)" % (TRAPS.get(k, "?"), k), v) for k, v in c.most_common(n)]
def scan(path):
    d = open(path, "rb").read(); rf = split_forks(d)
    try: res = resources(rf)
    except Exception as e: return {"error": str(e)}
    types = {t: len(v) for t, v in res.items()}
    hist = collections.Counter()
    for rid, body in res.get("CODE", []):
        for i in range(0, len(body) - 1, 2):
            w = struct.unpack(">H", body[i: i + 2])[0]
            if w & 0xF000 == 0xA000 and w in TRAPS: hist[TRAPS[w]] += 1
    groups = collections.defaultdict(set)
    for name in hist:
        for g, rx in MGR:
            if re.search(rx, name): groups[g].add(name); break
    return {"resource_types": types, "traps": len(hist), "managers": {g: sorted(v) for g, v in sorted(groups.items())}}
if __name__ == "__main__":
    if sys.argv[1:2] == ["--rank"]:
        for name, k in rank(sys.argv[2], sys.argv[3:], 25): print("%3d apps  _%s" % (k, name))
        sys.exit(0)
    out = {os.path.basename(p): scan(p) for p in sys.argv[1:]}
    print(json.dumps(out, indent=1))
