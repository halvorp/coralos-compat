#!/usr/bin/env bash
# Enforces ruling 17 (docs/doctrine HEIRLOOM-UI-HAS-ONE-SEAM, HEIRLOOM-TRANSLATION-BY-TOOLKIT-NOT-APP) on THIS repo:
#   (a) DEPENDENCY: the normal deps of the three engines are only heirloom-ui-abi, heirloom-* library crates (never another
#       engine), the engine-to-engine chain (macppc -> mac68k) and the pure crates listed in ALLOWED_EXTRA. Anything else (a
#       gadget-library, a UI crate) is RED.
#   (c) LAYERS: a CPU core (heirloom-cpu-*) may use a memory model (its memory contract) and never a format, UI or engine
#       crate; a memory model (heirloom-mem-*) or format reader (heirloom-fmt-*) never a CPU, UI or engine crate.
#   (b) APP NAME: no application identity in crates/*/src outside crates/*/src/bindings/ (comments are provenance, not code).
#   (d) FACE: no crate's src constructs DrawOp::Face except the generic binding interpreter (crates/heirloom-bind), the seam's own
#       codec (crates/heirloom-ui-abi) and the plane mover (crates/heirloom-present): a button face of an app's own chrome comes only
#       from version-bound binding DATA read by that one interpreter (ruling 17; doctrine HEIRLOOM-APP-CHROME-NEEDS-A-BINDING).
# Usage: tools/check-translation-seam.sh [--root DIR]   (default: the repo this script lives in)
#        tools/check-translation-seam.sh --selftest      (plants one violation of each kind in a scratch copy; each must go RED)
set -u -o pipefail
# A gate that cannot run must say RED, never GREEN: find cargo the way the test matrix does, and require jq.
PATH="$HOME/.cargo/bin:$PATH"
command -v "${CARGO:-cargo}" >/dev/null 2>&1 || { echo "RED  seam/deps: cargo not found (PATH=$PATH): the dependency check cannot run"; exit 1; }
command -v jq >/dev/null 2>&1 || { echo "RED  seam/deps: jq not found: the dependency check cannot run"; exit 1; }
ENGINES="heirloom-ami68k-engine heirloom-mac68k-engine heirloom-macppc-engine"
# Pure crates an engine may use besides heirloom-ui-abi and the heirloom-* libraries: the interpreter cores' soft-float, the engine chain, and libm
# (musl's maths as a no_std crate: StdCLib/MathLib natives answer sin/pow/... with it - computation, no host or UI reach).
ALLOWED_EXTRA="heirloom-mac68k-engine musashi softfloat soft-float rustc_apfloat libm"
APPS='dopus|directoryopus|igame|amimon|amiagent|finder|executor'

check_deps() { # $1 = root; prints RED lines, returns 1 if any
  local root=$1 bad=0 e
  for e in $ENGINES; do
    local mp="$root/crates/$e/Cargo.toml"
    [ -f "$mp" ] || { echo "RED  seam/deps: $mp missing"; bad=1; continue; }
    local deps
    deps=$("${CARGO:-cargo}" metadata --format-version 1 --no-deps --manifest-path "$root/Cargo.toml" 2>/dev/null \
      | jq -r --arg mp "$mp" '.packages[] | select(.manifest_path == $mp) | .dependencies[] | select(.kind == null) | .name') \
      || { echo "RED  seam/deps: cargo metadata failed for $e"; bad=1; continue; }
    local d
    for d in $deps; do
      case " $ALLOWED_EXTRA " in *" $d "*) continue ;; esac
      case "$d" in *-engine) ;; heirloom-*) continue ;; esac
      echo "RED  seam/deps: $e depends on '$d' (an engine may link only heirloom-ui-abi, heirloom-* libraries, soft-float: ruling 17 piece 1)"; bad=1
    done
  done
  return $bad
}

