#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# Compares the conformance counts a test run wrote against the last
# release's, and fails if any suite got worse: fewer cases passed or more
# failed. A release can't ship until this passes (release.yml); on a pull
# request it runs for information (ci.yml).
#
#   scripts/conformance-gate.sh BASELINE DIR
#
# BASELINE is conformance/baseline.json: an object of suite name to
# {"passed", "failed", "skipped"}. DIR holds the <suite>.json files that
# `cargo test --workspace` writes when CONFORMANCE_DIR is set. A suite in
# the baseline that wrote no counts fails too, since a suite that stopped
# running has stopped proving anything. A suite with counts but not in
# the baseline is reported and passes: it's new, and goes into the
# baseline in the next release pull request.
#
# The table on stdout is Markdown, for a run's summary.
set -euo pipefail

if [ $# -ne 2 ]; then
  echo "usage: $0 BASELINE DIR" >&2
  exit 2
fi
baseline=$1
dir=$2
# Every count must be a number, in the baseline and in each run's file:
# jq inside `< <(...)` is out of `set -e`'s reach, and `[` on an empty or
# "null" value is false rather than an error, so an unreadable file would
# otherwise pass.
numbers='[.passed, .failed, .skipped] | all(type == "number")'
jq -e "type == \"object\" and all(.[]; $numbers)" "$baseline" > /dev/null

status=0
echo "| Suite | Passed | Failed | Skipped | Against the baseline |"
echo "|---|---:|---:|---:|---|"
while IFS=$'\t' read -r suite passed failed; do
  file="$dir/$suite.json"
  if [ ! -f "$file" ]; then
    echo "| $suite | - | - | - | **wrote no counts** (baseline $passed passed) |"
    status=1
    continue
  fi
  if ! jq -e "$numbers" "$file" > /dev/null 2>&1; then
    echo "| $suite | - | - | - | **unreadable counts** |"
    status=1
    continue
  fi
  read -r now_passed now_failed now_skipped < <(jq -r '"\(.passed) \(.failed) \(.skipped)"' "$file")
  verdict=same
  if [ "$now_passed" -lt "$passed" ] || [ "$now_failed" -gt "$failed" ]; then
    verdict="**worse**: baseline $passed passed, $failed failed"
    status=1
  elif [ "$now_passed" -gt "$passed" ] || [ "$now_failed" -lt "$failed" ]; then
    verdict="better: baseline $passed passed, $failed failed"
  fi
  echo "| $suite | $now_passed | $now_failed | $now_skipped | $verdict |"
done < <(jq -r 'to_entries[] | "\(.key)\t\(.value.passed)\t\(.value.failed)"' "$baseline")

for file in "$dir"/*.json; do
  [ -f "$file" ] || continue
  # Unreadable files were reported above if the baseline has their suite;
  # one it doesn't have fails here, since we can't tell what it is.
  if ! suite=$(jq -er .suite "$file" 2> /dev/null); then
    if ! jq -e --arg s "$(basename "$file" .json)" 'has($s)' "$baseline" > /dev/null; then
      echo "| $(basename "$file" .json) | - | - | - | **unreadable counts** |"
      status=1
    fi
    continue
  fi
  if ! jq -e --arg s "$suite" 'has($s)' "$baseline" > /dev/null; then
    jq -r '"| \(.suite) | \(.passed) | \(.failed) | \(.skipped) | new: not in the baseline yet |"' "$file"
  fi
done

if [ $status -ne 0 ]; then
  echo "A conformance suite is worse than $baseline; see the table above" >&2
fi
exit $status
