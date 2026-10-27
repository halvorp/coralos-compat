#!/usr/bin/env bash
# Proves each Phase 0 negative control can go RED: copies the crate to a scratch dir, breaks the implementation ONE way
# per guarantee, and requires (a) the baseline to be GREEN and (b) the named test to FAIL (or the test binary to abort).
# Usage: crates/heirloom-gl-abi/mutants.sh        (needs cargo; builds for the host)
set -u -o pipefail
PATH="$HOME/.cargo/bin:$PATH"
HERE="$(cd "$(dirname "$0")" && pwd)"
TARGET="${CARGO_BUILD_TARGET:-$(rustc -vV | sed -n 's/^host: //p')}"
WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT
fresh() { rm -rf "$WORK/c"; mkdir -p "$WORK/c"; cp -r "$HERE/src" "$HERE/tests" "$WORK/c/"
  printf '[package]\nname = "heirloom-gl-abi"\nversion = "0.1.0"\nedition = "2021"\n[workspace]\n' >"$WORK/c/Cargo.toml"; }
run() { (cd "$WORK/c" && cargo test -q --no-fail-fast --target "$TARGET" 2>&1); }
fresh; BASE="$(run)"; if echo "$BASE" | grep -q 'FAILED\|error\['; then echo "RED baseline is not green"; echo "$BASE" | tail -20; exit 1; fi
echo "baseline GREEN"
bad=0
mutant() { # name file sed-expr expected-test
  fresh; sed -i -E "$3" "$WORK/c/$2"
  if cmp -s "$WORK/c/$2" "$HERE/$2"; then echo "!! mutant '$1' did not change $2"; bad=1; return; fi
  out="$(run)"
  if echo "$out" | grep -q "$4"; then echo "RED as required: $1 -> $4"
  else echo "!! mutant '$1' survived (expected $4 to fail)"; echo "$out" | tail -8; bad=1; fi
}
mutant "wrong-endian: no byte swap"      src/snapshot.rs 's/if n > 1 \{/if false \&\& n > 1 {/'                         nc_indices_and_floats_are_converted_to_little_endian
mutant "offset arithmetic wraps"         src/snapshot.rs 's/o\.checked_add\((d\.offset|start)\)/Some(o.wrapping_add(\1))/g' nc_offset_and_stride_overflow_is_an_error_not_a_wrap
mutant "no range proof before alloc"     src/snapshot.rs '0,/if end > mem\.len\(\) \{/s//if false \&\& end > mem.len() {/' nc_a_huge_count_is_refused_before_anything_is_allocated
mutant "token ignores incarnation"       src/token.rs    's/s\.live && s\.incarnation == t\.incarnation/s.live/'          nc_a_token_from_an_earlier_incarnation_is_refused
mutant "unsupported call admitted"       src/refusal.rs  's/if e\.implemented \{/if true {/'                             nc_unsupported_call_is_refused_by_name
mutant "version string written by hand"  src/caps.rs     's/c\.version\.ok_or\(Refusal::NoClaimableVersion\)\?/c.version.unwrap_or(Version::V1_2)/' nc_with_nothing_implemented_no_version_and_no_extension_is_claimed
mutant "extension claimed on any entry"  src/caps.rs     's/it\.all\(\|e\| e\.complete\(\)\)/it.any(|e| e.complete())/'   nc_one_missing_entry_withdraws_its_extension_and_the_version
mutant "version ignores the attestation" src/caps.rs     's/attested_through\.map_or\(true, \|a\| v > a\)/false/'        the_unattested_table_claims_no_version_even_when_all_on
mutant "frame fields big-endian"         src/wire.rs     's/self\.buf\.extend_from_slice\(&v\.to_le_bytes\(\)\);/self.buf.extend_from_slice(\&v.to_be_bytes());/' nc_frames_are_version_first_and_little_endian
mutant "fence completes unretired"       src/fence.rs    's/Ok\(\*s <= self\.retired_through\)/Ok(true)/'                 a_fence_is_incomplete_until_the_worker_retires_its_batch
mutant "a gap does not withdraw a version" src/caps.rs   's/\|e\| !e\.complete\(\)/|e| !e.implemented/'                       a_gap_withdraws_the_claim_but_not_the_call
mutant "a gap does not withdraw an extension" src/caps.rs 's/it\.all\(\|e\| e\.complete\(\)\)/it.all(|e| e.implemented)/'     a_gap_withdraws_the_claim_but_not_the_call
mutant "pixel rows are not made tight"   src/pixel.rs    's/let stride = if l\.elem >= a \{ raw \}/let stride = if true { raw }/'                rgba_bytes_are_untouched_and_rows_are_tight
mutant "packed words are not converted"  src/pixel.rs    's/if elem > 1 \&\& guest_is_big_endian \{/if false \&\& guest_is_big_endian {/' multi_byte_elements_become_little_endian_and_swap_bytes_cancels_it
exit $bad
