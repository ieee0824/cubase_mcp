#!/usr/bin/env bash
set -euo pipefail

# Execute a named test, never accept a caller-authored "passed" artifact.
# The caller must trust this repository, Cargo home, and the local toolchain.
test "$#" -eq 0 || { echo 'usage: check-input-guard-sampling.sh' >&2; exit 1; }
ROOT=$(cd "$(dirname "$0")/.." && pwd -P)
OUTPUT=$(mktemp "${TMPDIR:-/tmp}/cmcp-sampling-test.XXXXXX")
trap 'rm -f "$OUTPUT"' EXIT HUP INT TERM
cd "$ROOT"
env -u RUSTC_WRAPPER -u RUSTC_WORKSPACE_WRAPPER -u RUSTFLAGS -u CARGO_ENCODED_RUSTFLAGS \
  cargo test --offline --locked --bin cubase_input_guard \
    tests::deterministic_sampling_contract -- --exact --nocapture --test-threads=1 > "$OUTPUT"
grep -Fq 'CMCP_SAMPLING_CONTRACT_V1_PASS' "$OUTPUT"
grep -Fq 'test result: ok. 1 passed; 0 failed;' "$OUTPUT"
jq -n \
  --arg source_sha "$(shasum -a 256 src/bin/cubase_input_guard.rs | awk '{print $1}')" \
  --arg lock_sha "$(shasum -a 256 Cargo.lock | awk '{print $1}')" \
  --arg checker_sha "$(shasum -a 256 "$ROOT/scripts/check-input-guard-sampling.sh" | awk '{print $1}')" \
  '{sampling_contract_report_version:1,status:"passed",mode:"deterministic_os_read_substitution",
    source_sha256:$source_sha,cargo_lock_sha256:$lock_sha,checker_sha256:$checker_sha,
    test:"tests::deterministic_sampling_contract",cases:12,
    runtime_physical_race_reproduced:false,
    limitations:["OS reads are substituted only in test builds; this does not reproduce an actual hardware event inside an OS sampling interval"]}'
