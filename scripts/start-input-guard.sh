#!/usr/bin/env bash
set -euo pipefail
umask 077

# Resolve OS/tool approvals before this launcher starts. This is an operator
# handshake, not evidence of key release and not a replacement for guard ready.
# Keep the launch log OUTSIDE the closed formal/calibration evidence directory.
die() { echo "start-input-guard: $*" >&2; exit 1; }
test "$#" -eq 3 || die 'usage: start-input-guard.sh GUARD_BINARY EXPECTED_SHA256 NEW_LAUNCH_LOG'
GUARD_BINARY=$1
EXPECTED_SHA=$2
LAUNCH_LOG=$3
case "$GUARD_BINARY" in /*) ;; *) die 'guard path must be absolute' ;; esac
case "$LAUNCH_LOG" in /*) ;; *) die 'launch log path must be absolute' ;; esac
case "$EXPECTED_SHA" in ''|*[!0-9a-f]*) die 'expected digest must be lowercase SHA-256' ;; esac
test "${#EXPECTED_SHA}" -eq 64 || die 'expected digest must be lowercase SHA-256'

check_binary() {
  test -f "$GUARD_BINARY" && test ! -L "$GUARD_BINARY" && test -x "$GUARD_BINARY" ||
    die 'guard must be an executable regular file, not a symbolic link'
  local digest
  digest=$(shasum -a 256 "$GUARD_BINARY") || die 'cannot hash guard'
  test "${digest%% *}" = "$EXPECTED_SHA" || die 'guard digest mismatch'
}
check_binary
test ! -e "$LAUNCH_LOG" && test ! -L "$LAUNCH_LOG" || die 'launch log already exists'
set -C
exec 3> "$LAUNCH_LOG"
printf '{"launcher_version":1,"phase":"awaiting_start","process_id":%s,"guard_sha256":"%s"}\n' \
  "$$" "$EXPECTED_SHA" >&3

# Consume exactly the five ASCII letters and newline, without reading ahead.
# NUL is a delimiter so it is rejected rather than silently skipped by read.
START_COMMAND=''
IFS= read -r -d '' -n 6 START_COMMAND || die 'EOF before start; guard was not launched'
test "$START_COMMAND" = $'start\n' || die 'expected exactly start followed by newline; guard was not launched'
printf '{"launcher_version":1,"phase":"start_received","startup_delay_seconds":3}\n' >&3
sleep 3
check_binary
printf '{"launcher_version":1,"phase":"launching_guard","process_id":%s}\n' "$$" >&3
exec 3>&-

# stdout and stderr belong only to the guard from this point. No restart or
# held-state override: the unmodified guard decides ready / KEY_HELD / etc.
exec "$GUARD_BINARY"
