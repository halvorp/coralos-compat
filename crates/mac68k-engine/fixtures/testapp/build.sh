#!/bin/sh
# Rebuild the checked-in fixture binaries (needs binutils for m68k: m68k-linux-gnu-as/objcopy).
set -e
cd "$(dirname "$0")"
for pair in oracle:oracle main:code1 menus:code2 memtest:memtest restest:restest evtest:evtest recttest:recttest qdapp:qdapp; do
  src=${pair%%:*}; out=${pair##*:}
  m68k-linux-gnu-as -m68020 "$src.s" -o "/tmp/$src.o"
  m68k-linux-gnu-objcopy -O binary -j .text "/tmp/$src.o" "$out.bin"
done
ls -l *.bin
