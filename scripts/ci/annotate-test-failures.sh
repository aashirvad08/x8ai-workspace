#!/usr/bin/env bash
# Turns failing Rust tests in a `cargo test` log into GitHub annotations, so the
# failing test and its assertion show on the run's summary page without opening
# the log. Does nothing if the log is missing (an earlier step failed).
set -euo pipefail

log="${1:?usage: annotate-test-failures.sh <cargo test log>}"
[ -f "$log" ] || exit 0

# "thread 'name' (id) panicked at file:line:col:" is followed by the message.
awk '
  /^thread .* panicked at / { where = $0; getline message; printf "::error::%s %s\n", where, message }
  /^test result: FAILED/    { printf "::error::%s\n", $0 }
' "$log"
