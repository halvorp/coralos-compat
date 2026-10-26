#!/usr/bin/env bash
# tools/check-fmt-real-corpus.sh [--selftest] - every heirloom-fmt-* reader has a test that reads a REAL file.
# A panel-written reader for SIS and E32 passed tests built on its own guessed layout (2026-10-25, docs/preflight-lessons
# PANEL-GUESSED-FORMAT in coralos): a test whose input the reader's author also wrote proves only self-consistency. So each
# crates/heirloom-fmt-*/tests/ must read a corpus named by an environment variable ending in _CORPUS (the files are the
# owner's and are never committed). tools/fmt-real-corpus.baseline lists the readers that still lack one; it only SHRINKS.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
BASE=tools/fmt-real-corpus.baseline
has_corpus_test() { grep -rqE '[A-Z0-9_]+_CORPUS' "$1/tests" 2>/dev/null; }
check() {   # $1 = crates root
  local bad=0 d n
  for d in "$1"/heirloom-fmt-*/; do
    d=${d%/}; n=$(basename "$d")
    if has_corpus_test "$d"; then
      if grep -qx "$n" "$BASE"; then echo "FMT-CORPUS: $n now has a real-corpus test - remove it from $BASE"; bad=1; fi
    elif ! grep -qx "$n" "$BASE"; then
      echo "FMT-CORPUS FAIL: $n has no test reading a real file (an env var *_CORPUS in tests/); add one, skipping loudly when unset"; bad=1
    fi
  done
  return $bad
}
if [[ "${1:-}" == --selftest ]]; then
  t=$(mktemp -d); trap 'rm -rf "$t"' EXIT
  mkdir -p "$t/heirloom-fmt-probe/tests"; echo '#[test] fn t() {}' > "$t/heirloom-fmt-probe/tests/a.rs"
  if check "$t" >/dev/null; then echo "fmt-corpus selftest: FAIL (a reader with only synthetic tests passed)"; exit 1; fi
  echo 'let _ = std::env::var("HEIRLOOM_PROBE_CORPUS");' >> "$t/heirloom-fmt-probe/tests/a.rs"
  check "$t" >/dev/null || { echo "fmt-corpus selftest: FAIL (a reader with a corpus test was refused)"; exit 1; }
  echo "fmt-corpus selftest: OK (a synthetic-only reader is RED, a corpus-reading one GREEN)"; exit 0
fi
check crates && echo "FMT-CORPUS OK: every heirloom-fmt-* reader reads a real file or is named in $BASE"
