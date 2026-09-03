#!/usr/bin/env bash
set -euo pipefail

GUARD_VERSION=4

die() {
  echo "check-input-guard-calibration: $*" >&2
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

require_clean_stderr() {
  test -f "$1" || die "missing stderr file: $1"
  test ! -L "$1" || die "symbolic links are not accepted: $1"
  if LC_ALL=C grep -q '[^[:space:]]' "$1"; then
    die "guard stderr contains non-whitespace content: $1"
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

  # Signature bytes alone are not evidence that the artifact is an image. Ask
  # macOS ImageIO (through sips) to open it, and require real pixel dimensions
  # plus a format matching the extension.
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

hex64() {
  printf '%s\n' "$1" | LC_ALL=C grep -Eq '^[0-9a-f]{64}$'
}

test "$#" -eq 4 || die "usage: $0 CALIBRATION_DIRECTORY FILE_PREFIX GUARD_BINARY EXPECTED_GUARD_SHA256"
CALIBRATION_DIRECTORY=$1
FILE_PREFIX=$2
GUARD_BINARY=$3
EXPECTED_GUARD_SHA256=$4

for command_name in awk find grep jq od shasum sips tr; do
  require_command "$command_name"
done

test -d "$CALIBRATION_DIRECTORY" || die "calibration directory does not exist"
test ! -L "$CALIBRATION_DIRECTORY" || die "calibration directory may not be a symbolic link"
case "$FILE_PREFIX" in
  ''|*[!A-Za-z0-9._-]*) die "file prefix must contain only ASCII letters, digits, dot, underscore, or hyphen" ;;
esac
hex64 "$EXPECTED_GUARD_SHA256" || die "expected guard digest must be lowercase 64-hex"
test -x "$GUARD_BINARY" || die "guard binary is missing or not executable"
test ! -L "$GUARD_BINARY" || die "guard binary may not be a symbolic link"
test "$(sha256_file "$GUARD_BINARY")" = "$EXPECTED_GUARD_SHA256" || die "guard binary digest mismatch"

path_for() {
  printf '%s/%s-%s.%s' "$CALIBRATION_DIRECTORY" "$FILE_PREFIX" "$1" "$2"
}

AUTOMATION_GUARD=$(path_for automation jsonl)
AUTOMATION_STDERR=$(path_for automation stderr)
AUTOMATION_TRACE=$(path_for automation-trace jsonl)
MOVE_GUARD=$(path_for move jsonl)
MOVE_STDERR=$(path_for move stderr)
MOVE_TRACE=$(path_for move-trace jsonl)
CLICK_GUARD=$(path_for positive-click jsonl)
CLICK_STDERR=$(path_for positive-click stderr)
CLICK_TRACE=$(path_for positive-click-trace jsonl)
KEY_GUARD=$(path_for positive-key jsonl)
KEY_STDERR=$(path_for positive-key stderr)
KEY_TRACE=$(path_for positive-key-trace jsonl)
SCROLL_GUARD=$(path_for positive-scroll jsonl)
SCROLL_STDERR=$(path_for positive-scroll stderr)
SCROLL_TRACE=$(path_for positive-scroll-trace jsonl)
DRAG_GUARD=$(path_for positive-drag jsonl)
DRAG_STDERR=$(path_for positive-drag stderr)
DRAG_TRACE=$(path_for positive-drag-trace jsonl)
WRONG_GUARD=$(path_for wrong-target jsonl)
WRONG_STDERR=$(path_for wrong-target stderr)
WRONG_TRACE=$(path_for wrong-target-trace jsonl)
SAMPLE_GUARD=$(path_for sample-rejection jsonl)
SAMPLE_STDERR=$(path_for sample-rejection stderr)
SAMPLE_TRACE=$(path_for sample-rejection-trace jsonl)

# A calibration directory is a closed evidence set: exactly three records for
# each of eight canonical guard processes plus screenshot and state directories.
# Rejecting extras prevents a failed or retried run from being hidden beside the
# selected evidence.
entry_count=0
while IFS= read -r -d '' entry; do
  entry_count=$((entry_count + 1))
  entry_name=${entry##*/}
  case "$entry_name" in
    "$FILE_PREFIX-automation.jsonl"|"$FILE_PREFIX-automation.stderr"|"$FILE_PREFIX-automation-trace.jsonl"|\
    "$FILE_PREFIX-move.jsonl"|"$FILE_PREFIX-move.stderr"|"$FILE_PREFIX-move-trace.jsonl"|\
    "$FILE_PREFIX-positive-click.jsonl"|"$FILE_PREFIX-positive-click.stderr"|"$FILE_PREFIX-positive-click-trace.jsonl"|\
    "$FILE_PREFIX-positive-key.jsonl"|"$FILE_PREFIX-positive-key.stderr"|"$FILE_PREFIX-positive-key-trace.jsonl"|\
    "$FILE_PREFIX-positive-scroll.jsonl"|"$FILE_PREFIX-positive-scroll.stderr"|"$FILE_PREFIX-positive-scroll-trace.jsonl"|\
    "$FILE_PREFIX-positive-drag.jsonl"|"$FILE_PREFIX-positive-drag.stderr"|"$FILE_PREFIX-positive-drag-trace.jsonl"|\
    "$FILE_PREFIX-wrong-target.jsonl"|"$FILE_PREFIX-wrong-target.stderr"|"$FILE_PREFIX-wrong-target-trace.jsonl"|\
    "$FILE_PREFIX-sample-rejection.jsonl"|"$FILE_PREFIX-sample-rejection.stderr"|"$FILE_PREFIX-sample-rejection-trace.jsonl")
      test -f "$entry" || die "canonical calibration artifact is not a regular file: $entry"
      test ! -L "$entry" || die "symbolic links are not accepted in calibration evidence: $entry"
      ;;
    screenshots|states)
      test -d "$entry" || die "$entry_name entry is not a directory"
      test ! -L "$entry" || die "$entry_name directory may not be a symbolic link"
      ;;
    *) die "unexpected calibration artifact: $entry_name" ;;
  esac
