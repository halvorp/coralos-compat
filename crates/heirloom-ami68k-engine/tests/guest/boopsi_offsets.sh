#!/bin/sh
# Regenerate boopsi_offsets.txt from boopsi_offsets.c with the m68k compiler (needs m68k-linux-gnu-gcc). Committed output.
set -e
cd "$(dirname "$0")"
m68k-linux-gnu-gcc -m68020 -mno-align-int -S -o boopsi_offsets.s boopsi_offsets.c
awk '/^[A-Z_]+:$/ {n=substr($1,1,length($1)-1); getline; if ($1==".long") print n, $2; else if ($1==".zero") print n, 0}' boopsi_offsets.s > boopsi_offsets.txt
rm -f boopsi_offsets.s
