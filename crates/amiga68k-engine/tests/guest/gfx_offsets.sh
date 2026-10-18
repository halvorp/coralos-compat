#!/bin/sh
# Regenerate gfx_offsets.txt from gfx_offsets.c with the m68k compiler (needs m68k-linux-gnu-gcc). Committed output.
set -e
cd "$(dirname "$0")"
m68k-linux-gnu-gcc -m68020 -mno-align-int -S -o gfx_offsets.s gfx_offsets.c
awk '/^[A-Z_]+:$/ {n=substr($1,1,length($1)-1); getline; if ($1==".long") print n, $2; else if ($1==".zero") print n, 0}' gfx_offsets.s > gfx_offsets.txt
rm -f gfx_offsets.s
