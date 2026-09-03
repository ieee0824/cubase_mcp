#!/usr/bin/env bash
set -euo pipefail

EXPECTED_ACTION_COUNT=63
EXPECTED_TRACE_RECORD_COUNT=$((EXPECTED_ACTION_COUNT + 2))
EXPECTED_CAPTURE_COUNT=$((EXPECTED_ACTION_COUNT * 2))
EXPECTED_CLICK_ACTION_COUNT=46
EXPECTED_ACTIVATION_ACTION_COUNT=7
# The detached index has 94 non-formal-capture entries. Each formal action
# contributes two screenshots and two state dumps.
EXPECTED_NON_CAPTURE_INDEX_ENTRY_COUNT=94
EXPECTED_DETACHED_INDEX_COUNT=$((EXPECTED_NON_CAPTURE_INDEX_ENTRY_COUNT + EXPECTED_ACTION_COUNT * 4))
CALIBRATION_PREFIX='cmcp-calibration'
EXPECTED_ACTIVATION_DIALOG_TEXT='プロジェクトをアクティブにしますか？'
EXPECTED_ACTIVATION_AFFIRMATIVE_IDENTITY='ボタン 有効化, ID: action-button-1'
EXPECTED_ACTIVATION_NEGATIVE_IDENTITY='ボタン いいえ, ID: action-button-2'
EXPECTED_ACTIVATION_OPEN_ACTION_IDS='["E0.open-project","E1.open-project","E8.open-project","C1.open-project","S9-empty.open-project","S9-mutation.open-project","S9-baseline.open-project"]'
EXPECTED_ACTIVATION_ACTION_IDS='["E0.activate-project","E1.activate-project","E8.activate-project","C1.activate-project","S9-empty.activate-project","S9-mutation.activate-project","S9-baseline.activate-project"]'
EXPECTED_ACTION_IDS='["INIT.launch-bootstrap","E0.open-project","E0.activate-project","E1.open-project","E1.activate-project","E8.open-project","E8.activate-project","C1.open-project","C1.activate-project","S0.save-as-shortcut","S0.save-as-name","S0.save-as-confirm","S1-select.track","S1-rename.activate","S1-rename.set-value","S1-rename.commit","S2-select-anchor.track-list-page-down","S2-select-anchor.track","S2-add.project-menu","S2-add.audio-item","S2-add.name","S2-add.confirm","S3-select-delete.track","S3-delete.project-menu","S3-delete.selected-item","S4-show.mixconsole-open","S4-show.visibility-toggle","S5-select-anchor.window-menu","S5-select-anchor.project-window","S5-select-anchor.track","S5-select-change.track","S6-mute.window-menu","S6-mute.mixconsole-window","S6-mute.control","S7-solo.control","S8-project-only-hide.sync-menu","S8-project-only-hide.sync-off","S8-project-only-hide.window-menu-project","S8-project-only-hide.project-window","S8-project-only-hide.left-zone-open","S8-project-only-hide.visibility-toggle","S8-project-only-hide.window-menu-mixconsole","S8-project-only-hide.mixconsole-window","S8-restore.window-menu-project","S8-restore.project-window","S8-restore.sync-menu","S8-restore.sync-on","S8-restore.save-shortcut","S8-restore.window-menu-mixconsole","S8-restore.mixconsole-window","S9-empty.open-project","S9-empty.activate-project","S9-mutation.open-project","S9-mutation.activate-project","S9-baseline.open-project","S9-baseline.activate-project","R1.studio-menu","R1.midi-remote-manager","R1.scripts-tab","R1.reload-scripts","R2.cubase-menu","R2.quit-item","R2.launch-baseline"]'
EXPECTED_ACTION_CHECKPOINTS='["INIT","E0","E0","E1","E1","E8","E8","C1","C1","S0","S0","S0","S1-select","S1-rename","S1-rename","S1-rename","S2-select-anchor","S2-select-anchor","S2-add","S2-add","S2-add","S2-add","S3-select-delete","S3-delete","S3-delete","S4-show","S4-show","S5-select-anchor","S5-select-anchor","S5-select-anchor","S5-select-change","S6-mute","S6-mute","S6-mute","S7-solo","S8-project-only-hide","S8-project-only-hide","S8-project-only-hide","S8-project-only-hide","S8-project-only-hide","S8-project-only-hide","S8-project-only-hide","S8-project-only-hide","S8-restore","S8-restore","S8-restore","S8-restore","S8-restore","S8-restore","S8-restore","S9-empty","S9-empty","S9-mutation","S9-mutation","S9-baseline","S9-baseline","R1","R1","R1","R1","R2","R2","R2"]'
EXPECTED_ACTION_APIS='["exec_command.open","exec_command.open","computer_use.click.element","exec_command.open","computer_use.click.element","exec_command.open","computer_use.click.element","exec_command.open","computer_use.click.element","computer_use.press_key","computer_use.set_value","computer_use.click.element","computer_use.click.coordinate","computer_use.click.coordinate","computer_use.set_value","computer_use.press_key","computer_use.click.coordinate","computer_use.click.coordinate","computer_use.click.element","computer_use.click.element","computer_use.set_value","computer_use.press_key","computer_use.click.coordinate","computer_use.click.element","computer_use.click.element","computer_use.press_key","computer_use.click.coordinate","computer_use.click.element","computer_use.click.element","computer_use.click.coordinate","computer_use.click.coordinate","computer_use.click.element","computer_use.click.element","computer_use.click.coordinate","computer_use.click.coordinate","computer_use.click.coordinate","computer_use.click.element","computer_use.click.element","computer_use.click.element","computer_use.click.coordinate","computer_use.click.coordinate","computer_use.click.element","computer_use.click.element","computer_use.click.element","computer_use.click.element","computer_use.click.coordinate","computer_use.click.element","computer_use.press_key","computer_use.click.element","computer_use.click.element","exec_command.open","computer_use.click.element","exec_command.open","computer_use.click.element","exec_command.open","computer_use.click.element","computer_use.click.element","computer_use.click.element","computer_use.click.coordinate","computer_use.click.coordinate","computer_use.click.element","computer_use.click.element","exec_command.open"]'
EXPECTED_ACTION_CLICK_COUNTS='[null,null,1,null,1,null,1,null,1,null,null,1,1,2,null,null,1,1,1,1,null,null,1,1,1,null,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,null,1,1,null,1,null,1,null,1,1,1,1,1,1,1,null]'

die() {
  echo "check-track-probe-evidence: $*" >&2
  exit 1
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "required command is unavailable: $1"
}

require_nonempty() {
  test -f "$1" || die "missing regular file: $1"
  test ! -L "$1" || die "symbolic links are not accepted: $1"
  test -s "$1" || die "empty file: $1"
}

require_path() {
  test -f "$1" || die "missing regular file: $1"
  test ! -L "$1" || die "symbolic links are not accepted: $1"
}

require_clean_stderr() {
  require_path "$1"
  if LC_ALL=C grep -q '[^[:space:]]' "$1"; then
    die "stderr artifact contains non-whitespace content: $1"
  fi
}

sha256_file() {
  shasum -a 256 "$1" | awk '{print $1}'
}

validate_screenshot_image() {
  local path=$1
  local magic
  local properties
  local format
  local width
  local height
  magic=$(od -An -tx1 -N 8 "$path" | tr -d ' \n')
  case "$path" in
    *.png) test "$magic" = '89504e470d0a1a0a' || die "PNG extension does not match file signature: $path" ;;
    *.jpg|*.jpeg) test "${magic#ffd8ff}" != "$magic" || die "JPEG extension does not match file signature: $path" ;;
    *) die "unsupported screenshot extension: $path" ;;
  esac

  properties=$(sips -1 -g format -g pixelWidth -g pixelHeight "$path" 2>/dev/null) ||
    die "screenshot cannot be decoded: $path"
  format=$(printf '%s\n' "$properties" | awk -F'|' '{for (i = 1; i <= NF; i++) if ($i ~ /^format: /) {sub(/^format: /, "", $i); print $i}}')
  width=$(printf '%s\n' "$properties" | awk -F'|' '{for (i = 1; i <= NF; i++) if ($i ~ /^pixelWidth: /) {sub(/^pixelWidth: /, "", $i); print $i}}')
  height=$(printf '%s\n' "$properties" | awk -F'|' '{for (i = 1; i <= NF; i++) if ($i ~ /^pixelHeight: /) {sub(/^pixelHeight: /, "", $i); print $i}}')
  case "$path" in
    *.png) test "$format" = 'png' || die "decoded screenshot format is not PNG: $path" ;;
    *.jpg|*.jpeg) test "$format" = 'jpeg' || die "decoded screenshot format is not JPEG: $path" ;;
  esac
  printf '%s\n' "$width" | LC_ALL=C grep -Eq '^([1-9][0-9]*(\.[0-9]+)?|0\.[0-9]*[1-9][0-9]*)$' || die "screenshot has no positive pixel width: $path"
  printf '%s\n' "$height" | LC_ALL=C grep -Eq '^([1-9][0-9]*(\.[0-9]+)?|0\.[0-9]*[1-9][0-9]*)$' || die "screenshot has no positive pixel height: $path"
}

require_hex64() {
  printf '%s\n' "$2" | LC_ALL=C grep -Eq '^[0-9a-f]{64}$' || die "$1 must be lowercase 64-hex"
}

canonical_directory() {
  (cd "$1" && pwd -P)
}

test "$#" -eq 2 || die "usage: $0 EVIDENCE_DIRECTORY REPOSITORY_ROOT"
EVIDENCE_DIRECTORY=$1
REPOSITORY_ROOT=$2

