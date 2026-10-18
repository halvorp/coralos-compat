#!/usr/bin/env python3
"""Extract every file of an HFS disk image (.dsk/.img, 400K/800K/1.4M or larger) as MacBinary II files (what retro-resfork reads),
with no mounting and no execution: machfs (pure Python, MIT) parses the volume. Needs `pip install machfs` (use a venv).
Usage: mac-extract-hfs.py IMAGE OUTDIR     -> OUTDIR/<path with '/' as '_'>.bin plus an index line per file on stdout."""
import os, struct, sys
import machfs

def crc16(b):
    c = 0
    for x in b:
        c ^= x << 8
        for _ in range(8):
            c = ((c << 1) ^ 0x1021) & 0xFFFF if c & 0x8000 else (c << 1) & 0xFFFF
    return c

def macbinary(name, ftype, creator, flags, data, rsrc):
    n = name.encode("mac_roman")[:63]
    h = bytearray(128)
    h[1] = len(n); h[2:2 + len(n)] = n
    h[65:69] = ftype; h[69:73] = creator; h[73] = (flags >> 8) & 0xFF
    h[83:87] = struct.pack(">I", len(data)); h[87:91] = struct.pack(">I", len(rsrc))
    h[101] = flags & 0xFF
    h[122] = 129; h[123] = 129
    h[124:126] = struct.pack(">H", crc16(h[:124]))
    pad = lambda x: x + b"\0" * (-len(x) % 128)
    return bytes(h) + pad(data) + pad(rsrc)

def walk(folder, path):
    for name, item in folder.items():
        if isinstance(item, machfs.Folder):
            yield from walk(item, path + [name])
        else:
            yield path + [name], item

def main(img, out):
    raw = open(img, "rb").read()
    if len(raw) > 84 and raw[82:84] == b"\x01\x00" and raw[0] < 64:       # DiskCopy 4.2: 84-byte header, data, then tags
        raw = raw[84:84 + struct.unpack(">I", raw[64:68])[0]]
    vol = machfs.Volume(); vol.read(raw)
    os.makedirs(out, exist_ok=True)
    for parts, f in walk(vol, []):
        safe = "_".join(p.replace("/", "_") for p in parts).replace("\0", "")
        fl = getattr(f, "flags", 0)
        open(os.path.join(out, safe + ".bin"), "wb").write(macbinary(parts[-1], f.type, f.creator, fl, f.data, f.rsrc))
        print("%s\t%s/%s\tdata=%d\trsrc=%d" % ("/".join(parts), f.type.decode("mac_roman"), f.creator.decode("mac_roman"), len(f.data), len(f.rsrc)))

if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
