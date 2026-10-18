#!/bin/sh
# Build the C guest fixtures: tests/guest/c/NAME.c -> tests/guest/NAME.bin (raw m68k code at address 0, entry = first
# function). Needs the m68k-linux-gnu gcc and binutils. The blobs are committed; the Rust tests wrap them in Hunk files.
# -mpcrel keeps every reference relative, so there are no relocations (NAME.relocs is written empty). String literals and
# constant tables (.rodata) are folded behind the code, in section .text.zz, so the entry stays at address 0.
set -e
cd "$(dirname "$0")"
for src in c/*.c; do
  n=$(basename "$src" .c)
  m68k-linux-gnu-gcc -m68020 -mno-align-int -mpcrel -O1 -Wall -Wextra -Werror -ffreestanding -fno-builtin -fno-toplevel-reorder \
    -fno-asynchronous-unwind-tables -fno-tree-loop-distribute-patterns -S -I c -o "$n.s" "$src"
  sed -E 's/^\s*\.section\s+\.(rodata|data)[^ ]*.*/\t.section .text.zz,"ax"/' "$n.s" > "$n.z.s"
  m68k-linux-gnu-as -m68020 -o "$n.o" "$n.z.s"
  m68k-linux-gnu-ld -e 0 -Ttext=0 -o "$n.elf" "$n.o"
  m68k-linux-gnu-objcopy -O binary "$n.elf" "$n.bin"
  [ -z "$(m68k-linux-gnu-objdump -r "$n.o" | grep "R_68K_32\b")" ] || { echo "$n: unexpected relocations" >&2; exit 1; }
  : > "$n.relocs"
  rm -f "$n.s" "$n.z.s" "$n.o" "$n.elf"
done