for command_name in awk cargo cmp find git grep jq mv od pwd shasum sips sort tr wc; do
  require_command "$command_name"
done

test -d "$EVIDENCE_DIRECTORY" || die "evidence directory does not exist"
test ! -L "$EVIDENCE_DIRECTORY" || die "evidence directory may not be a symbolic link"
test -d "$REPOSITORY_ROOT/.git" || die "repository root is not a Git worktree"
EVIDENCE_DIRECTORY=$(canonical_directory "$EVIDENCE_DIRECTORY")
REPOSITORY_ROOT=$(canonical_directory "$REPOSITORY_ROOT")
test "$(git -C "$REPOSITORY_ROOT" rev-parse --show-toplevel)" = "$REPOSITORY_ROOT" || die "REPOSITORY_ROOT is not the worktree root"
case "$EVIDENCE_DIRECTORY" in
  "$REPOSITORY_ROOT"|"$REPOSITORY_ROOT"/*) die "evidence directory must be outside the repository worktree" ;;
esac

TRUSTED_CALIBRATION_CHECKER="$REPOSITORY_ROOT/scripts/check-input-guard-calibration.sh"
TRUSTED_EVIDENCE_CHECKER="$REPOSITORY_ROOT/scripts/check-track-probe-evidence.sh"
require_nonempty "$TRUSTED_CALIBRATION_CHECKER"
require_nonempty "$TRUSTED_EVIDENCE_CHECKER"
checker_source=${BASH_SOURCE[0]}
case "$checker_source" in
  /*) invoked_checker=$checker_source ;;
  *) invoked_checker="$PWD/$checker_source" ;;
esac
INVOKED_CHECKER_DIRECTORY=$(cd "${invoked_checker%/*}" && pwd -P)
INVOKED_CHECKER="$INVOKED_CHECKER_DIRECTORY/${invoked_checker##*/}"
test "$INVOKED_CHECKER" = "$TRUSTED_EVIDENCE_CHECKER" ||
  die "invoke the checker from the repository trust root, not the evidence copy"

INVENTORY="$EVIDENCE_DIRECTORY/ui-action-inventory.json"
require_nonempty "$INVENTORY"
jq -s -e 'length == 1' "$INVENTORY" >/dev/null || die "inventory must contain exactly one JSON value"

RUN_ID=$(jq -er '.run_id | select(type == "string" and test("^[A-Za-z0-9][A-Za-z0-9._-]*$"))' "$INVENTORY") || die "inventory run_id is invalid"
PROFILE=$(jq -er '.profile | select(. == "c13_mixer_bank" or . == "c15_combined")' "$INVENTORY") || die "inventory profile is invalid"
EXPECTED_COMMIT=$(jq -er '.repository_commit | select(type == "string" and test("^[0-9a-f]{40}$"))' "$INVENTORY") || die "inventory repository_commit is invalid"
EXPECTED_GUARD_SHA=$(jq -er '.guard.binary_sha256' "$INVENTORY") || die "inventory guard digest is missing"
require_hex64 'inventory guard digest' "$EXPECTED_GUARD_SHA"

RAW_NAME="CMCP_TrackProbe_${RUN_ID}.jsonl"
RAW="$EVIDENCE_DIRECTORY/$RAW_NAME"
COLLECTOR_STDERR="$EVIDENCE_DIRECTORY/collector.stderr.log"
MANIFEST="$EVIDENCE_DIRECTORY/audit-manifest-v1.json"
AUDIT_REPORT="$EVIDENCE_DIRECTORY/audit-report-v2.json"
GUARD="$EVIDENCE_DIRECTORY/input-guard.jsonl"
GUARD_STDERR="$EVIDENCE_DIRECTORY/input-guard.stderr.log"
TRACE="$EVIDENCE_DIRECTORY/operator-tool-trace.jsonl"
RUN_RECORD="$EVIDENCE_DIRECTORY/operator-run-record.json"
CALIBRATION_SUMMARY="$EVIDENCE_DIRECTORY/guard-calibration-summary.json"
CALIBRATION_DIRECTORY="$EVIDENCE_DIRECTORY/calibration"
SCREENSHOT_DIRECTORY="$EVIDENCE_DIRECTORY/screenshots"
STATE_DIRECTORY="$EVIDENCE_DIRECTORY/states"
CALIBRATION_CHECKER="$EVIDENCE_DIRECTORY/check-calibration.sh"
EVIDENCE_CHECKER="$EVIDENCE_DIRECTORY/check-evidence.sh"
CALIBRATION_REPORT="$EVIDENCE_DIRECTORY/guard-calibration-report.json"
ACTION_REPORT="$EVIDENCE_DIRECTORY/action-evidence-report.json"
ARTIFACT_SHA256="$EVIDENCE_DIRECTORY/artifact-sha256.txt"
AUDITOR="$REPOSITORY_ROOT/target/release/cubase_track_probe_audit"
COLLECTOR="$REPOSITORY_ROOT/target/release/cubase_track_probe_collector"
GUARD_BINARY="$REPOSITORY_ROOT/target/release/cubase_input_guard"
PROBE_SOURCE="$REPOSITORY_ROOT/cubase/midi_remote/CubaseMCPTrackProbe/CubaseMCPTrackProbe/CubaseMCPTrackProbe_CubaseMCPTrackProbe.js"

# The top-level directory is a closed set. Generated reports and the detached
# checksum index are allowed on a repeat verification, but no unnamed scratch,
# abandoned, or alternate-run artifacts are accepted.
top_entry_count=0
while IFS= read -r -d '' entry; do
  top_entry_count=$((top_entry_count + 1))
  entry_name=${entry##*/}
  case "$entry_name" in
    "$RAW_NAME"|collector.stderr.log|audit-manifest-v1.json|audit-report-v2.json|input-guard.jsonl|input-guard.stderr.log|ui-action-inventory.json|operator-tool-trace.jsonl|operator-run-record.json|guard-calibration-summary.json|check-calibration.sh|check-evidence.sh|guard-calibration-report.json|action-evidence-report.json|artifact-sha256.txt)
      test -f "$entry" || die "canonical evidence artifact is not a regular file: $entry"
      test ! -L "$entry" || die "canonical evidence artifact may not be a symbolic link: $entry"
      ;;
    calibration|screenshots|states)
      test -d "$entry" || die "canonical evidence entry is not a directory: $entry"
      test ! -L "$entry" || die "canonical evidence directory may not be a symbolic link: $entry"
      ;;
    *) die "unexpected top-level evidence artifact: $entry_name" ;;
  esac
done < <(find "$EVIDENCE_DIRECTORY" -mindepth 1 -maxdepth 1 -print0)
test "$top_entry_count" -ge 14 || die "evidence staging is missing canonical input artifacts"
test "$top_entry_count" -le 18 || die "evidence staging contains too many top-level entries"

for file in "$RAW" "$GUARD" "$TRACE"; do
  require_nonempty "$file"
  jq -e . "$file" >/dev/null || die "invalid JSONL: $file"
done
for file in "$MANIFEST" "$RUN_RECORD" "$CALIBRATION_SUMMARY"; do
  require_nonempty "$file"
  jq -s -e 'length == 1' "$file" >/dev/null || die "file must contain exactly one JSON value: $file"
done
require_nonempty "$CALIBRATION_CHECKER"
require_nonempty "$EVIDENCE_CHECKER"
require_clean_stderr "$COLLECTOR_STDERR"
require_clean_stderr "$GUARD_STDERR"
test -d "$CALIBRATION_DIRECTORY" || die "missing calibration directory"
test -d "$SCREENSHOT_DIRECTORY" || die "missing screenshots directory"
test -d "$STATE_DIRECTORY" || die "missing states directory"
test -x "$AUDITOR" || die "release auditor missing or not executable"
test -x "$COLLECTOR" || die "release collector missing or not executable"
test -x "$GUARD_BINARY" || die "release guard missing or not executable"
require_nonempty "$PROBE_SOURCE"

# The checked-out repository is the trust root. Verify both evidence copies byte
# for byte, bind their hashes in the pre-existing calibration summary, and only
# then execute the calibration checker.
cmp -s "$TRUSTED_CALIBRATION_CHECKER" "$CALIBRATION_CHECKER" || die "evidence calibration checker differs from committed source"
cmp -s "$TRUSTED_EVIDENCE_CHECKER" "$EVIDENCE_CHECKER" || die "evidence final checker differs from committed source"
TRUSTED_CALIBRATION_CHECKER_SHA=$(sha256_file "$TRUSTED_CALIBRATION_CHECKER")
TRUSTED_EVIDENCE_CHECKER_SHA=$(sha256_file "$TRUSTED_EVIDENCE_CHECKER")
test "$(sha256_file "$CALIBRATION_CHECKER")" = "$TRUSTED_CALIBRATION_CHECKER_SHA" || die "calibration checker digest mismatch"
test "$(sha256_file "$EVIDENCE_CHECKER")" = "$TRUSTED_EVIDENCE_CHECKER_SHA" || die "final checker digest mismatch"
jq -e \
  --arg calibration_checker_sha "$TRUSTED_CALIBRATION_CHECKER_SHA" \
  --arg evidence_checker_sha "$TRUSTED_EVIDENCE_CHECKER_SHA" '
    .mechanical_validation.checker == "check-calibration.sh" and
    .mechanical_validation.checker_sha256 == $calibration_checker_sha and
    .mechanical_validation.final_checker == "check-evidence.sh" and
    .mechanical_validation.final_checker_sha256 == $evidence_checker_sha
  ' "$CALIBRATION_SUMMARY" >/dev/null || die "calibration summary does not bind both trusted checker digests"

