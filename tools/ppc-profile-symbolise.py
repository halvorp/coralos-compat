#!/usr/bin/env python3
"""Symbolise macppc's guest-PC profile ([HEIRLOOM-PROFILE*] lines, cpu-tier "jitprofile") against a static function map.

    ppc-profile-symbolise.py <ring-or-log> <functions.tsv> [categories.tsv] [--top N]

The engine prints, per profile, a header naming each fragment's code section (f<i>="<name>" sec<k> <base>+<size>) and the
hottest function STARTS it found on the device as f<i>+<section offset> with their share of JIT ticks (t=) and of samples
(s=, ~ the share of guest instructions). A start found on the device is a guess (a bl target or a code pointer in data); this
snaps each to the map's function that CONTAINS it, matched by (fragment name, section offset) - never by guest address, so a
different load order cannot mislabel - merges the starts that fall in one function, and prints the top functions with their
names and categories, then the natives as printed. The map (a static analysis of the user's own copy of the program) is
input only: nothing of it is written anywhere but stdout.
"""
import bisect
import csv
import re
import sys
from collections import defaultdict

HDR = re.compile(r'\[HEIRLOOM-PROFILE frames=(\d+) profiled=(\d+) samples=(\d+) chunk_mean=\d+ jit_us/frame=(\d+)\] map: (.*)')
FRAG = re.compile(r'f(\d+)="([^"]*)" sec(\d+) (0x[0-9a-f]+)\+(\d+)')
CELL = re.compile(r'(f(\d+)\+(0x[0-9a-f]+)|outside) t=([\d.]+)% s=([\d.]+)% n=(\d+)')
NATIVE = re.compile(r'\[HEIRLOOM-PROFILE-NATIVE frames=(\d+)\] (.*)')


def load_map(path):
    by_frag = defaultdict(list)  # fragment -> sorted [(offset, size, name)]
    with open(path, newline='') as f:
        for r in csv.DictReader(f, delimiter='\t'):
            off = int(r['sec_offset'], 16)
            by_frag[r['fragment']].append((off, int(r['size'] or 0), r.get('demangled') or r.get('name') or ''))
    for v in by_frag.values():
        v.sort()
    return by_frag


def load_categories(path):
    cats = {}
    if not path:
        return cats
    with open(path, newline='') as f:
        for r in csv.DictReader(f, delimiter='\t'):
            cats[(r['fragment'], int(r['sec_offset'], 16))] = (r.get('category', ''), r.get('suggested_name', ''), r.get('hle_candidate', ''))
    return cats


def containing(funcs, off):
    i = bisect.bisect_right(funcs, (off, float('inf'), '')) - 1
    if i < 0:
        return None
    start, size, name = funcs[i]
    if size and off >= start + size:
        return None
    return start, size, name


def main(argv):
    args = [a for a in argv[1:] if not a.startswith('--')]
    top = int(argv[argv.index('--top') + 1]) if '--top' in argv else 40
    if '--top' in argv:
        args.remove(str(top))
    if len(args) < 2:
        print(__doc__)
        return 2
    log, fmap = args[0], load_map(args[1])
    cats = load_categories(args[2] if len(args) > 2 else None)
    profiles = []  # (header, frags, cells, natives)
    cur = None
    for line in open(log, errors='replace'):
        m = HDR.search(line)
        if m:
            frags = {int(i): name for i, name, *_ in FRAG.findall(m.group(5))}
            cur = {'frames': int(m.group(1)), 'profiled': int(m.group(2)), 'samples': int(m.group(3)), 'jit_us': int(m.group(4)), 'frags': frags, 'cells': [], 'natives': ''}
            profiles.append(cur)
            continue
        if cur and '[HEIRLOOM-PROFILE-TOP' in line:
            cur['cells'].extend(CELL.findall(line))
        n = NATIVE.search(line)
        if cur and n:
            cur['natives'] = n.group(2)
    if not profiles:
        print('no [HEIRLOOM-PROFILE ...] lines in', log)
        return 1
    for p in profiles:
        merged = defaultdict(lambda: [0.0, 0.0, 0])
        unmatched = 0.0
        for whole, fi, off, t, s, n in p['cells']:
            if whole == 'outside':
                key = ('outside', 0, 0, 'outside every code section')
            else:
                frag = p['frags'].get(int(fi), f'f{fi}')
                hit = containing(fmap.get(frag, []), int(off, 16))
                if hit is None:
                    unmatched += float(t)
                    key = (frag, int(off, 16), 0, '(not in the map)')
                else:
                    key = (frag, hit[0], hit[1], hit[2])
            acc = merged[key]
            acc[0] += float(t)
            acc[1] += float(s)
            acc[2] += int(n)
        print(f"# profile at frame {p['frames']}: {p['profiled']} frames, {p['samples']} samples, JIT {p['jit_us']} us/frame; "
              f"the printed top covers {sum(v[0] for v in merged.values()):.1f}% of JIT ticks ({unmatched:.1f}% not in the map)")
        print('rank\tjit%\tinsn%\tsamples\tjit_us/frame\tfragment\tsec_offset\tsize\tname\tcategory\tsuggested\thle')
        for k, (key, v) in enumerate(sorted(merged.items(), key=lambda kv: -kv[1][0])[:top]):
            frag, off, size, name = key
            cat = cats.get((frag, off), ('', '', ''))
            print(f"{k + 1}\t{v[0]:.1f}\t{v[1]:.1f}\t{v[2]}\t{p['jit_us'] * v[0] / 100:.0f}\t{frag}\t{off:#x}\t{size}\t{name}\t{cat[0]}\t{cat[1]}\t{cat[2]}")
        print('# natives:', p['natives'])
        print()
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))
