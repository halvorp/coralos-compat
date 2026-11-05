#!/usr/bin/env python3
"""Split macppc's guest-PC profile by frame weight: which functions grow in the HEAVY frames.

    ppc-profile-frames.py <frames.txt> <functions.tsv> [categories.tsv] [--from SWAP] [--top N] [--heavy Q]

<frames.txt> is the WC3 harness's WC3_PROFILE_FRAMES output (tests/gl_triangle.rs, with WC3_PROFILE): per guest frame
`F <swaps> <guest insns>`, then `S "<fragment>" <section> <offset> <insns> <ticks> <samples>` per sampled function start.
Frames from swap SWAP (default 900) are split into HEAVY (guest insns >= quantile Q, default 0.9) and MEDIAN (0.4..0.6);
per function (each sampled start snapped to the map's containing function, as ppc-profile-symbolise.py does): mean guest
instructions a frame in each set, the difference and its share of the heavy-median gap, the OLS slope against the frame's
instructions, then the largest in heavy frames and the largest by sampled ticks. The map (a static analysis of the user's own
copy of the program) is input only: nothing of it is written anywhere but stdout."""
import sys, bisect, csv
from collections import defaultdict
args=[a for a in sys.argv[1:]]
def opt(n,d):
    if n in args: i=args.index(n); v=args[i+1]; del args[i:i+2]; return type(d)(v)
    return d
frm=opt('--from',900); top=opt('--top',30); hq=opt('--heavy',0.9)
pf,fn=args[0],args[1]; cat=args[2] if len(args)>2 else None
funcs=defaultdict(list)
for r in csv.DictReader(open(fn),delimiter='\t'):
    funcs[r['fragment']].append((int(r['sec_offset'],16),int(r['size'] or 0),r.get('demangled') or r.get('name') or r.get('ghidra_name') or ''))
for v in funcs.values(): v.sort()
cats={}
if cat:
    for r in csv.DictReader(open(cat),delimiter='\t'): cats[(r['fragment'],int(r['sec_offset'],16))]=(r.get('category',''),r.get('suggested_name',''))
def sym(frag,off):
    v=funcs.get(frag,[]); i=bisect.bisect_right(v,(off,1<<62,''))-1
    if i<0: return (frag,off,'?')
    s,sz,n=v[i]
    if sz and off>=s+sz: return (frag,off,'?')
    return (frag,s,n)
frames=[]; cur=None
for l in open(pf):
    if l[0]=='F':
        _,sw,ins=l.split(); cur=[int(sw),int(ins),defaultdict(lambda:[0,0])]; frames.append(cur)
    elif l[0]=='S' and cur:
        # S "frag" sec off insns ticks samples  (frag may contain spaces, quoted)
        if l.startswith('S outside'): frag='outside'; rest=l.split()[2:]
        else: q2=l.rindex('"'); frag=l[3:q2]; rest=l[q2+1:].split()
        sec,off,ins,tk,sm=rest[0],rest[1],int(rest[2]),int(rest[3]),int(rest[4])
        k=sym(frag,int(off,16)) if frag!='outside' else ('outside',0,'')
        a=cur[2][k]; a[0]+=ins; a[1]+=tk
# a frame line is written at the swap seen AFTER the frame's samples: its F carries that frame's insns and the S lines after it
fr=[f for f in frames if f[0]>=frm]
w=sorted(f[1] for f in fr); n=len(w)
q=lambda x:w[min(n-1,int(n*x))]
hcut=q(hq); lo,hi=q(0.4),q(0.6)
H=[f for f in fr if f[1]>=hcut]; M=[f for f in fr if lo<=f[1]<=hi]
print(f"frames {n} from swap {frm}: insns p10 {q(.1)} p50 {q(.5)} p90 {q(.9)} p95 {q(.95)} max {w[-1]}; heavy(>={hcut}) {len(H)} median({lo}..{hi}) {len(M)}")
def mean(F,k,i): return sum(f[2][k][i] for f in F if k in f[2])/max(1,len(F))
keys=set(); [keys.update(f[2].keys()) for f in fr]
mH=sum(f[1] for f in H)/len(H); mM=sum(f[1] for f in M)/len(M); gap=mH-mM
tH=sum(sum(a[1] for a in f[2].values()) for f in H)/len(H); tM=sum(sum(a[1] for a in f[2].values()) for f in M)/len(M)
print(f"mean insns heavy {mH:.0f} median {mM:.0f} gap {gap:.0f}; mean ticks heavy {tH:.0f} median {tM:.0f}")
xs=[f[1] for f in fr]; mx=sum(xs)/n; vx=sum((x-mx)**2 for x in xs)
rows=[]
for k in keys:
    h=mean(H,k,0); m=mean(M,k,0); ht=mean(H,k,1); mt=mean(M,k,1)
    ys=[f[2][k][0] if k in f[2] else 0 for f in fr]; my=sum(ys)/n
    slope=sum((x-mx)*(y-my) for x,y in zip(xs,ys))/vx
    rows.append((h-m,h,m,ht,mt,slope,my,k))
rows.sort(key=lambda r:-r[0])
print(f"{'dIns/frame':>10} {'%gap':>6} {'heavy':>9} {'median':>9} {'slope':>6} {'tH-tM':>8}  function")
for d,h,m,ht,mt,sl,my,k in rows[:top]:
    c=cats.get((k[0],k[1]),('',''))
    print(f"{d:10.0f} {100*d/gap:6.1f} {h:9.0f} {m:9.0f} {sl:6.3f} {ht-mt:8.0f}  {k[0]}+{k[1]:#x} {k[2]} [{c[0]}] {c[1]}")
print("--- largest in heavy frames (absolute)")
rows.sort(key=lambda r:-r[1])
for d,h,m,ht,mt,sl,my,k in rows[:top]:
    c=cats.get((k[0],k[1]),('',''))
    print(f"{d:10.0f} {100*h/mH:6.1f} {h:9.0f} {m:9.0f} {sl:6.3f} {ht:8.0f}  {k[0]}+{k[1]:#x} {k[2]} [{c[0]}] {c[1]}")

print("--- by host ticks in heavy frames (JIT time estimate): ticks/insn vs the mean")
mt=tH/mH
rows.sort(key=lambda r:-r[3])
for d,h,m,ht,mt_,sl,my,k in rows[:top]:
    c=cats.get((k[0],k[1]),('',''))
    print(f"{100*ht/tH:6.1f}% ticks {ht:8.0f} (median {mt_:8.0f}) insns {h:8.0f} cost/insn x{(ht/max(h,1))/mt:4.2f}  {k[0]}+{k[1]:#x} {k[2]} [{c[0]}] {c[1]}")