test "$(git -C "$REPOSITORY_ROOT" rev-parse HEAD)" = "$EXPECTED_COMMIT" || die "repository commit drifted after inventory freeze"
test -z "$(git -C "$REPOSITORY_ROOT" status --short)" || die "repository worktree is not clean"

# Rebuild the auditor from the clean, inventory-bound commit in an isolated
# target directory. The run record cannot establish trust by merely asserting
# the digest of an untracked target/release executable.
REPRO_BUILD_DIRECTORY=$(mktemp -d "${TMPDIR:-/tmp}/cmcp-auditor-build.XXXXXX")
trap 'rm -rf "$REPRO_BUILD_DIRECTORY"' EXIT HUP INT TERM
(
  cd "$REPOSITORY_ROOT"
  unset RUSTC_WRAPPER RUSTC_WORKSPACE_WRAPPER RUSTFLAGS CARGO_ENCODED_RUSTFLAGS
  CARGO_TARGET_DIR="$REPRO_BUILD_DIRECTORY" cargo build \
    --manifest-path "$REPOSITORY_ROOT/Cargo.toml" \
    --release --locked --offline --bin cubase_track_probe_audit --quiet
) ||
  die "failed to rebuild the auditor from the inventory-bound commit"
REBUILT_AUDITOR="$REPRO_BUILD_DIRECTORY/release/cubase_track_probe_audit"
test -x "$REBUILT_AUDITOR" || die "isolated auditor rebuild did not produce an executable"

EXPECTED_INVENTORY_SHA=$(sha256_file "$INVENTORY")
EXPECTED_COLLECTOR_SHA=$(jq -er '.digests.collector_binary_sha256' "$RUN_RECORD") || die "collector digest is missing from run record"
EXPECTED_AUDITOR_SHA=$(jq -er '.digests.auditor_binary_sha256' "$RUN_RECORD") || die "auditor digest is missing from run record"
EXPECTED_PROBE_SHA=$(jq -er '.digests.probe_source_sha256' "$RUN_RECORD") || die "probe digest is missing from run record"
for pair in \
  "collector:$EXPECTED_COLLECTOR_SHA" \
  "auditor:$EXPECTED_AUDITOR_SHA" \
  "probe:$EXPECTED_PROBE_SHA"; do
  require_hex64 "${pair%%:*} digest" "${pair#*:}"
done
test "$(sha256_file "$GUARD_BINARY")" = "$EXPECTED_GUARD_SHA" || die "guard binary digest mismatch"
test "$(sha256_file "$COLLECTOR")" = "$EXPECTED_COLLECTOR_SHA" || die "collector binary digest mismatch"
test "$(sha256_file "$AUDITOR")" = "$EXPECTED_AUDITOR_SHA" || die "auditor binary digest mismatch"
test "$(sha256_file "$REBUILT_AUDITOR")" = "$EXPECTED_AUDITOR_SHA" || die "auditor digest does not match isolated clean-commit rebuild"
cmp -s "$AUDITOR" "$REBUILT_AUDITOR" || die "release auditor differs from isolated clean-commit rebuild"
test "$(sha256_file "$PROBE_SOURCE")" = "$EXPECTED_PROBE_SHA" || die "probe source digest mismatch"

if LC_ALL=C grep -En '__PENDING_|DRAFT_NOT_RUNTIME_EVIDENCE|"PENDING"|:[[:space:]]*null([,}])' "$MANIFEST" "$RUN_RECORD" "$CALIBRATION_SUMMARY" >/dev/null; then
  die "final evidence input contains a draft marker or null"
fi

