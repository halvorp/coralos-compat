#!/usr/bin/env bash
# Proves the guest-GL ENGINE side (src/gl_c.rs) can go RED: copies the workspace to a scratch dir, breaks gl_c.rs one way per
# guarantee, and requires the named tests/gl_triangle.rs test to FAIL. Baseline must be GREEN first.
# Usage: crates/heirloom-macppc-engine/mutants-gl.sh
set -u -o pipefail
PATH="$HOME/.cargo/bin:$PATH"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TARGET="${CARGO_BUILD_TARGET:-$(rustc -vV | sed -n 's/^host: //p')}"
WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT
fresh() { rm -rf "$WORK/w"; mkdir -p "$WORK/w"; cp -r "$ROOT/Cargo.toml" "$ROOT/Cargo.lock" "$ROOT/crates" "$WORK/w/" 2>/dev/null; rm -rf "$WORK"/w/crates/*/target; }
run() { (cd "$WORK/w" && CARGO_TARGET_DIR="$WORK/target" cargo test -q --no-fail-fast --target "$TARGET" -p heirloom-macppc-engine --features fixtures --test gl_triangle 2>&1); }
fresh; BASE="$(run)"; if echo "$BASE" | grep -q 'FAILED\|error\['; then echo "RED baseline is not green"; echo "$BASE" | tail -20; exit 1; fi
echo "baseline GREEN"
bad=0
mutant() { # name sed-expr expected-test
  fresh; f="$WORK/w/crates/heirloom-macppc-engine/src/gl_c.rs"; sed -i -E "$2" "$f"
  if cmp -s "$f" "$ROOT/crates/heirloom-macppc-engine/src/gl_c.rs"; then echo "!! mutant '$1' did not change gl_c.rs"; bad=1; return; fi
  out="$(run)"
  if echo "$out" | grep -q "$3 ... FAILED\|error\["; then echo "RED as required: $1 -> $3"; else echo "!! mutant '$1' survived (expected $3 to fail)"; echo "$out" | tail -6; bad=1; fi
}
T=the_pef_triangle_program_draws_through_the_ring_and_the_snapshot_survives_the_guests_scribbling
mutant "unimplemented call admitted"    's/if let Err\(r\) = admit_call\(ENTRIES, f\.0\) \{/if false { let r = heirloom_gl_abi::Refusal::NoGpuAccel;/' an_unimplemented_entry_is_refused_by_name_never_skipped
mutant "GL_VERSION written by hand"     's/version_string\(&caps\)\.unwrap_or_else\(\|_\| String::from\("0\.0 CoralOS Heirloom GL \(no GL version attested\)"\)\)/String::from("1.2 CoralOS Heirloom GL")/' "$T"
mutant "GL_EXTENSIONS written by hand"  's/GL_EXTENSIONS => Some\(extensions_string\(&caps\)\)/GL_EXTENSIONS => Some(String::from("GL_ARB_multitexture GL_EXT_texture_env_combine"))/' "$T"
mutant "destroyed context stays valid"  's/match self\.gl\.ctxs\.remove\(&a0\) \{/match self.gl.ctxs.get(\&a0).copied() {/' a_destroyed_context_handle_is_bad_context_not_a_stale_use
mutant "no refusal without a worker"    's/ret = if !self\.gl\.has_worker\(\) \{/ret = if false {/' without_a_worker_there_is_no_pixel_format_and_the_refusal_is_named
mutant "draw carries no arrays"         's/let descs: Vec<Descriptor> = self\.gl\.enabled\.iter\(\)\.filter_map\(\|k\| self\.gl\.arrays\.get\(k\)\.copied\(\)\)\.collect\(\);/let descs: Vec<Descriptor> = Vec::new();/' "$T"
exit $bad
