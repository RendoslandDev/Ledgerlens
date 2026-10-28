#!/usr/bin/env bash
# Runs a command and, if it fails, publishes the last 120 lines of its output as an error
# annotation, so failures are readable from the run summary and the API without opening logs.
# Usage: .github/scripts/annotate.sh "<title>" <command> [args...]
set -uo pipefail
title="$1"
shift
log="$(mktemp)"
"$@" 2>&1 | tee "$log"
status=${PIPESTATUS[0]}
if [ "$status" -ne 0 ]; then
  # Annotations are one line: escape %, CR, and LF as the workflow-command format requires.
  # Drop colour codes and cargo's per-crate progress lines, which otherwise crowd out the errors.
  esc="$(printf '\033')"
  body="$(sed -e "s/${esc}\[[0-9;]*m//g" "$log" |
    grep -vE '^ *(Checking|Compiling|Downloaded|Downloading|Fresh|Blocking|Updating|Locking|Adding) ' |
    tail -n 150 | sed -e 's/%/%25/g' -e 's/\r/%0D/g' | awk '{ printf "%s%%0A", $0 }')"
  echo "::error title=${title} failed (exit ${status})::${body}"
fi
rm -f "$log"
exit "$status"