check_names() { # $1 = root
  local root=$1 bad=0 f
  while IFS= read -r f; do
    case "$f" in */src/bindings/*) continue ;; esac
    # drop the comment tail of each line (a // with an even number of quotes before it), then look for an app name
    local hits
    hits=$(awk -v re="$APPS" '{ l=$0; n=0
        for (i=1; i<=length(l); i++) { c=substr(l,i,1); if (c=="\"") n++; else if (c=="/" && substr(l,i,2)=="//" && n%2==0) { l=substr(l,1,i-1); break } }
        if (tolower(l) ~ re) print FILENAME ":" NR ": " $0 }' "$f")
    [ -n "$hits" ] && { echo "$hits" | sed 's/^/RED  seam\/names: application identity in code, outside bindings\/: /'; bad=1; }
  done < <(find "$root"/crates -path '*/src/*' -name '*.rs' 2>/dev/null | sort)
  return $bad
}

check_faces() { # $1 = root; DrawOp::Face is made only by the binding interpreter
  local root=$1 bad=0 f hits
  while IFS= read -r f; do
    case "$f" in */crates/heirloom-bind/*|*/crates/heirloom-ui-abi/*|*/crates/heirloom-present/*) continue ;; esac
    hits=$(awk '{ l=$0; n=0
        for (i=1; i<=length(l); i++) { c=substr(l,i,1); if (c=="\"") n++; else if (c=="/" && substr(l,i,2)=="//" && n%2==0) { l=substr(l,1,i-1); break } }
        if (l ~ /DrawOp::Face[ \t]*[{]/) print FILENAME ":" NR ": " $0 }' "$f")
    [ -n "$hits" ] && { echo "$hits" | sed 's/^/RED  seam\/faces: a button face made outside the binding interpreter: /'; bad=1; }
  done < <(find "$root"/crates -path '*/src/*' -name '*.rs' 2>/dev/null | sort)
  return $bad
}

check_layers() { # $1 = root; a lower layer never reaches up
  local root=$1 bad=0 line pkg d
  while IFS=$'\t' read -r pkg d; do
    case "$pkg" in
      heirloom-cpu-*) case "$d" in heirloom-fmt-*|heirloom-ui-abi|*-engine) ;; *) continue ;; esac ;;
      heirloom-mem-*|heirloom-fmt-*) case "$d" in heirloom-cpu-*|heirloom-ui-abi|*-engine) ;; *) continue ;; esac ;;
      *) continue ;;
    esac
    echo "RED  seam/layers: $pkg depends on '$d' (a CPU core never reaches a format/UI/engine crate; memory and format never a CPU/UI/engine)"; bad=1
  done < <("${CARGO:-cargo}" metadata --format-version 1 --no-deps --manifest-path "$root/Cargo.toml" 2>/dev/null \
      | jq -r '.packages[] | .name as $p | .dependencies[] | select(.kind == null) | [$p, .name] | @tsv')
  return $bad
}

run() { local r=0; check_deps "$1" || r=1; check_layers "$1" || r=1; check_names "$1" || r=1; check_faces "$1" || r=1; [ $r = 0 ] && echo "GREEN seam: dependencies, names and faces clean"; return $r; }

selftest() {
  local here rc=0
  here=$(cd "$(dirname "$0")/.." && pwd); TMP=$(mktemp -d); tmp=$TMP; trap 'rm -rf "$TMP"' EXIT
  mkdir -p "$tmp/w" && (cd "$here" && tar --exclude=target --exclude=.git -cf - Cargo.toml Cargo.lock crates) | tar -xf - -C "$tmp/w"
  run "$tmp/w" >/dev/null || { echo "selftest FAIL: the clean tree is RED"; run "$tmp/w"; return 1; }
  # control 1: a gadget-library dependency in an engine
  mkdir -p "$tmp/w/crates/gadget-library/src" && printf '[package]\nname = "gadget-library"\nversion = "0.0.0"\nedition = "2021"\n' > "$tmp/w/crates/gadget-library/Cargo.toml" && : > "$tmp/w/crates/gadget-library/src/lib.rs"
  sed -i 's#^members = \[#members = ["crates/gadget-library", #' "$tmp/w/Cargo.toml"
  sed -i '0,/^\[dependencies\]/s##[dependencies]\ngadget-library = { path = "../gadget-library" }#' "$tmp/w/crates/heirloom-mac68k-engine/Cargo.toml"
  out=$(check_deps "$tmp/w")   # captured first: under pipefail `grep -q` closing the pipe early would fail the pipeline
  if grep -q "heirloom-mac68k-engine depends on 'gadget-library'" <<<"$out"; then echo "selftest ok: planted gadget-library dependency is RED"; else echo "selftest FAIL: gadget-library dependency not caught"; rc=1; fi
  # control 3: a CPU core that reaches a format reader
  cp "$tmp/w/crates/heirloom-cpu-68k/Cargo.toml" "$tmp/cpu.toml"
  sed -i '0,/^\[dependencies\]/s##[dependencies]\nheirloom-fmt-hunk = { path = "../heirloom-fmt-hunk" }#' "$tmp/w/crates/heirloom-cpu-68k/Cargo.toml"
  out=$(check_layers "$tmp/w" 2>&1)
  if grep -q "heirloom-cpu-68k depends on 'heirloom-fmt-hunk'" <<<"$out"; then echo "selftest ok: planted CPU -> format dependency is RED"; else echo "selftest FAIL: CPU -> format dependency not caught"; rc=1; fi
  cp "$tmp/cpu.toml" "$tmp/w/crates/heirloom-cpu-68k/Cargo.toml"
  # control 2: an application name in engine code, then the same name under bindings/ (allowed) and in a comment (allowed)
  mkdir -p "$tmp/w/crates/heirloom-ami68k-engine/src/bindings"
  printf 'pub const X: &str = "DirectoryOpus";\n' > "$tmp/w/crates/heirloom-ami68k-engine/src/planted.rs"
  out=$(check_names "$tmp/w")
  if grep -q "planted.rs" <<<"$out"; then echo "selftest ok: planted app name is RED"; else echo "selftest FAIL: app name not caught"; rc=1; fi
  mv "$tmp/w/crates/heirloom-ami68k-engine/src/planted.rs" "$tmp/w/crates/heirloom-ami68k-engine/src/bindings/planted.rs"
  printf '// provenance: iGame did this\npub const Y: u8 = 1; // Finder too\n' > "$tmp/w/crates/heirloom-ami68k-engine/src/commented.rs"
  if check_names "$tmp/w" >/dev/null; then echo "selftest ok: the name under bindings/ and in comments is allowed"; else echo "selftest FAIL: bindings/ or comment wrongly RED"; check_names "$tmp/w"; rc=1; fi
  # control 4: a face made in engine code (RED, even under the engine's bindings/), then the same face in the binding interpreter
  # (allowed) and a matcher in a comment (allowed)
  printf 'fn f() -> DrawOp { DrawOp::Face { rect, label, pressed: false, accent: None } }\n' > "$tmp/w/crates/heirloom-ami68k-engine/src/face_planted.rs"
  out=$(check_faces "$tmp/w")
  if grep -q "face_planted.rs" <<<"$out"; then echo "selftest ok: planted face in an engine is RED"; else echo "selftest FAIL: face in an engine not caught"; rc=1; fi
  mv "$tmp/w/crates/heirloom-ami68k-engine/src/face_planted.rs" "$tmp/w/crates/heirloom-ami68k-engine/src/bindings/face_planted.rs"
  out=$(check_faces "$tmp/w")
  if grep -q "bindings/face_planted.rs" <<<"$out"; then echo "selftest ok: a face in an engine's bindings/ is RED (bindings are data)"; else echo "selftest FAIL: a face in an engine's bindings/ not caught"; rc=1; fi
  mkdir -p "$tmp/w/crates/heirloom-bind/src" && mv "$tmp/w/crates/heirloom-ami68k-engine/src/bindings/face_planted.rs" "$tmp/w/crates/heirloom-bind/src/face_planted.rs"
  printf '// a DrawOp::Face { .. } is a binding'"'"'s\npub const Z: u8 = 1;\n' > "$tmp/w/crates/heirloom-ami68k-engine/src/face_comment.rs"
  if check_faces "$tmp/w" >/dev/null; then echo "selftest ok: the face in the binding interpreter and in a comment is allowed"; else echo "selftest FAIL: interpreter face or comment wrongly RED"; check_faces "$tmp/w"; rc=1; fi
  return $rc
}

case "${1:-}" in
  --selftest) selftest ;;
  --root) run "$(cd "$2" && pwd)" ;;
  "") run "$(cd "$(dirname "$0")/.." && pwd)" ;;
  *) echo "usage: $0 [--selftest | --root DIR]" >&2; exit 2 ;;
esac