done < <(find "$CALIBRATION_DIRECTORY" -mindepth 1 -maxdepth 1 -print0)
test "$entry_count" -eq 26 || die "calibration directory must contain 24 canonical files, screenshots/, and states/ (found $entry_count entries)"

GUARD_FILES=(
  "$AUTOMATION_GUARD" "$MOVE_GUARD"
  "$CLICK_GUARD" "$KEY_GUARD" "$SCROLL_GUARD" "$DRAG_GUARD"
  "$WRONG_GUARD" "$SAMPLE_GUARD"
)
TRACE_FILES=(
  "$AUTOMATION_TRACE" "$MOVE_TRACE"
  "$CLICK_TRACE" "$KEY_TRACE" "$SCROLL_TRACE" "$DRAG_TRACE"
  "$WRONG_TRACE" "$SAMPLE_TRACE"
)
STDERR_FILES=(
  "$AUTOMATION_STDERR" "$MOVE_STDERR"
  "$CLICK_STDERR" "$KEY_STDERR" "$SCROLL_STDERR" "$DRAG_STDERR"
  "$WRONG_STDERR" "$SAMPLE_STDERR"
)

for file in "${GUARD_FILES[@]}" "${TRACE_FILES[@]}"; do
  require_nonempty "$file"
  jq -e . "$file" >/dev/null || die "invalid JSONL: $file"
done
for file in "${STDERR_FILES[@]}"; do
  require_clean_stderr "$file"
done

validate_trace_timestamps() {
  local trace_file=$1
  jq -s -e '
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
    [.[1:-1][]] as $actions |
    ([.[0].started_at] + [$actions[] | .pre_state.captured_at, .call_started_at, .call_ended_at, .post_state.captured_at] + [.[-1].ended_at]) as $times |
    ($times[0] | zone) as $zone |
    all($times[]; timestamp) and
    all($times[]; zone == $zone) and
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
    $actions[-1].post_state.captured_at < $records[-1].ended_at
  ' "$trace_file" >/dev/null || die "trace timestamps must be in one timezone and strictly increase from session start through session end: $trace_file"
}

for file in "${TRACE_FILES[@]}"; do
  validate_trace_timestamps "$file"
  jq -s -e 'all(.[]; (has("retry_attempted") | not) and (has("completed_without_retry") | not))' "$file" >/dev/null ||
    die "legacy ambiguous retry fields are forbidden; use no_retry_within_process: $file"
done

