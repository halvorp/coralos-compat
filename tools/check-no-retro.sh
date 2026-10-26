#!/usr/bin/env bash
# tools/check-no-retro.sh [--selftest] - the subsystem is Heirloom: the word "retro" (any case) may only SHRINK in this repo.
# Counts case-insensitive occurrences in tracked files, except dated consult records (docs/consults/: history, kept as
# written), the sentinel's adjudication log, and the proper name of the Retro68 toolchain (autc04/Retro68). FAILS when the count exceeds tools/no-retro.baseline; when it is below,
# says so and asks for the baseline to be lowered in the same change. --selftest plants a file and must see it go RED.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
BASE=tools/no-retro.baseline

count() {
  git grep -I -i -o -P -e 'retro(?!68)' -- . ':!docs/consults/' ':!tools/sentinel-adjudications.ndjson' ':!tools/no-retro.baseline' \
    ':!tools/check-no-retro.sh' 2>/dev/null | wc -l || true   # no match is exit 1, a count of 0
}

if [[ "${1:-}" == --selftest ]]; then
  before=$(count)
  probe=tools/.no-retro-selftest-probe
  printf 'retro\n' > "$probe"
  git add -N "$probe"
  after=$(count)
  git rm -q --cached "$probe"; rm -f "$probe"
  if (( after == before + 1 )); then echo "no-retro selftest: OK (a planted occurrence is counted: $before -> $after)"; exit 0; fi
  echo "no-retro selftest: FAIL (planted one occurrence, count went $before -> $after)"; exit 1
fi

n=$(count)
max=$(tr -dc 0-9 < "$BASE")
if (( n > max )); then
  echo "NO-RETRO FAIL: $n occurrences of \"retro\" > baseline $max. The subsystem is Heirloom; new names never use the word."
  git grep -I -i -n -P -e 'retro(?!68)' -- . ':!docs/consults/' ':!tools/sentinel-adjudications.ndjson' ':!tools/no-retro.baseline' \
    ':!tools/check-no-retro.sh' | head -20
  exit 1
fi
if (( n < max )); then echo "no-retro: OK, $n < baseline $max - lower $BASE to $n in this change"; else echo "no-retro: OK ($n, baseline $max)"; fi
