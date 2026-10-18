#!/bin/sh
# Rebuild the guest test blobs: raw 68k code at address 0 (*.bin) and the offsets of its 32-bit pointers (*.relocs).
# Needs m68k-linux-gnu-binutils. The blobs are committed; tests/disklib.rs wraps them in Hunk files.
set -e
cd "$(dirname "$0")"
# mccstub.s: testlib renamed and versioned like a MUI class, so a program asking for TextEditor.mcc v15 finds a library.
sed -e 's/test\.library/TextEditor.mcc/g' -e 's/^\(\s*\.word\s*\)3\b/\115/' testlib.s > mccstub.s
for n in testlib prog progdos progmisc mccstub; do
  m68k-linux-gnu-as -m68020 -o $n.o $n.s
  m68k-linux-gnu-ld -e 0 -Ttext=0 -o $n.elf $n.o
  m68k-linux-gnu-objcopy -O binary $n.elf $n.bin
  m68k-linux-gnu-objdump -r $n.o | awk '/R_68K_32/ {print strtonum("0x"$1)}' > $n.relocs
  rm -f $n.o $n.elf
done
rm -f mccstub.s