# Each action carries relative paths for its actual pre/post screenshots and
# state dumps. Paths are deliberately constrained to flat canonical directories.
# This makes traversal impossible and lets the checker reject missing, altered,
# and extra artifacts.
SCREENSHOT_REFS=$(jq -sc '
  [.[] | select(.record_type == "action") |
    {path: .pre_state.screenshot_path, sha256: .pre_state.screenshot_sha256},
    {path: .post_state.screenshot_path, sha256: .post_state.screenshot_sha256}]
' "${TRACE_FILES[@]}")
jq -e '
  length == 28 and
  all(.[];
    (.path | type == "string" and test("^screenshots/[A-Za-z0-9][A-Za-z0-9._-]*\\.(png|jpg|jpeg)$")) and
    (.sha256 | type == "string" and test("^[0-9a-f]{64}$"))
  ) and
  ((map(.path) | unique | length) == 28)
' <<<"$SCREENSHOT_REFS" >/dev/null || die "every calibration action must name two unique canonical screenshot files"

screenshot_count=0
while IFS= read -r -d '' screenshot; do
  screenshot_count=$((screenshot_count + 1))
  test -f "$screenshot" || die "unexpected non-file in screenshots directory: $screenshot"
  test ! -L "$screenshot" || die "screenshot may not be a symbolic link: $screenshot"
  validate_screenshot_image "$screenshot"
  relative_path="screenshots/${screenshot##*/}"
  expected_sha=$(jq -er --arg path "$relative_path" '.[] | select(.path == $path) | .sha256' <<<"$SCREENSHOT_REFS") || die "extra screenshot not referenced by a trace: $relative_path"
  test "$(sha256_file "$screenshot")" = "$expected_sha" || die "screenshot digest mismatch: $relative_path"
done < <(find "$CALIBRATION_DIRECTORY/screenshots" -mindepth 1 -maxdepth 1 -print0)
test "$screenshot_count" -eq 28 || die "screenshots directory must contain exactly the 28 trace-bound files (found $screenshot_count)"

STATE_REFS=$(jq -sc '
  [.[] | select(.record_type == "action") |
    {path: .pre_state.state_path, sha256: .pre_state.state_sha256, captured_at: .pre_state.captured_at, app: .pre_state.app},
    {path: .post_state.state_path, sha256: .post_state.state_sha256, captured_at: .post_state.captured_at, app: .post_state.app}]
' "${TRACE_FILES[@]}")
jq -e '
  length == 28 and
  all(.[];
    (.path | type == "string" and test("^states/[A-Za-z0-9][A-Za-z0-9._-]*\\.json$")) and
    (.sha256 | type == "string" and test("^[0-9a-f]{64}$")) and
    (.captured_at | type == "string" and length > 0) and
    (.app | type == "string" and length > 0)
  ) and
  ((map(.path) | unique | length) == 28)
' <<<"$STATE_REFS" >/dev/null || die "every calibration action must name two unique canonical JSON state dumps"

state_count=0
while IFS= read -r -d '' state_dump; do
  state_count=$((state_count + 1))
  test -f "$state_dump" || die "unexpected non-file in states directory: $state_dump"
  test ! -L "$state_dump" || die "state dump may not be a symbolic link: $state_dump"
  relative_path="states/${state_dump##*/}"
  expected_sha=$(jq -er --arg path "$relative_path" '.[] | select(.path == $path) | .sha256' <<<"$STATE_REFS") || die "extra state dump not referenced by a trace: $relative_path"
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
done < <(find "$CALIBRATION_DIRECTORY/states" -mindepth 1 -maxdepth 1 -print0)
test "$state_count" -eq 28 || die "states directory must contain exactly the 28 trace-bound files (found $state_count)"

validate_guard_identity() {
  local guard_file=$1
  jq -s -e --argjson version "$GUARD_VERSION" '
    def hex64: type == "string" and test("^[0-9a-f]{64}$");
    . as $records |
    .[0] as $ready |
    $ready.guard_session_id as $session_id |
    $ready.guard_process_id as $process_id |
    $ready.guard_started_at_unix_ms as $started_at |
    length > 0 and
    ($ready.guard_session_id | hex64) and
    ($ready.guard_process_id | type == "number" and . > 0 and floor == .) and
    ($ready.guard_started_at_unix_ms | type == "number" and . > 0 and floor == .) and
    all(.[];
      .version == $version and
      .coverage == "action_windows" and .policy == "consequential_input_only" and
      .guard_session_id == $session_id and
      .guard_process_id == $process_id and
      .guard_started_at_unix_ms == $started_at
    ) and
    ([.[].record_sequence] == [range(1; length + 1)])
  ' "$guard_file" >/dev/null || die "guard v4 identity or record sequence invalid: $guard_file"
}

validate_trace_identity() {
  local guard_file=$1
  local trace_file=$2
  jq -s -e --slurpfile guard "$guard_file" --arg guard_sha "$EXPECTED_GUARD_SHA256" '
    def hex64: type == "string" and test("^[0-9a-f]{64}$");
    length > 0 and
    .[0].calibration_trace_version == 1 and .[0].record_type == "session" and
    .[0].fresh_guard_process == true and
    .[0].guard_binary_sha256 == $guard_sha and
    (.[0].guard_session_id | hex64) and
    .[0].guard_session_id == $guard[0].guard_session_id and
    .[0].guard_process_id == $guard[0].guard_process_id and
    .[0].guard_started_at_unix_ms == $guard[0].guard_started_at_unix_ms
  ' "$trace_file" >/dev/null || die "trace session header does not bind the canonical guard identity: $trace_file"
}

index=0
while test "$index" -lt 8; do
  validate_guard_identity "${GUARD_FILES[$index]}"
  validate_trace_identity "${GUARD_FILES[$index]}" "${TRACE_FILES[$index]}"
  index=$((index + 1))
done

# Freshness is derived from the canonical ready records, not from operator trace
# booleans. PID, start time, and session correlation ID must all identify eight
# distinct processes, and each stream has already been checked for identity
# continuity above.
jq -s -e '
  [.[] | select(.type == "ready")] as $ready |
  ($ready | length) == 8 and
  (([$ready[].guard_session_id] | unique | length) == 8) and
  (([$ready[].guard_process_id] | unique | length) == 8) and
  (([$ready[].guard_started_at_unix_ms] | unique | length) == 8) and
  (([$ready[] | [.guard_session_id, .guard_process_id, .guard_started_at_unix_ms]] | unique | length) == 8)
' "${GUARD_FILES[@]}" >/dev/null || die "canonical guard records do not prove eight distinct fresh process identities"

AUTOMATION_IDS='["cal.automation.open","cal.automation.press-key","cal.automation.set-value","cal.automation.semantic-click","cal.automation.coordinate-single","cal.automation.coordinate-double"]'
AUTOMATION_APIS='["exec_command.open","computer_use.press_key","computer_use.set_value","computer_use.click.element","computer_use.click.coordinate","computer_use.click.coordinate"]'

jq -s -e --argjson ids "$AUTOMATION_IDS" '
  def counter_keys: ["flags_changed","key_down","key_up","left_mouse_down","left_mouse_dragged","left_mouse_up","mouse_moved","other_mouse_down","other_mouse_dragged","other_mouse_up","right_mouse_down","right_mouse_dragged","right_mouse_up","scroll_wheel","tablet_pointer","tablet_proximity"] | sort;
  def counters_valid:
    (.deltas | keys | sort) == counter_keys and
    all(.deltas[]; type == "number" and . >= 0 and floor == . and . <= 4294967295);
  def clean_result:
    .type == "result" and .source == "hid_system_state" and
    .interference_detected == false and counters_valid and
    all(.deltas | to_entries[] | select(.key != "mouse_moved"); .value == 0);
  . as $records |
  length == 14 and
  .[0].type == "ready" and .[0].source == "hid_system_state" and
  .[0].privacy == "counts_and_held_state_boolean" and
  .[-1].type == "finished" and .[-1].source == "hid_system_state" and
  .[-1].interference_detected == false and
  ([.[] | select(.type == "armed") | .action_id] == $ids) and
  ([.[] | select(.type == "result") | .action_id] == $ids) and
  ([.[] | select(.type == "error" or .type == "cancelled" or .type == "rejected" or .type == "pong")] | length) == 0 and
  all(range(0; 6); . as $i |
    ($i * 2 + 1) as $armed | ($i * 2 + 2) as $result |
    $records[$armed].type == "armed" and $records[$armed].source == "hid_system_state" and
    $records[$armed].action_id == $ids[$i] and
    $records[$result].action_id == $ids[$i] and ($records[$result] | clean_result)
  )
' "$AUTOMATION_GUARD" >/dev/null || die "automation guard stream invalid"

jq -s -e --arg guard_sha "$EXPECTED_GUARD_SHA256" --argjson ids "$AUTOMATION_IDS" --argjson apis "$AUTOMATION_APIS" '
  def hex64: type == "string" and test("^[0-9a-f]{64}$");
  def nonempty: type == "string" and length > 0;
  def timestamp: type == "string" and test("^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}\\.[0-9]{3}(Z|[+-][0-9]{2}:[0-9]{2})$");
  def common_action:
    .calibration_trace_version == 1 and .record_type == "action" and
    (.session_id | nonempty) and (.timestamp | timestamp) and
    .guard.armed_observed == true and .guard.result_observed == true and
    .guard.interference_detected == false and .guard.consequential_deltas_zero == true and
    .pre_state.fresh == true and .pre_state.expected_condition_confirmed == true and
    (.pre_state.state_sha256 | hex64) and (.pre_state.screenshot_sha256 | hex64) and (.pre_state.app | nonempty) and
    .injected_call_count == 1 and .target_binding.resolved_fresh == true and .call_succeeded == true and
    .post_state.fresh == true and .post_state.expected_condition_confirmed == true and
    (.post_state.state_sha256 | hex64) and (.post_state.screenshot_sha256 | hex64) and (.post_state.app | nonempty) and
    (.expected_postcondition | nonempty) and (.observed_postcondition | nonempty) and
    .postcondition_confirmed == true and .no_retry_within_process == true;
  . as $records |
  length == 8 and
  .[0].control == "automation_negative" and (.[0].session_id | nonempty) and
  (.[0].started_at | timestamp) and
  .[-1].calibration_trace_version == 1 and .[-1].record_type == "session_end" and
  .[-1].session_id == .[0].session_id and (.[-1].ended_at | timestamp) and
  .[-1].guard_process_exit_status == 0 and .[-1].finished_observed == true and
  .[-1].no_retry_within_process == true and
  ([.[1:7][] | .action_id] == $ids) and ([.[1:7][] | .api] == $apis) and
  all(range(0; 6); . as $i |
    ($records[$i + 1] | common_action) and
    $records[$i + 1].session_id == $records[0].session_id and
    $records[$i + 1].ordinal == ($i + 1)
  ) and
  all($records[1:7][]; .pre_state.app == "Finder" and .post_state.app == "Finder") and
  $records[1].target_binding.kind == "os_open" and
  $records[1].target_binding.exact_application_or_bundle_path == "/System/Library/CoreServices/Finder.app" and
  ($records[1].target_binding.absolute_document_path | startswith("/")) and
  $records[2].target_binding.kind == "key_chord" and
  ($records[2].target_binding.application | nonempty) and ($records[2].target_binding.window | nonempty) and
  ($records[2].target_binding.keys | type == "array" and length > 0 and all(.[]; nonempty)) and
  $records[3].target_binding.kind == "semantic_element" and
  ($records[3].target_binding.application | nonempty) and ($records[3].target_binding.window | nonempty) and
  ($records[3].target_binding.element_identity | nonempty) and ($records[3].target_binding.value_sha256 | hex64) and
  $records[4].target_binding.kind == "semantic_element" and
  ($records[4].target_binding.application | nonempty) and ($records[4].target_binding.window | nonempty) and
  ($records[4].target_binding.element_identity | nonempty) and $records[4].target_binding.click_count == 1 and
  all($records[2:7][]; .target_binding.application == .pre_state.app) and
  all($records[5:7][];
    .target_binding.kind == "window_coordinate" and
    (.target_binding.application | nonempty) and (.target_binding.window | nonempty) and
    (.target_binding.x | type == "number") and (.target_binding.y | type == "number") and
    (.target_binding.fresh_screenshot_sha256 | hex64) and
    .target_binding.fresh_screenshot_sha256 == .pre_state.screenshot_sha256
  ) and
  $records[5].target_binding.click_count == 1 and $records[6].target_binding.click_count == 2
' "$AUTOMATION_TRACE" >/dev/null || die "automation trace invalid"

MOVE_IDS='["cal.move-only.semantic","cal.move-only.coordinate"]'
jq -s -e --argjson ids "$MOVE_IDS" '
  def counter_keys: ["flags_changed","key_down","key_up","left_mouse_down","left_mouse_dragged","left_mouse_up","mouse_moved","other_mouse_down","other_mouse_dragged","other_mouse_up","right_mouse_down","right_mouse_dragged","right_mouse_up","scroll_wheel","tablet_pointer","tablet_proximity"] | sort;
  def move_result:
    .type == "result" and .source == "hid_system_state" and .interference_detected == false and
    (.deltas | keys | sort) == counter_keys and
    all(.deltas[]; type == "number" and . >= 0 and floor == . and . <= 4294967295) and
    .deltas.mouse_moved >= 1 and
    all(.deltas | to_entries[] | select(.key != "mouse_moved"); .value == 0);
  . as $records |
  length == 6 and .[0].type == "ready" and .[0].source == "hid_system_state" and
  .[0].privacy == "counts_and_held_state_boolean" and
  .[-1].type == "finished" and .[-1].source == "hid_system_state" and .[-1].interference_detected == false and
  all(range(0; 2); . as $i |
    ($i * 2 + 1) as $armed | ($i * 2 + 2) as $result |
    $records[$armed].type == "armed" and $records[$armed].source == "hid_system_state" and
    $records[$armed].action_id == $ids[$i] and
    $records[$result].action_id == $ids[$i] and ($records[$result] | move_result)
  ) and
  ([.[] | select(.type == "error" or .type == "cancelled" or .type == "rejected" or .type == "pong")] | length) == 0
' "$MOVE_GUARD" >/dev/null || die "move-only guard stream invalid"

jq -s -e --argjson ids "$MOVE_IDS" '
  def hex64: type == "string" and test("^[0-9a-f]{64}$");
  def nonempty: type == "string" and length > 0;
  def timestamp: type == "string" and test("^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}\\.[0-9]{3}(Z|[+-][0-9]{2}:[0-9]{2})$");
  def common_move:
    .calibration_trace_version == 1 and .record_type == "action" and
    (.session_id | nonempty) and (.timestamp | timestamp) and
    .guard.armed_observed == true and .guard.result_observed == true and
    .guard.interference_detected == false and .guard.mouse_moved_at_least_one == true and
    .guard.consequential_deltas_zero == true and
    .physical_input.kind == "pointer_move_only" and .physical_input.operator_attested == true and
    .physical_input.began_after_armed == true and .physical_input.ended_before_check == true and
    .physical_input.no_button_key_scroll_or_drag == true and
    .pre_state.fresh == true and .pre_state.expected_condition_confirmed == true and
    (.pre_state.state_sha256 | hex64) and (.pre_state.screenshot_sha256 | hex64) and (.pre_state.app | nonempty) and
    .injected_call_count == 1 and .target_binding.resolved_fresh == true and .call_succeeded == true and
    .post_state.fresh == true and .post_state.expected_condition_confirmed == true and
    (.post_state.state_sha256 | hex64) and (.post_state.screenshot_sha256 | hex64) and (.post_state.app | nonempty) and
    (.expected_postcondition | nonempty) and (.observed_postcondition | nonempty) and
    .postcondition_confirmed == true and .no_retry_within_process == true;
  . as $records |
  length == 4 and
  .[0].control == "move_only_acceptance" and (.[0].session_id | nonempty) and (.[0].started_at | timestamp) and
  .[-1].record_type == "session_end" and .[-1].session_id == .[0].session_id and
  (.[-1].ended_at | timestamp) and .[-1].guard_process_exit_status == 0 and
  .[-1].finished_observed == true and .[-1].no_retry_within_process == true and
  ([.[1:3][] | .action_id] == $ids) and
  all(range(0; 2); . as $i |
    ($records[$i + 1] | common_move) and
    $records[$i + 1].session_id == $records[0].session_id and $records[$i + 1].ordinal == ($i + 1)
  ) and
  all($records[1:3][]; .pre_state.app == "Finder" and .post_state.app == "Finder" and .target_binding.application == "Finder") and
  $records[1].api == "computer_use.click.element" and
  $records[1].target_binding.kind == "semantic_element" and
  ($records[1].target_binding.application | nonempty) and ($records[1].target_binding.window | nonempty) and
  ($records[1].target_binding.element_identity | nonempty) and $records[1].target_binding.click_count == 1 and
  $records[2].api == "computer_use.click.coordinate" and
  $records[2].target_binding.kind == "window_coordinate" and
  ($records[2].target_binding.application | nonempty) and ($records[2].target_binding.window | nonempty) and
  ($records[2].target_binding.x | type == "number") and ($records[2].target_binding.y | type == "number") and
  ($records[2].target_binding.fresh_screenshot_sha256 | hex64) and
  $records[2].target_binding.fresh_screenshot_sha256 == $records[2].pre_state.screenshot_sha256 and
  $records[2].target_binding.click_count == 1
' "$MOVE_TRACE" >/dev/null || die "move-only trace invalid"

validate_positive() {
  local kind=$1
  local action_id=$2
  local physical_kind=$3
  local guard_file=$4
  local trace_file=$5

  jq -s -e --arg kind "$kind" --arg action_id "$action_id" '
    def counter_keys: ["flags_changed","key_down","key_up","left_mouse_down","left_mouse_dragged","left_mouse_up","mouse_moved","other_mouse_down","other_mouse_dragged","other_mouse_up","right_mouse_down","right_mouse_dragged","right_mouse_up","scroll_wheel","tablet_pointer","tablet_proximity"] | sort;
    def counters_valid($d):
      ($d | keys | sort) == counter_keys and all($d[]; type == "number" and . >= 0 and floor == . and . <= 4294967295);
    def zero_except($d; $allowed):
      all($d | to_entries[]; . as $entry | if ($allowed | index($entry.key)) == null then $entry.value == 0 else true end);
    . as $records | .[2].deltas as $d |
    length == 4 and
    .[0].type == "ready" and .[0].source == "hid_system_state" and .[0].privacy == "counts_and_held_state_boolean" and
    .[1].type == "armed" and .[1].source == "hid_system_state" and .[1].action_id == $action_id and
    .[2].type == "result" and .[2].source == "hid_system_state" and .[2].action_id == $action_id and
    .[2].interference_detected == true and counters_valid($d) and
    .[3].type == "error" and .[3].error.code == "INTERFERENCE_LATCHED" and
    (if $kind == "click" then
      $d.left_mouse_down >= 1 and $d.left_mouse_up >= 1 and zero_except($d; ["mouse_moved","left_mouse_down","left_mouse_up"])
     elif $kind == "key" then
      $d.key_down >= 1 and $d.key_up >= 1 and zero_except($d; ["mouse_moved","key_down","key_up"])
     elif $kind == "scroll" then
      $d.scroll_wheel >= 1 and zero_except($d; ["mouse_moved","scroll_wheel"])
     elif $kind == "drag" then
      $d.left_mouse_down >= 1 and $d.left_mouse_dragged >= 1 and $d.left_mouse_up >= 1 and
      zero_except($d; ["mouse_moved","left_mouse_down","left_mouse_dragged","left_mouse_up"])
     else false end)
  ' "$guard_file" >/dev/null || die "positive $kind guard stream invalid"

  jq -s -e --arg kind "$kind" --arg action_id "$action_id" --arg physical_kind "$physical_kind" '
    def hex64: type == "string" and test("^[0-9a-f]{64}$");
    def nonempty: type == "string" and length > 0;
    def timestamp: type == "string" and test("^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}\\.[0-9]{3}(Z|[+-][0-9]{2}:[0-9]{2})$");
    length == 3 and
    .[0].control == ("consequential_positive_" + $kind) and (.[0].session_id | nonempty) and (.[0].started_at | timestamp) and
    .[1].calibration_trace_version == 1 and .[1].record_type == "action" and
    .[1].session_id == .[0].session_id and .[1].action_id == $action_id and
    .[1].api == "physical_input" and .[1].injected_call_count == 0 and
    .[1].physical_input.kind == $physical_kind and .[1].physical_input.operator_attested == true and
    .[1].physical_input.began_after_armed == true and .[1].physical_input.ended_before_check == true and
    .[1].physical_input.no_additional_input_class == true and
    .[1].pre_state.fresh == true and .[1].pre_state.expected_condition_confirmed == true and
    (.[1].pre_state.state_sha256 | hex64) and (.[1].pre_state.screenshot_sha256 | hex64) and (.[1].pre_state.app | nonempty) and
    .[1].post_state.fresh == true and .[1].post_state.expected_condition_confirmed == true and
    (.[1].post_state.state_sha256 | hex64) and (.[1].post_state.screenshot_sha256 | hex64) and
    .[1].pre_state.app == "Finder" and .[1].post_state.app == "Finder" and
    .[1].guard.armed_observed == true and .[1].guard.result_observed == true and
    .[1].guard.interference_detected == true and .[1].guard.expected_counter_positive == true and
    .[1].finish_sent_after_result == true and .[1].latched_finish_error_confirmed == true and
    .[1].continued_after_latch == false and .[1].no_retry_within_process == true and (.[1].timestamp | timestamp) and
    .[2].calibration_trace_version == 1 and .[2].record_type == "session_end" and
    .[2].session_id == .[0].session_id and (.[2].ended_at | timestamp) and
    .[2].guard_process_exit_status == 1 and .[2].terminal_guard_error == "INTERFERENCE_LATCHED" and
    .[2].no_retry_within_process == true
  ' "$trace_file" >/dev/null || die "positive $kind trace invalid"
}

validate_positive click 'cal.physical-click' left_mouse_click "$CLICK_GUARD" "$CLICK_TRACE"
validate_positive key 'cal.physical-key' keyboard_nonmodifier_tap "$KEY_GUARD" "$KEY_TRACE"
validate_positive scroll 'cal.physical-scroll' scroll "$SCROLL_GUARD" "$SCROLL_TRACE"
validate_positive drag 'cal.physical-drag' left_mouse_drag "$DRAG_GUARD" "$DRAG_TRACE"

# A valid coordinate can still target the wrong element.  It must be checked
# first (so the clean HID result is preserved), then explicitly rejected, and a
# subsequent finish must fail because the process is aborted.
jq -s -e '
  def counter_keys: ["flags_changed","key_down","key_up","left_mouse_down","left_mouse_dragged","left_mouse_up","mouse_moved","other_mouse_down","other_mouse_dragged","other_mouse_up","right_mouse_down","right_mouse_dragged","right_mouse_up","scroll_wheel","tablet_pointer","tablet_proximity"] | sort;
  length == 5 and
  .[0].type == "ready" and .[0].source == "hid_system_state" and .[0].privacy == "counts_and_held_state_boolean" and
  .[1].type == "armed" and .[1].source == "hid_system_state" and .[1].action_id == "cal.wrong-target" and
  .[2].type == "result" and .[2].source == "hid_system_state" and .[2].action_id == "cal.wrong-target" and
  .[2].interference_detected == false and (.[2].deltas | keys | sort) == counter_keys and
  all(.[2].deltas[]; type == "number" and . >= 0 and floor == . and . <= 4294967295) and
  all(.[2].deltas | to_entries[] | select(.key != "mouse_moved"); .value == 0) and
  .[3].type == "rejected" and .[3].action_id == "cal.wrong-target" and
  .[3].reason == "postcondition_failed" and .[3].after_clean_result == true and .[3].session_aborted == true and
  .[4].type == "error" and .[4].error.code == "SESSION_ABORTED"
' "$WRONG_GUARD" >/dev/null || die "wrong-target guard stream must be ready/armed/clean-result/rejected/SESSION_ABORTED"

jq -s -e '
  def hex64: type == "string" and test("^[0-9a-f]{64}$");
  def nonempty: type == "string" and length > 0;
  def timestamp: type == "string" and test("^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}\\.[0-9]{3}(Z|[+-][0-9]{2}:[0-9]{2})$");
  length == 3 and
  .[0].control == "target_binding_rejection" and (.[0].session_id | nonempty) and (.[0].started_at | timestamp) and
  .[1].calibration_trace_version == 1 and .[1].record_type == "action" and
  .[1].session_id == .[0].session_id and .[1].action_id == "cal.wrong-target" and
  .[1].api == "computer_use.click.coordinate" and .[1].injected_call_count == 1 and
  .[1].target_binding.kind == "window_coordinate" and .[1].target_binding.resolved_fresh == true and
  (.[1].target_binding.application | nonempty) and (.[1].target_binding.window | nonempty) and
  (.[1].target_binding.x | type == "number") and (.[1].target_binding.y | type == "number") and
  .[1].target_binding.click_count == 1 and (.[1].target_binding.fresh_screenshot_sha256 | hex64) and
  .[1].target_binding.fresh_screenshot_sha256 == .[1].pre_state.screenshot_sha256 and
  .[1].target_binding.coordinate_within_current_window == true and .[1].target_binding.intentionally_wrong == true and
  .[1].pre_state.fresh == true and .[1].pre_state.expected_condition_confirmed == true and
  (.[1].pre_state.state_sha256 | hex64) and (.[1].pre_state.screenshot_sha256 | hex64) and (.[1].pre_state.app | nonempty) and
  .[1].call_succeeded == true and
  .[1].post_state.fresh == true and .[1].post_state.expected_condition_confirmed == false and
  (.[1].post_state.state_sha256 | hex64) and (.[1].post_state.screenshot_sha256 | hex64) and
  .[1].pre_state.app == "Finder" and .[1].post_state.app == "Finder" and .[1].target_binding.application == "Finder" and
  (.[1].expected_postcondition | nonempty) and (.[1].observed_postcondition | nonempty) and
  .[1].postcondition_confirmed == false and
  .[1].check_sent == true and .[1].reject_sent == true and .[1].cancel_sent == false and
  .[1].guard.armed_observed == true and .[1].guard.result_observed == true and
  .[1].guard.interference_detected == false and .[1].guard.consequential_deltas_zero == true and
  .[1].guard.rejected_observed == true and .[1].guard.reject_reason == "postcondition_failed" and
  .[1].guard.reject_after_clean_result == true and .[1].guard.session_aborted_finish_error_observed == true and
  .[1].run_rejected == true and .[1].no_retry_within_process == true and (.[1].timestamp | timestamp) and
  .[2].record_type == "session_end" and .[2].session_id == .[0].session_id and
  (.[2].ended_at | timestamp) and .[2].guard_process_exit_status == 1 and
  .[2].terminal_guard_error == "SESSION_ABORTED" and .[2].no_retry_within_process == true
' "$WRONG_TRACE" >/dev/null || die "wrong-target trace invalid"

jq -s -e '
  length == 2 and
  .[0].type == "ready" and .[0].source == "hid_system_state" and .[0].privacy == "counts_and_held_state_boolean" and
  .[1].type == "error" and
  (.[1].error.code == "KEY_HELD" or .[1].error.code == "MOUSE_BUTTON_HELD" or .[1].error.code == "INPUT_DURING_SAMPLE")
' "$SAMPLE_GUARD" >/dev/null || die "held/sample-race guard stream invalid"

jq -s -e --slurpfile guard "$SAMPLE_GUARD" '
  def hex64: type == "string" and test("^[0-9a-f]{64}$");
  def nonempty: type == "string" and length > 0;
  def timestamp: type == "string" and test("^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}\\.[0-9]{3}(Z|[+-][0-9]{2}:[0-9]{2})$");
  ($guard[1].error.code) as $code |
  length == 3 and
  .[0].record_type == "session" and .[0].control == "sample_guard_rejection" and
  (.[0].session_id | nonempty) and (.[0].started_at | timestamp) and
  .[1].record_type == "action" and .[1].session_id == .[0].session_id and
  .[1].action_id == "cal.sample-rejection" and .[1].api == "physical_input" and .[1].injected_call_count == 0 and
  .[1].guard_command_phase == "arm" and .[1].physical_input.operator_attested == true and
  .[1].guard.ready_observed == true and .[1].guard.armed_observed == false and .[1].guard.observed_error == $code and
  .[1].pre_state.fresh == true and .[1].pre_state.expected_condition_confirmed == true and
  (.[1].pre_state.state_sha256 | hex64) and (.[1].pre_state.screenshot_sha256 | hex64) and (.[1].pre_state.app | nonempty) and
  .[1].post_state.fresh == true and .[1].post_state.expected_condition_confirmed == true and
  (.[1].post_state.state_sha256 | hex64) and (.[1].post_state.screenshot_sha256 | hex64) and
  .[1].pre_state.app == "Finder" and .[1].post_state.app == "Finder" and
  (if $code == "INPUT_DURING_SAMPLE" then
    .[1].mode == "sample_race" and .[1].physical_input.kind == "pointer_move_during_sample" and
    .[1].physical_input.continuous_during_arm_sample == true
   elif $code == "KEY_HELD" then
    .[1].mode == "held_state" and .[1].physical_input.kind == "keyboard_key_held" and
    .[1].physical_input.began_before_arm_command == true and .[1].physical_input.held_through_sample == true
   else
    .[1].mode == "held_state" and .[1].physical_input.kind == "mouse_button_held" and
    .[1].physical_input.began_before_arm_command == true and .[1].physical_input.held_through_sample == true
   end) and
  .[1].no_retry_within_process == true and (.[1].timestamp | timestamp) and
  .[2].record_type == "session_end" and .[2].session_id == .[0].session_id and
  (.[2].ended_at | timestamp) and .[2].guard_process_exit_status == 1 and
  .[2].terminal_guard_error == $code and .[2].no_retry_within_process == true
' "$SAMPLE_TRACE" >/dev/null || die "held/sample-race trace invalid"

EVIDENCE='{}'
SESSIONS='{}'
for name in automation move positive-click positive-key positive-scroll positive-drag wrong-target sample-rejection; do
  guard_file=$(path_for "$name" jsonl)
  stderr_file=$(path_for "$name" stderr)
  trace_file=$(path_for "$name-trace" jsonl)
  EVIDENCE=$(jq -nc \
    --argjson evidence "$EVIDENCE" \
    --arg name "$name" \
    --arg guard "$(sha256_file "$guard_file")" \
    --arg stderr "$(sha256_file "$stderr_file")" \
    --arg trace "$(sha256_file "$trace_file")" \
    '$evidence + {($name): {guard_jsonl: $guard, guard_stderr: $stderr, operator_trace_jsonl: $trace}}')
  SESSIONS=$(jq -nc \
    --argjson sessions "$SESSIONS" \
    --arg name "$name" \
    --slurpfile guard "$guard_file" \
    '$sessions + {($name): {
      guard_session_id: $guard[0].guard_session_id,
      guard_process_id: $guard[0].guard_process_id,
      guard_started_at_unix_ms: $guard[0].guard_started_at_unix_ms
    }}')
done

SAMPLE_REJECTION_MODE=$(jq -rs 'if .[1].error.code == "INPUT_DURING_SAMPLE" then "sample_race" else "held_state" end' "$SAMPLE_GUARD")
CHECKER_SHA=$(sha256_file "$0")

jq -n \
  --arg prefix "$FILE_PREFIX" \
  --arg guard_binary_sha256 "$EXPECTED_GUARD_SHA256" \
  --arg checker_sha256 "$CHECKER_SHA" \
  --arg sample_rejection_mode "$SAMPLE_REJECTION_MODE" \
  --argjson evidence "$EVIDENCE" \
  --argjson sessions "$SESSIONS" \
  '{
    calibration_report_version: 2,
    status: "valid",
    file_prefix: $prefix,
    guard_contract: {
      version: 4,
      source: "hid_system_state",
      coverage: "action_windows",
      privacy: "counts_and_held_state_boolean",
      policy: "consequential_input_only",
      binary_sha256: $guard_binary_sha256
    },
    checker_sha256: $checker_sha256,
    controls: {
      automation_negative: ["exec_command.open","computer_use.press_key","computer_use.set_value","computer_use.click.element","computer_use.click.coordinate.single","computer_use.click.coordinate.double"],
      move_only_acceptance: ["semantic_target_binding","coordinate_target_binding"],
      consequential_positive: ["physical_click","physical_key","physical_scroll","physical_drag"],
      target_binding_rejection: "wrong_valid_coordinate_rejected_after_clean_result",
      sample_guard_rejection: $sample_rejection_mode
    },
    fresh_guard_identity_count: 8,
    all_pre_post_artifact_digests_recomputed: true,
    guard_sessions: $sessions,
    evidence_sha256: $evidence,
    limitations: [
      "guard session identity fields correlate records; they do not authenticate the actor or make the JSONL tamper-evident"
    ]
  }'