jq -e \
  --arg run "$RUN_ID" --arg profile "$PROFILE" --arg commit "$EXPECTED_COMMIT" --arg guard_sha "$EXPECTED_GUARD_SHA" \
  --argjson action_count "$EXPECTED_ACTION_COUNT" --argjson action_ids "$EXPECTED_ACTION_IDS" \
  --argjson action_checkpoints "$EXPECTED_ACTION_CHECKPOINTS" --argjson action_apis "$EXPECTED_ACTION_APIS" \
  --argjson action_click_counts "$EXPECTED_ACTION_CLICK_COUNTS" \
  --argjson click_action_count "$EXPECTED_CLICK_ACTION_COUNT" '
    .ui_action_inventory_version == 1 and .inventory_state == "frozen" and
    .run_id == $run and .profile == $profile and .fixture_revision == 2 and .repository_commit == $commit and
    .guard.protocol_version == 4 and .guard.source == "hid_system_state" and
    .guard.coverage == "action_windows" and .guard.privacy == "counts_and_held_state_boolean" and
    .guard.policy == "consequential_input_only" and .guard.binary_sha256 == $guard_sha and
    .rules.one_injected_call_per_action == true and
    .rules.fresh_pre_state_required == true and .rules.fresh_post_state_required == true and
    .rules.no_retry_within_process == true and
    (.actions | length) == $action_count and ([.actions[].ordinal] == [range(1; $action_count + 1)]) and
    ([.actions[].action_id] == $action_ids) and
    ([.actions[].checkpoint_id] == $action_checkpoints) and
    ([.actions[].api] == $action_apis) and
    ([.actions[] | (.click_count // null)] == $action_click_counts) and
    all(.actions[];
      (.action_id | type == "string" and length > 0) and
      (.checkpoint_id | type == "string" and length > 0) and
      (.api | type == "string" and length > 0) and
      (.operation | type == "string" and length > 0) and
      (.precondition | type == "string" and length > 0) and
      (.postcondition | type == "string" and length > 0)
    ) and
    ([.actions[] | select(.api == "computer_use.click.element" or .api == "computer_use.click.coordinate")] | length) == $click_action_count and
    all(.actions[] | select(.api == "computer_use.click.element" or .api == "computer_use.click.coordinate");
      (.click_count | type == "number" and (. == 1 or . == 2))
    ) and
    all(.actions[] | select((.api == "computer_use.click.element" or .api == "computer_use.click.coordinate") | not);
      has("click_count") | not
    )
  ' "$INVENTORY" >/dev/null || die "inventory contract invalid"

jq -e \
  --arg run "$RUN_ID" --arg profile "$PROFILE" --arg commit "$EXPECTED_COMMIT" \
  --arg probe "$EXPECTED_PROBE_SHA" --arg collector "$EXPECTED_COLLECTOR_SHA" '
    .audit_manifest_version == 1 and .fixture_revision == 2 and .profile == $profile and .run_id == $run and
    (.run_started_at | type == "string" and test("^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}\\.[0-9]{3}[+-][0-9]{2}:[0-9]{2}$")) and
    .environment.repository_commit == $commit and
    .environment.probe_source_sha256 == $probe and
    .environment.installer_embedded_sha256 == $probe and .environment.deployed_probe_sha256 == $probe and
    .environment.collector_binary_sha256 == $collector and
    (if $profile == "c15_combined" then
      .environment.host == {product:"Cubase Pro",version:"15.0.30.287",api_version:"1.3"} and .filters.main_filter == "explicit"
     else
      .environment.host == {product:"Cubase Pro",version:"13.0.30.226",api_version:"1.1"} and .filters.main_filter == "implicit"
     end) and
    .callback_window_ms == 5000 and .reconnect_deadline_ms == 30000 and
    .mixconsole.surface == "separate" and
    (.mixconsole.visibility_sync_initial as $initial | (["on","off","not_open"] | index($initial)) != null) and
    .mixconsole.visibility_sync_during_baseline == "on" and .mixconsole.visibility_sync_restored == true and
    .optional_o1 == {status:"skipped",reason:"not_separately_authorized"} and
    (.annotations | length) == 44 and
    all(.annotations[]; .result == "observed" and .ui_ground_truth_confirmed == true and .action_confirmed == true)
  ' "$MANIFEST" >/dev/null || die "manifest structure or confirmations invalid"

# Validate the formal guard identity and exact sequence before binding it to the
# operator trace header.
jq -s -e --argjson action_count "$EXPECTED_ACTION_COUNT" '
  def hex64: type == "string" and test("^[0-9a-f]{64}$");
  def counter_keys: ["flags_changed","key_down","key_up","left_mouse_down","left_mouse_dragged","left_mouse_up","mouse_moved","other_mouse_down","other_mouse_dragged","other_mouse_up","right_mouse_down","right_mouse_dragged","right_mouse_up","scroll_wheel","tablet_pointer","tablet_proximity"] | sort;
  . as $records |
  .[0] as $ready |
  $ready.guard_session_id as $session_id |
  $ready.guard_process_id as $process_id |
  $ready.guard_started_at_unix_ms as $started_at |
  length == (2 * $action_count + 2) and
  $ready.type == "ready" and $ready.source == "hid_system_state" and $ready.privacy == "counts_and_held_state_boolean" and
  ($ready.guard_session_id | hex64) and
  ($ready.guard_process_id | type == "number" and . > 0 and floor == .) and
  ($ready.guard_started_at_unix_ms | type == "number" and . > 0 and floor == .) and
  all(.[];
    .version == 4 and .coverage == "action_windows" and .policy == "consequential_input_only" and
    .guard_session_id == $session_id and .guard_process_id == $process_id and .guard_started_at_unix_ms == $started_at
  ) and
  ([.[].record_sequence] == [range(1; length + 1)]) and
  .[-1].type == "finished" and .[-1].source == "hid_system_state" and .[-1].interference_detected == false and
  ([.[] | select(.type == "error" or .type == "cancelled" or .type == "rejected" or .type == "pong")] | length) == 0 and
  ([.[] | select(.type == "armed")] | length) == $action_count and
  ([.[] | select(.type == "result")] | length) == $action_count and
  all(.[] | select(.type == "result");
    .source == "hid_system_state" and .interference_detected == false and
    (.deltas | keys | sort) == counter_keys and
    all(.deltas[]; type == "number" and . >= 0 and floor == . and . <= 4294967295) and
    all(.deltas | to_entries[] | select(.key != "mouse_moved"); .value == 0)
  )
' "$GUARD" >/dev/null || die "formal guard v4 contract, identity, sequence, or deltas invalid"

jq -s -e --argjson action_count "$EXPECTED_ACTION_COUNT" --slurpfile inventory "$INVENTORY" '
  . as $records |
  ([.[] | select(.type == "armed") | .action_id] == ($inventory[0].actions | map(.action_id))) and
  ([.[] | select(.type == "result") | .action_id] == ($inventory[0].actions | map(.action_id))) and
  all(range(0; $action_count); . as $i |
    ($i * 2 + 1) as $armed | ($i * 2 + 2) as $result |
    $records[$armed].type == "armed" and $records[$result].type == "result" and
    $records[$armed].action_id == $inventory[0].actions[$i].action_id and
    $records[$result].action_id == $inventory[0].actions[$i].action_id
  )
' "$GUARD" >/dev/null || die "formal guard pairs are not adjacent and inventory ordered"

jq -s -e --slurpfile guard "$GUARD" --slurpfile inventory "$INVENTORY" --slurpfile run_record "$RUN_RECORD" \
  --arg run "$RUN_ID" --arg profile "$PROFILE" --argjson action_count "$EXPECTED_ACTION_COUNT" \
  --argjson trace_record_count "$EXPECTED_TRACE_RECORD_COUNT" \
  --argjson activation_action_ids "$EXPECTED_ACTIVATION_ACTION_IDS" \
  --arg activation_identity "$EXPECTED_ACTIVATION_AFFIRMATIVE_IDENTITY" '
  def hex64: type == "string" and test("^[0-9a-f]{64}$");
  def nonempty: type == "string" and length > 0;
  def leap($year): ($year % 4 == 0) and (($year % 100 != 0) or ($year % 400 == 0));
  def days_in_month($year; $month):
    if $month == 2 then (if leap($year) then 29 else 28 end)
    elif ($month == 4 or $month == 6 or $month == 9 or $month == 11) then 30
    else 31 end;
  def timestamp:
    type == "string" and
    test("^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}\\.[0-9]{3}(Z|[+-][0-9]{2}:[0-9]{2})$") and
    (capture("^(?<year>[0-9]{4})-(?<month>[0-9]{2})-(?<day>[0-9]{2})T(?<hour>[0-9]{2}):(?<minute>[0-9]{2}):(?<second>[0-9]{2})\\.(?<millisecond>[0-9]{3})(?<zone>Z|(?<sign>[+-])(?<zone_hour>[0-9]{2}):(?<zone_minute>[0-9]{2}))$") as $parts |
     ($parts.year | tonumber) as $year |
     ($parts.month | tonumber) as $month |
     ($parts.day | tonumber) as $day |
     ($parts.hour | tonumber) as $hour |
     ($parts.minute | tonumber) as $minute |
     ($parts.second | tonumber) as $second |
     $year >= 1970 and $month >= 1 and $month <= 12 and
     $day >= 1 and $day <= days_in_month($year; $month) and
     $hour <= 23 and $minute <= 59 and $second <= 59 and
     ($parts.zone == "Z" or (($parts.zone_hour | tonumber) <= 23 and ($parts.zone_minute | tonumber) <= 59)));
  def zone: capture("(?<zone>Z|[+-][0-9]{2}:[0-9]{2})$").zone;
  . as $records |
  [.[1:($action_count + 1)][]] as $actions |
  ([.[0].started_at] + [$actions[] | .pre_state.captured_at, .call_started_at, .call_ended_at, .post_state.captured_at] + [.[-1].ended_at]) as $times |
  ($times[0] | zone) as $zone |
  $run_record[0].fixture as $fixture |
  {
    "INIT.launch-bootstrap": $fixture.bootstrap.absolute_path,
    "E0.open-project": $fixture.empty.absolute_path,
    "E1.open-project": $fixture.one.absolute_path,
    "E8.open-project": $fixture.eight.absolute_path,
    "C1.open-project": $fixture.core_baseline.absolute_path,
    "S9-empty.open-project": $fixture.empty.absolute_path,
    "S9-mutation.open-project": $fixture.mutation_copy.absolute_path,
    "S9-baseline.open-project": $fixture.core_baseline.absolute_path,
    "R2.launch-baseline": $fixture.core_baseline.absolute_path
  } as $expected_open_paths |
  (if $profile == "c15_combined" then "Cubase 15" else "Cubase 13" end) as $expected_ui_app |
  length == $trace_record_count and
  .[0].operator_tool_trace_version == 1 and .[0].record_type == "session" and .[0].run_id == $run and
  (.[0].session_id | nonempty) and
  .[0].guard_session_id == $guard[0].guard_session_id and
  .[0].guard_process_id == $guard[0].guard_process_id and
  .[0].guard_started_at_unix_ms == $guard[0].guard_started_at_unix_ms and
  .[-1].operator_tool_trace_version == 1 and .[-1].record_type == "session_end" and
  .[-1].run_id == $run and .[-1].session_id == .[0].session_id and
  .[-1].finished_observed == true and .[-1].guard_process_exit_status == 0 and .[-1].no_retry_within_process == true and
  ([.[1:($action_count + 1)][].action_id] == ($inventory[0].actions | map(.action_id))) and
  ([.[1:($action_count + 1)][].ordinal] == [range(1; $action_count + 1)]) and
  ([.[1:($action_count + 1)][].checkpoint_id] == ($inventory[0].actions | map(.checkpoint_id))) and
  ([.[1:($action_count + 1)][].api] == ($inventory[0].actions | map(.api))) and
  ([.[1:($action_count + 1)][].operation] == ($inventory[0].actions | map(.operation))) and
  ([.[1:($action_count + 1)][].expected_precondition] == ($inventory[0].actions | map(.precondition))) and
  ([.[1:($action_count + 1)][].expected_postcondition] == ($inventory[0].actions | map(.postcondition))) and
  all(.[1:($action_count + 1)][];
    .operator_tool_trace_version == 1 and .record_type == "action" and .run_id == $run and
    .session_id == $records[0].session_id and
    .guard_armed_confirmed == true and .guard_result_confirmed == true and
    .pre_state.fresh == true and .pre_state.expected_condition_confirmed == true and
    (.pre_state.state_sha256 | hex64) and (.pre_state.screenshot_sha256 | hex64) and (.pre_state.app | nonempty) and
    .injected_call_count == 1 and .target_binding.resolved_fresh == true and .call_succeeded == true and
    .post_state.fresh == true and .post_state.expected_condition_confirmed == true and
    (.post_state.state_sha256 | hex64) and (.post_state.screenshot_sha256 | hex64) and (.post_state.app | nonempty) and
    .outcome == "confirmed" and .no_retry_within_process == true
  ) and
  all($times[]; timestamp) and all($times[]; zone == $zone) and
  all($actions[];
    .timestamp == .call_started_at and
    .pre_state.captured_at < .call_started_at and
    .call_started_at <= .call_ended_at and
    .call_ended_at < .post_state.captured_at
  ) and
  $records[0].started_at < $actions[0].pre_state.captured_at and
  all(range(1; $actions | length); . as $i |
    $actions[$i - 1].post_state.captured_at < $actions[$i].pre_state.captured_at
  ) and
  $actions[-1].post_state.captured_at < $records[-1].ended_at and
  all(range(0; $action_count); . as $i |
    $records[$i + 1].action_id == $inventory[0].actions[$i].action_id and
    (if ($records[$i + 1].api == "computer_use.click.element" or $records[$i + 1].api == "computer_use.click.coordinate") then
      $records[$i + 1].target_binding.click_count == $inventory[0].actions[$i].click_count
     else true end)
  ) and
  all(.[1:($action_count + 1)][] | select(.api == "computer_use.click.element");
    .target_binding.kind == "semantic_element" and
    (.target_binding.application | nonempty) and (.target_binding.window | nonempty) and (.target_binding.element_identity | nonempty)
  ) and
  all(.[1:($action_count + 1)][] |
      select(.action_id as $action_id | ($activation_action_ids | index($action_id)) != null);
    .target_binding.element_identity == $activation_identity
  ) and
  all(.[1:($action_count + 1)][] | select(.api == "computer_use.click.coordinate");
    .target_binding.kind == "window_coordinate" and
    (.target_binding.application | nonempty) and (.target_binding.window | nonempty) and
    (.target_binding.x | type == "number") and (.target_binding.y | type == "number") and
    (.target_binding.fresh_screenshot_sha256 | hex64) and
    .target_binding.fresh_screenshot_sha256 == .pre_state.screenshot_sha256
  ) and
  all(.[1:($action_count + 1)][] | select(.api == "computer_use.set_value");
    .target_binding.kind == "semantic_element" and
    (.target_binding.application | nonempty) and (.target_binding.window | nonempty) and (.target_binding.element_identity | nonempty)
  ) and
  all(.[1:($action_count + 1)][] | select(.api == "computer_use.press_key");
    .target_binding.kind == "key_chord" and
    (.target_binding.application | nonempty) and (.target_binding.window | nonempty) and
    (.target_binding.keys | type == "array" and length > 0 and all(.[]; nonempty))
  ) and
  all(.[1:($action_count + 1)][] | select(.api == "exec_command.open");
    .target_binding.kind == "os_open" and
    (.target_binding.exact_application_or_bundle_path | startswith("/")) and
    (.target_binding.absolute_document_path | startswith("/")) and
    .target_binding.absolute_document_path == $expected_open_paths[.action_id] and
    .target_binding.exact_application_or_bundle_path == $run_record[0].environment.application.exact_application_or_bundle_path and
    (if (.action_id == "INIT.launch-bootstrap" or .action_id == "R2.launch-baseline") then
       .pre_state.app == "Finder"
     else .pre_state.app == $expected_ui_app end) and
    .post_state.app == $expected_ui_app
  ) and
  all(.[1:($action_count + 1)][] | select(.api != "exec_command.open" and .action_id != "R2.quit-item");
    .target_binding.application == $expected_ui_app and
    .pre_state.app == $expected_ui_app and .post_state.app == $expected_ui_app
  ) and
  (.[1:($action_count + 1)][] | select(.action_id == "R2.quit-item") |
    .target_binding.application == $expected_ui_app and
    .pre_state.app == $expected_ui_app and .post_state.app == "Finder") and
  all(.[]; (has("retry_attempted") | not) and (has("completed_without_retry") | not))
' "$TRACE" >/dev/null || die "operator tool trace or guard identity binding invalid"

SCREENSHOT_REFS=$(jq -sc '
  [.[] | select(.record_type == "action") |
    {path: .pre_state.screenshot_path, sha256: .pre_state.screenshot_sha256},
    {path: .post_state.screenshot_path, sha256: .post_state.screenshot_sha256}]
' "$TRACE")
jq -e --argjson capture_count "$EXPECTED_CAPTURE_COUNT" '
  length == $capture_count and
  all(.[];
    (.path | type == "string" and test("^screenshots/[A-Za-z0-9][A-Za-z0-9._-]*\\.(png|jpg|jpeg)$")) and
    (.sha256 | type == "string" and test("^[0-9a-f]{64}$"))
  ) and ((map(.path) | unique | length) == $capture_count)
' <<<"$SCREENSHOT_REFS" >/dev/null || die "each formal action must bind two unique canonical screenshot files"

screenshot_count=0
while IFS= read -r -d '' screenshot; do
  screenshot_count=$((screenshot_count + 1))
  test -f "$screenshot" || die "unexpected non-file in screenshots directory: $screenshot"
  test ! -L "$screenshot" || die "screenshot may not be a symbolic link: $screenshot"
  validate_screenshot_image "$screenshot"
  relative_path="screenshots/${screenshot##*/}"
  expected_sha=$(jq -er --arg path "$relative_path" '.[] | select(.path == $path) | .sha256' <<<"$SCREENSHOT_REFS") || die "extra screenshot not referenced by trace: $relative_path"
  test "$(sha256_file "$screenshot")" = "$expected_sha" || die "screenshot digest mismatch: $relative_path"
done < <(find "$SCREENSHOT_DIRECTORY" -mindepth 1 -maxdepth 1 -print0)
test "$screenshot_count" -eq "$EXPECTED_CAPTURE_COUNT" ||
  die "formal screenshots directory must contain exactly $EXPECTED_CAPTURE_COUNT trace-bound files (found $screenshot_count)"

STATE_REFS=$(jq -sc '
  [.[] | select(.record_type == "action") |
    {path: .pre_state.state_path, sha256: .pre_state.state_sha256, captured_at: .pre_state.captured_at, app: .pre_state.app},
    {path: .post_state.state_path, sha256: .post_state.state_sha256, captured_at: .post_state.captured_at, app: .post_state.app}]
' "$TRACE")
jq -e --argjson capture_count "$EXPECTED_CAPTURE_COUNT" '
  length == $capture_count and
  all(.[];
    (.path | type == "string" and test("^states/[A-Za-z0-9][A-Za-z0-9._-]*\\.json$")) and
    (.sha256 | type == "string" and test("^[0-9a-f]{64}$")) and
    (.captured_at | type == "string" and length > 0) and
    (.app | type == "string" and length > 0)
  ) and ((map(.path) | unique | length) == $capture_count)
' <<<"$STATE_REFS" >/dev/null || die "each formal action must bind two unique canonical JSON state dumps"

state_count=0
while IFS= read -r -d '' state_dump; do
  state_count=$((state_count + 1))
  test -f "$state_dump" || die "unexpected non-file in states directory: $state_dump"
  test ! -L "$state_dump" || die "state dump may not be a symbolic link: $state_dump"
  relative_path="states/${state_dump##*/}"
  expected_sha=$(jq -er --arg path "$relative_path" '.[] | select(.path == $path) | .sha256' <<<"$STATE_REFS") || die "extra state dump not referenced by trace: $relative_path"
  expected_captured_at=$(jq -er --arg path "$relative_path" '.[] | select(.path == $path) | .captured_at' <<<"$STATE_REFS") || die "state capture time missing from trace: $relative_path"
  expected_app=$(jq -er --arg path "$relative_path" '.[] | select(.path == $path) | .app' <<<"$STATE_REFS") || die "state application missing from trace: $relative_path"
  jq -s -e '
    length == 1 and (.[0] |
      (keys | sort) == (["app","capture_format_version","captured_at","text"] | sort) and
      .capture_format_version == 1 and
      (.captured_at | type == "string" and length > 0) and
      (.app | type == "string" and length > 0) and
      (.text | type == "string" and length > 0)
    )
  ' "$state_dump" >/dev/null || die "state dump does not match the canonical capture schema: $relative_path"
  test "$(sha256_file "$state_dump")" = "$expected_sha" || die "state dump digest mismatch: $relative_path"
  test "$(jq -sr '.[0].captured_at | select(type == "string")' "$state_dump")" = "$expected_captured_at" || die "state dump captured_at does not match trace: $relative_path"
  test "$(jq -sr '.[0].app | select(type == "string")' "$state_dump")" = "$expected_app" || die "state dump application does not match trace: $relative_path"
done < <(find "$STATE_DIRECTORY" -mindepth 1 -maxdepth 1 -print0)
test "$state_count" -eq "$EXPECTED_CAPTURE_COUNT" ||
  die "formal states directory must contain exactly $EXPECTED_CAPTURE_COUNT trace-bound files (found $state_count)"

validate_activation_dialog_state() {
  local path=$1
  jq -s -e \
    --arg dialog "$EXPECTED_ACTIVATION_DIALOG_TEXT" \
    --arg affirmative "$EXPECTED_ACTIVATION_AFFIRMATIVE_IDENTITY" \
    --arg negative "$EXPECTED_ACTIVATION_NEGATIVE_IDENTITY" '
      length == 1 and (.[0].text as $text |
        ($text | split($dialog) | length) == 2 and
        ([$text | split("\n")[] | select(test("^[[:space:]]*[0-9]+ ダイアログ( |$)"))] | length) == 1 and
        ([$text | split("\n")[] | select(test("^[[:space:]]*[0-9]+ ボタン ")) |
          sub("^[[:space:]]*[0-9]+ "; "")] | sort) == ([$negative, $affirmative] | sort)
      )
    ' "$path" >/dev/null || die "activation dialog state is not the exact two-button Japanese contract: ${path#"$EVIDENCE_DIRECTORY"/}"
}

validate_no_activation_dialog_state() {
  local path=$1
  jq -s -e \
    --arg dialog "$EXPECTED_ACTIVATION_DIALOG_TEXT" \
    --arg affirmative "$EXPECTED_ACTIVATION_AFFIRMATIVE_IDENTITY" \
    --arg negative "$EXPECTED_ACTIVATION_NEGATIVE_IDENTITY" '
      length == 1 and (.[0].text |
        ([split("\n")[] | select(test("^[[:space:]]*[0-9]+ ダイアログ( |$)"))] | length) == 0 and
        (contains($dialog) | not) and
        (contains($affirmative) | not) and
        (contains($negative) | not)
      )
    ' "$path" >/dev/null || die "activation dialog persisted outside its exact open-to-activate transition: ${path#"$EVIDENCE_DIRECTORY"/}"
}

activation_transition_count=0
while IFS=$'\t' read -r open_action open_pre open_post activation_action activation_pre activation_post activation_identity; do
  activation_transition_count=$((activation_transition_count + 1))
  test -n "$open_action" && test -n "$activation_action" || die "activation transition is incomplete in operator trace"
  test "$activation_identity" = "$EXPECTED_ACTIVATION_AFFIRMATIVE_IDENTITY" ||
    die "activation action did not target the exact affirmative semantic button: $activation_action"
  for relative_path in "$open_pre" "$open_post" "$activation_pre" "$activation_post"; do
    case "$relative_path" in
      states/[A-Za-z0-9]*.json) ;;
      *) die "activation transition references an invalid state path: $relative_path" ;;
    esac
    require_nonempty "$EVIDENCE_DIRECTORY/$relative_path"
  done
  validate_no_activation_dialog_state "$EVIDENCE_DIRECTORY/$open_pre"
  validate_activation_dialog_state "$EVIDENCE_DIRECTORY/$open_post"
  validate_activation_dialog_state "$EVIDENCE_DIRECTORY/$activation_pre"
  validate_no_activation_dialog_state "$EVIDENCE_DIRECTORY/$activation_post"
done < <(
  jq -sr \
    --argjson open_action_ids "$EXPECTED_ACTIVATION_OPEN_ACTION_IDS" \
    --argjson activation_action_ids "$EXPECTED_ACTIVATION_ACTION_IDS" '
      [.[] | select(.record_type == "action")] as $actions |
      range(0; $activation_action_ids | length) as $index |
      ($actions | map(.action_id) | index($open_action_ids[$index])) as $open_index |
      ($actions | map(.action_id) | index($activation_action_ids[$index])) as $activation_index |
      select($open_index != null and $activation_index == ($open_index + 1)) |
      [
        $actions[$open_index].action_id,
        $actions[$open_index].pre_state.state_path,
        $actions[$open_index].post_state.state_path,
        $actions[$activation_index].action_id,
        $actions[$activation_index].pre_state.state_path,
        $actions[$activation_index].post_state.state_path,
        $actions[$activation_index].target_binding.element_identity
      ] | @tsv
    ' "$TRACE"
)
test "$activation_transition_count" -eq "$EXPECTED_ACTIVATION_ACTION_COUNT" ||
  die "operator trace must contain exactly $EXPECTED_ACTIVATION_ACTION_COUNT adjacent open-to-activation transitions"

noninteractive_launch_count=0
while IFS=$'\t' read -r launch_action launch_post; do
  noninteractive_launch_count=$((noninteractive_launch_count + 1))
  case "$launch_post" in
    states/[A-Za-z0-9]*.json) ;;
    *) die "noninteractive launch references an invalid state path: $launch_post" ;;
  esac
  require_nonempty "$EVIDENCE_DIRECTORY/$launch_post"
  validate_no_activation_dialog_state "$EVIDENCE_DIRECTORY/$launch_post"
done < <(
  jq -sr '
    [.[] | select(.record_type == "action")][] |
    select(.action_id == "INIT.launch-bootstrap" or .action_id == "R2.launch-baseline") |
    [.action_id, .post_state.state_path] | @tsv
  ' "$TRACE"
)
test "$noninteractive_launch_count" -eq 2 ||
  die "operator trace must contain INIT and R2 noninteractive launch post-states"

calibration_artifact_paths=()
while IFS= read -r -d '' calibration_artifact; do
  calibration_artifact_paths+=("${calibration_artifact#"$EVIDENCE_DIRECTORY"/}")
done < <(find "$CALIBRATION_DIRECTORY" -type f -print0)
EXPECTED_CALIBRATION_ARTIFACT_PATHS=$(jq -cn --args '$ARGS.positional | sort' "${calibration_artifact_paths[@]}")

jq -e \
  --arg run "$RUN_ID" --arg profile "$PROFILE" --arg commit "$EXPECTED_COMMIT" \
  --arg inventory_sha "$EXPECTED_INVENTORY_SHA" --arg guard_sha "$EXPECTED_GUARD_SHA" \
  --arg probe_sha "$EXPECTED_PROBE_SHA" --arg collector_sha "$EXPECTED_COLLECTOR_SHA" \
  --arg auditor_sha "$EXPECTED_AUDITOR_SHA" --arg calibration_checker_sha "$TRUSTED_CALIBRATION_CHECKER_SHA" \
  --arg evidence_checker_sha "$TRUSTED_EVIDENCE_CHECKER_SHA" \
  --argjson calibration_artifact_paths "$EXPECTED_CALIBRATION_ARTIFACT_PATHS" \
  --argjson action_count "$EXPECTED_ACTION_COUNT" \
  --slurpfile guard "$GUARD" --slurpfile manifest "$MANIFEST" '
    def hex64: type == "string" and test("^[0-9a-f]{64}$");
    def absolute: type == "string" and startswith("/");
    .fixture.directory as $fixture_directory |
    .operator_run_record_version == 1 and .record_state == "final" and
    .run_id == $run and .profile == $profile and .fixture_revision == 2 and
    .preflight.repository_commit == $commit and .preflight.working_tree_clean == true and
    (.preflight.offline_validation | keys | sort) ==
      (["cargo_clippy","cargo_fmt","cargo_release_build","cargo_test","midi_remote_script_test","track_probe_script_test"] | sort) and
    all(.preflight.offline_validation[]; . == "passed") and
    .preflight.cubase_absent_before_collector_start == true and .preflight.os_permissions_resolved_before_guard_start == true and
    .environment.host.runtime_reconfirmed == true and .environment.os.runtime_reconfirmed == true and
    .environment.application.identity_reconfirmed == true and .environment.midi.bounded_discovery_exactly_one_ready_instance == true and
    .environment.application.exact_application_or_bundle_path ==
      (if $profile == "c15_combined" then "/Applications/Cubase 15.app" else "/Applications/Cubase 13.app" end) and
    .digests.ui_action_inventory_sha256 == $inventory_sha and
    .digests.input_guard_binary_sha256 == $guard_sha and
    .digests.probe_source_sha256 == $probe_sha and .digests.installer_embedded_sha256 == $probe_sha and
    .digests.deployed_probe_sha256 == $probe_sha and .digests.collector_binary_sha256 == $collector_sha and
    .digests.auditor_binary_sha256 == $auditor_sha and
    .digests.calibration_checker_sha256 == $calibration_checker_sha and .digests.final_checker_sha256 == $evidence_checker_sha and
    ($fixture_directory | absolute) and
    (.fixture.bootstrap | keys | sort) ==
      (["absolute_path","basename","no_media_events_parts_automation_plugins_presets_or_new_routing_confirmed","sha256_after","sha256_before","unchanged_after_run","zero_project_tracks_confirmed"] | sort) and
    .fixture.bootstrap.basename == "CMCP_TrackFixture_Bootstrap_Empty.cpr" and
    .fixture.bootstrap.absolute_path == ($fixture_directory + "/CMCP_TrackFixture_Bootstrap_Empty.cpr") and
    (.fixture.bootstrap.sha256_before | hex64) and .fixture.bootstrap.sha256_after == .fixture.bootstrap.sha256_before and
    .fixture.bootstrap.zero_project_tracks_confirmed == true and
    .fixture.bootstrap.no_media_events_parts_automation_plugins_presets_or_new_routing_confirmed == true and
    .fixture.bootstrap.unchanged_after_run == true and
    (.fixture.empty | keys | sort) == (["absolute_path","sha256_before"] | sort) and
    .fixture.empty.absolute_path == ($fixture_directory + "/CMCP_TrackFixture_Empty.cpr") and
    (.fixture.empty.sha256_before | hex64) and
    (.fixture.one | keys | sort) == (["absolute_path","sha256_before"] | sort) and
    .fixture.one.absolute_path == ($fixture_directory + "/CMCP_TrackFixture_One.cpr") and
    (.fixture.one.sha256_before | hex64) and
    (.fixture.eight | keys | sort) == (["absolute_path","sha256_before"] | sort) and
    .fixture.eight.absolute_path == ($fixture_directory + "/CMCP_TrackFixture_Eight.cpr") and
    (.fixture.eight.sha256_before | hex64) and
    (.fixture.core_baseline | keys | sort) == (["absolute_path","sha256_after","sha256_before","unchanged_after_run"] | sort) and
    .fixture.core_baseline.absolute_path == ($fixture_directory + "/CMCP_TrackFixture_Core_Baseline.cpr") and
    (.fixture.core_baseline.sha256_before | hex64) and
    .fixture.core_baseline.sha256_after == .fixture.core_baseline.sha256_before and
    .fixture.core_baseline.unchanged_after_run == true and
    (.fixture.mutation_copy | keys | sort) == (["absolute_path","created_only_after_s0","sha256_after"] | sort) and
    .fixture.mutation_copy.absolute_path == ($fixture_directory + "/CMCP_TrackFixture_Mutation.cpr") and
    (.fixture.mutation_copy.sha256_after | hex64) and
    .fixture.mutation_copy.absolute_path != .fixture.bootstrap.absolute_path and
    .fixture.mutation_copy.absolute_path != .fixture.empty.absolute_path and
    .fixture.mutation_copy.absolute_path != .fixture.one.absolute_path and
    .fixture.mutation_copy.absolute_path != .fixture.eight.absolute_path and
    .fixture.mutation_copy.absolute_path != .fixture.core_baseline.absolute_path and
    .fixture.mutation_copy.created_only_after_s0 == true and
    .collector.started_before_cubase == true and .collector.first_record_confirmed == true and
    (.collector.first_timestamp_unix_ms | type == "number" and floor == .) and
    .collector.same_process_for_entire_run == true and .collector.summary_integrity_ok == true and
    .collector.summary_exit_ok == true and .collector.summary_exit_reason == "stdin_eof" and
    .input_guard.protocol_version == 4 and .input_guard.same_process_for_entire_run == true and
    .input_guard.guard_session_id == $guard[0].guard_session_id and
    .input_guard.guard_process_id == $guard[0].guard_process_id and
    .input_guard.guard_started_at_unix_ms == $guard[0].guard_started_at_unix_ms and
    .input_guard.started_after_collector_and_before_first_checkpoint == true and .input_guard.ready_confirmed == true and
    .input_guard.inventory_action_count == $action_count and .input_guard.armed_result_pairs_exactly_match_inventory == true and
    .input_guard.all_consequential_deltas_zero == true and .input_guard.no_error_cancel_reject_or_latch == true and
    .input_guard.successful_finish_after_collector_summary == true and
    (.input_guard.calibration | keys | sort) ==
      (["artifact_paths","exact_automation_context_negative_matrix","physical_click_rejection","physical_drag_rejection","physical_key_rejection","physical_move_only_acceptance","physical_scroll_rejection","wrong_coordinate_clean_guard_but_failed_postcondition_rejection"] | sort) and
    all(.input_guard.calibration | to_entries[] | select(.key != "artifact_paths"); .value == "passed") and
    (.input_guard.calibration.artifact_paths | type == "array" and length == 80 and
      all(.[]; type == "string" and test("^calibration/[A-Za-z0-9][A-Za-z0-9._/-]*$") and (contains("..") | not)) and
      (unique | length) == length and (sort == $calibration_artifact_paths)) and
    .checkpoint_execution.expected_order == ($manifest[0].annotations | map(.checkpoint_id)) and
    .checkpoint_execution.completed_order == .checkpoint_execution.expected_order and
    (.checkpoint_execution.expected_order | length) == 44 and .checkpoint_execution.invalidating_events == [] and
    .checkpoint_execution.all_action_markers_exactly_once == true and .checkpoint_execution.all_callback_windows_at_least_5000_ms == true and
    .checkpoint_execution.all_final_snapshot_quiet_periods_at_least_1000_ms == true and
    .checkpoint_execution.all_required_direct_access_then_bank_snapshots_complete == true and
    .ui_actions.inventory_frozen_before_run == true and .ui_actions.expected_count == $action_count and .ui_actions.tool_trace_count == $action_count and
    .ui_actions.all_fresh_pre_states == true and .ui_actions.all_target_bound_exactly_one_call == true and
    .ui_actions.all_fresh_action_specific_postconditions == true and .ui_actions.all_screenshot_hashes_recomputed == true and
    .ui_actions.all_state_hashes_recomputed == true and
    .ui_actions.no_retry_within_process == true and
    .cleanup.visibility_sync_restored_to_initial == true and .cleanup.bootstrap_closed == true and
    .cleanup.bootstrap_hash_unchanged == true and .cleanup.core_baseline_hash_unchanged == true and
    .cleanup.fixture_only_changes == true and .cleanup.repository_worktree_clean_of_runtime_artifacts == true and
    .audit.auditor_exit_success == true and .audit.audit_report_version == 2 and .audit.status == "evidence_valid" and
    .audit.semantic_assessment == "observed_not_evaluated" and .audit.checkpoint_count == 44 and
    .audit.artifact_digests_match == true and .audit.secrets_paths_ports_and_raw_ids_absent_from_shareable_report == true
  ' "$RUN_RECORD" >/dev/null || die "operator run record is incomplete or inconsistent"

# The run record is local-sensitive evidence and names the actual project
# fixtures. Recompute their hashes here so its unchanged claims are not merely
# self-asserted booleans.
FIXTURE_DIRECTORY=$(jq -er '.fixture.directory | select(type == "string" and startswith("/"))' "$RUN_RECORD") ||
  die "fixture directory is not an absolute path"
test -d "$FIXTURE_DIRECTORY" || die "fixture directory does not exist: $FIXTURE_DIRECTORY"
test ! -L "$FIXTURE_DIRECTORY" || die "fixture directory may not be a symbolic link"
test "$(canonical_directory "$FIXTURE_DIRECTORY")" = "$FIXTURE_DIRECTORY" || die "fixture directory must be canonical"
case "$FIXTURE_DIRECTORY" in
  "$REPOSITORY_ROOT"|"$REPOSITORY_ROOT"/*) die "fixture directory must be outside the repository worktree" ;;
esac
for fixture_key in bootstrap empty one eight core_baseline; do
  fixture_path=$(jq -er --arg key "$fixture_key" '.fixture[$key].absolute_path' "$RUN_RECORD") ||
    die "fixture path missing for $fixture_key"
  fixture_sha=$(jq -er --arg key "$fixture_key" '.fixture[$key].sha256_before' "$RUN_RECORD") ||
    die "fixture digest missing for $fixture_key"
  require_nonempty "$fixture_path"
  test "$(sha256_file "$fixture_path")" = "$fixture_sha" || die "fixture digest drifted: $fixture_key"
done
MUTATION_COPY=$(jq -er '.fixture.mutation_copy.absolute_path' "$RUN_RECORD") || die "mutation copy path missing"
require_nonempty "$MUTATION_COPY"
MUTATION_COPY_SHA=$(jq -er '.fixture.mutation_copy.sha256_after' "$RUN_RECORD") || die "mutation copy digest missing"
test "$(sha256_file "$MUTATION_COPY")" = "$MUTATION_COPY_SHA" || die "fixture digest drifted: mutation_copy"

CALIBRATION_TMP=$(mktemp "${TMPDIR:-/tmp}/cmcp-calibration-report.XXXXXX")
AUDIT_TMP=$(mktemp "${TMPDIR:-/tmp}/cmcp-audit-report.XXXXXX")
AUDIT_STDERR_TMP=$(mktemp "${TMPDIR:-/tmp}/cmcp-audit-stderr.XXXXXX")
ACTION_TMP=$(mktemp "${TMPDIR:-/tmp}/cmcp-action-report.XXXXXX")
SHA_TMP=$(mktemp "${TMPDIR:-/tmp}/cmcp-artifact-sha.XXXXXX")
cleanup_temporary_files() {
  rm -f "$CALIBRATION_TMP" "$AUDIT_TMP" "$AUDIT_STDERR_TMP" "$ACTION_TMP" "$SHA_TMP"
  rm -rf "$REPRO_BUILD_DIRECTORY"
}
trap cleanup_temporary_files EXIT HUP INT TERM

# Trust checks above intentionally precede this execution. Execute the
# repository trust-root path itself so a later change to the writable evidence
# copy cannot create a check/use race.
bash "$TRUSTED_CALIBRATION_CHECKER" "$CALIBRATION_DIRECTORY" "$CALIBRATION_PREFIX" "$GUARD_BINARY" "$EXPECTED_GUARD_SHA" > "$CALIBRATION_TMP"
jq -s -e \
  --arg guard_sha "$EXPECTED_GUARD_SHA" --arg checker_sha "$TRUSTED_CALIBRATION_CHECKER_SHA" '
    length == 1 and (.[0] |
    .calibration_report_version == 2 and .status == "valid" and .file_prefix == "cmcp-calibration" and
    .guard_contract == {
      version:4, source:"hid_system_state", coverage:"action_windows",
      privacy:"counts_and_held_state_boolean", policy:"consequential_input_only", binary_sha256:$guard_sha
    } and
    .checker_sha256 == $checker_sha and
    .all_pre_post_artifact_digests_recomputed == true and
    .controls.automation_negative == ["exec_command.open","computer_use.press_key","computer_use.set_value","computer_use.click.element","computer_use.click.coordinate.single","computer_use.click.coordinate.double"] and
    .controls.move_only_acceptance == ["semantic_target_binding","coordinate_target_binding"] and
    .controls.consequential_positive == ["physical_click","physical_key","physical_scroll","physical_drag"] and
    .controls.target_binding_rejection == "wrong_valid_coordinate_rejected_after_clean_result" and
    (.controls.sample_guard_rejection == "held_state" or .controls.sample_guard_rejection == "sample_race") and
    .fresh_guard_identity_count == 8 and (.guard_sessions | length) == 8 and
    (.evidence_sha256 | keys | sort) == (["automation","move","positive-click","positive-drag","positive-key","positive-scroll","sample-rejection","wrong-target"] | sort) and
    all(.evidence_sha256[];
      (.guard_jsonl | test("^[0-9a-f]{64}$")) and (.guard_stderr | test("^[0-9a-f]{64}$")) and
      (.operator_trace_jsonl | test("^[0-9a-f]{64}$"))
    ))
  ' "$CALIBRATION_TMP" >/dev/null || die "mechanical calibration report invalid"
mv "$CALIBRATION_TMP" "$CALIBRATION_REPORT"
CALIBRATION_REPORT_SHA=$(sha256_file "$CALIBRATION_REPORT")

jq -e \
  --arg run "$RUN_ID" --arg calibration_report_sha "$CALIBRATION_REPORT_SHA" \
  --arg calibration_checker_sha "$TRUSTED_CALIBRATION_CHECKER_SHA" \
  --arg evidence_checker_sha "$TRUSTED_EVIDENCE_CHECKER_SHA" \
  --slurpfile calibration_report "$CALIBRATION_REPORT" '
    .calibration_summary_version == 1 and .summary_state == "final" and .run_id == $run and
    .guard_contract == {version:4,source:"hid_system_state",coverage:"action_windows",privacy:"counts_and_held_state_boolean",policy:"consequential_input_only"} and
    .mechanical_validation.status == "valid" and .mechanical_validation.calibration_directory == "calibration" and
    .mechanical_validation.report == "guard-calibration-report.json" and
    .mechanical_validation.checker == "check-calibration.sh" and
    .mechanical_validation.final_checker == "check-evidence.sh" and
    .mechanical_validation.report_sha256 == $calibration_report_sha and
    .mechanical_validation.checker_sha256 == $calibration_checker_sha and
    .mechanical_validation.final_checker_sha256 == $evidence_checker_sha and
    (.automation_negative | keys | sort) ==
      (["computer_use_press_key","computer_use_set_value","coordinate_double_click","coordinate_single_click","exec_command_open","semantic_element_click"] | sort) and
    all(.automation_negative[]; . == "passed") and
    (.physical_controls | keys | sort) ==
      (["click_rejection","drag_rejection","key_rejection","move_only_acceptance","scroll_rejection"] | sort) and
    all(.physical_controls[]; . == "passed") and
    .wrong_target_control.guard_clean == true and .wrong_target_control.postcondition_failed == true and
    .wrong_target_control.rejected_after_clean_result == true and .wrong_target_control.run_rejected == true and
    (.sample_guard_rejection.mode == "held_state" or .sample_guard_rejection.mode == "sample_race") and
    .sample_guard_rejection.mode == $calibration_report[0].controls.sample_guard_rejection and
    .sample_guard_rejection.status == "passed"
  ' "$CALIBRATION_SUMMARY" >/dev/null || die "guard calibration summary is incomplete or its report digest is stale"

"$REBUILT_AUDITOR" --manifest "$MANIFEST" --jsonl "$RAW" > "$AUDIT_TMP" 2> "$AUDIT_STDERR_TMP"
require_clean_stderr "$AUDIT_STDERR_TMP"
jq -s -e --arg profile "$PROFILE" '
  length == 1 and (.[0] |
  .audit_report_version == 2 and .status == "evidence_valid" and
  .semantic_assessment == "observed_not_evaluated" and .profile == $profile and
  .fixture_revision == 2 and .checkpoint_count == 44 and .artifact_digests_match == true
  )
' "$AUDIT_TMP" >/dev/null || die "auditor output did not meet the expected report contract"
mv "$AUDIT_TMP" "$AUDIT_REPORT"

RAW_SHA=$(sha256_file "$RAW")
MANIFEST_SHA=$(sha256_file "$MANIFEST")
jq -e --arg raw "$RAW_SHA" --arg manifest "$MANIFEST_SHA" '
  .evidence_sha256.raw_jsonl == $raw and .evidence_sha256.manifest == $manifest
' "$AUDIT_REPORT" >/dev/null || die "audit report input digests mismatch"

if LC_ALL=C grep -En '/Users/|/private/|/tmp/|MIDI (In|Out)|source_instance_id|host_id_raw|gh[pousr]_[A-Za-z0-9_]|github_pat_' "$AUDIT_REPORT" >/dev/null; then
  die "audit report contains a forbidden path, port/raw-ID marker, or credential-shaped token"
fi

jq -n \
  --arg run "$RUN_ID" --arg inventory_sha "$EXPECTED_INVENTORY_SHA" \
  --arg guard_sha "$(sha256_file "$GUARD")" --arg trace_sha "$(sha256_file "$TRACE")" \
  --arg calibration_sha "$(sha256_file "$CALIBRATION_SUMMARY")" \
  --arg calibration_report_sha "$CALIBRATION_REPORT_SHA" \
  --arg calibration_checker_sha "$TRUSTED_CALIBRATION_CHECKER_SHA" \
  --arg evidence_checker_sha "$TRUSTED_EVIDENCE_CHECKER_SHA" \
  --arg audit_report_sha "$(sha256_file "$AUDIT_REPORT")" \
  --arg guard_session_id "$(jq -sr '.[0].guard_session_id' "$GUARD")" \
  --argjson guard_process_id "$(jq -sr '.[0].guard_process_id' "$GUARD")" \
  --argjson guard_started_at "$(jq -sr '.[0].guard_started_at_unix_ms' "$GUARD")" \
  --argjson action_count "$EXPECTED_ACTION_COUNT" '
  {
    action_evidence_report_version:2,
    status:"valid",
    run_id:$run,
    action_count:$action_count,
    guard_protocol_version:4,
    guard_policy:"consequential_input_only",
    guard_identity:{
      guard_session_id:$guard_session_id,
      guard_process_id:$guard_process_id,
      guard_started_at_unix_ms:$guard_started_at
    },
    all_consequential_deltas_zero:true,
    all_actions_inventory_ordered_and_adjacent:true,
    all_tool_calls_fresh_target_bound_single_call_with_confirmed_postcondition:true,
    all_screenshot_digests_recomputed_from_canonical_artifacts:true,
    all_state_digests_recomputed_from_canonical_artifacts:true,
    exact_context_calibration_passed:true,
    no_retry_within_process:true,
    checker_trust:{
      calibration_checker_compared_to_committed_source:true,
      final_checker_compared_to_committed_source:true
    },
    detached_checksum_strategy:{
      path:"artifact-sha256.txt",
      includes_every_canonical_regular_file_except_itself:true,
      includes_final_checker:true,
      self_exclusion_reason:"A digest index cannot include its own final digest without a circular dependency."
    },
    evidence_sha256:{
      inventory:$inventory_sha,
      guard_jsonl:$guard_sha,
      operator_tool_trace:$trace_sha,
      calibration_summary:$calibration_sha,
      calibration_report:$calibration_report_sha,
      calibration_checker:$calibration_checker_sha,
      evidence_checker:$evidence_checker_sha,
      audit_report:$audit_report_sha
    },
    limitations:[
      "guard identity correlates records but does not authenticate the actor or make evidence tamper-evident",
      "the isolated auditor rebuild assumes the local Rust toolchain and Cargo home configuration are trusted"
    ]
  }' > "$ACTION_TMP"
mv "$ACTION_TMP" "$ACTION_REPORT"

# Build a detached digest index after every other artifact is final. It includes
# both checker copies and all generated reports, but necessarily excludes itself.
cmp -s "$TRUSTED_CALIBRATION_CHECKER" "$CALIBRATION_CHECKER" || die "evidence calibration checker changed during verification"
cmp -s "$TRUSTED_EVIDENCE_CHECKER" "$EVIDENCE_CHECKER" || die "evidence final checker changed during verification"
(
  cd "$EVIDENCE_DIRECTORY"
  for relative_path in \
    "$RAW_NAME" collector.stderr.log audit-manifest-v1.json audit-report-v2.json \
    input-guard.jsonl input-guard.stderr.log ui-action-inventory.json operator-tool-trace.jsonl \
    operator-run-record.json guard-calibration-summary.json check-calibration.sh check-evidence.sh \
    guard-calibration-report.json action-evidence-report.json; do
    shasum -a 256 "$relative_path"
  done
  for relative_path in calibration/"$CALIBRATION_PREFIX"-automation.jsonl calibration/"$CALIBRATION_PREFIX"-automation.stderr calibration/"$CALIBRATION_PREFIX"-automation-trace.jsonl \
    calibration/"$CALIBRATION_PREFIX"-move.jsonl calibration/"$CALIBRATION_PREFIX"-move.stderr calibration/"$CALIBRATION_PREFIX"-move-trace.jsonl \
    calibration/"$CALIBRATION_PREFIX"-positive-click.jsonl calibration/"$CALIBRATION_PREFIX"-positive-click.stderr calibration/"$CALIBRATION_PREFIX"-positive-click-trace.jsonl \
    calibration/"$CALIBRATION_PREFIX"-positive-key.jsonl calibration/"$CALIBRATION_PREFIX"-positive-key.stderr calibration/"$CALIBRATION_PREFIX"-positive-key-trace.jsonl \
    calibration/"$CALIBRATION_PREFIX"-positive-scroll.jsonl calibration/"$CALIBRATION_PREFIX"-positive-scroll.stderr calibration/"$CALIBRATION_PREFIX"-positive-scroll-trace.jsonl \
    calibration/"$CALIBRATION_PREFIX"-positive-drag.jsonl calibration/"$CALIBRATION_PREFIX"-positive-drag.stderr calibration/"$CALIBRATION_PREFIX"-positive-drag-trace.jsonl \
    calibration/"$CALIBRATION_PREFIX"-wrong-target.jsonl calibration/"$CALIBRATION_PREFIX"-wrong-target.stderr calibration/"$CALIBRATION_PREFIX"-wrong-target-trace.jsonl \
    calibration/"$CALIBRATION_PREFIX"-sample-rejection.jsonl calibration/"$CALIBRATION_PREFIX"-sample-rejection.stderr calibration/"$CALIBRATION_PREFIX"-sample-rejection-trace.jsonl; do
    shasum -a 256 "$relative_path"
  done
  for absolute_path in \
    "$CALIBRATION_DIRECTORY"/screenshots/* "$CALIBRATION_DIRECTORY"/states/*.json \
    "$SCREENSHOT_DIRECTORY"/* "$STATE_DIRECTORY"/*.json; do
    relative_path=${absolute_path#"$EVIDENCE_DIRECTORY"/}
    shasum -a 256 "$relative_path"
  done
) | LC_ALL=C sort > "$SHA_TMP"

test "$(wc -l < "$SHA_TMP" | tr -d ' ')" -eq "$EXPECTED_DETACHED_INDEX_COUNT" ||
  die "detached checksum index has an unexpected entry count"
(
  cd "$EVIDENCE_DIRECTORY"
  shasum -a 256 -c "$SHA_TMP" >/dev/null
) || die "detached checksum verification failed"
mv "$SHA_TMP" "$ARTIFACT_SHA256"

final_entry_count=$(find "$EVIDENCE_DIRECTORY" -mindepth 1 -maxdepth 1 -print | wc -l | tr -d ' ')
test "$final_entry_count" -eq 18 || die "final evidence directory is not the exact canonical 18-entry set"
for file in "$AUDIT_REPORT" "$CALIBRATION_REPORT" "$ACTION_REPORT" "$ARTIFACT_SHA256"; do
  require_nonempty "$file"
done

trap - EXIT HUP INT TERM
cleanup_temporary_files
echo "check-track-probe-evidence: PASS"
echo "artifact-sha256.txt: $(sha256_file "$ARTIFACT_SHA256") (detached index; the index itself is intentionally not self-listed)"
