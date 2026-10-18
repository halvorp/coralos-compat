#!/bin/sh
# Rebuild the guest test blobs: raw 68k code at address 0 (*.bin) and the offsets of its 32-bit pointers (*.relocs).
# Needs m68k-linux-gnu-binutils. The blobs are committed; tests/disklib.rs wraps them in Hunk files.
set -e
cd "$(dirname "$0")"
for n in testlib prog progdos; do
  m68k-linux-gnu-as -m68020 -o $n.o $n.s
  m68k-linux-gnu-ld -e 0 -Ttext=0 -o $n.elf $n.o
  m68k-linux-gnu-objcopy -O binary $n.elf $n.bin
  m68k-linux-gnu-objdump -r $n.o | awk '/R_68K_32/ {print strtonum("0x"$1)}' > $n.relocs
  rm -f $n.o $n.elf
done
