#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# Tests scripts/conformance-gate.sh: each case writes a baseline and a
# run's counts, and checks the gate's exit status. CI's lint job runs it.
set -euo pipefail
gate="$(cd "$(dirname "$0")" && pwd)/conformance-gate.sh"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

counts() { # DIR SUITE PASSED FAILED SKIPPED
  mkdir -p "$1"
  printf '{"suite": "%s", "passed": %s, "failed": %s, "skipped": %s, "skipped_because": "", "test_data_commit": "x"}\n' \
    "$2" "$3" "$4" "$5" > "$1/$2.json"
}
printf '{"a": {"passed": 10, "failed": 2, "skipped": 1}, "b": {"passed": 5, "failed": 0, "skipped": 0}}\n' > "$tmp/baseline.json"

failures=0
expect() { # STATUS NAME DIR
  set +e
  "$gate" "$tmp/baseline.json" "$3" > "$tmp/out" 2>&1
  got=$?
  set -e
  if [ "$got" -ne "$1" ]; then
    echo "FAIL: $2: exit $got, wanted $1" >&2
    cat "$tmp/out" >&2
    failures=$((failures + 1))
  else
    echo "ok: $2"
  fi
}

counts "$tmp/same" a 10 2 1
counts "$tmp/same" b 5 0 0
expect 0 "the same counts pass" "$tmp/same"

counts "$tmp/better" a 11 1 1
counts "$tmp/better" b 5 0 0
expect 0 "more passed and fewer failed pass" "$tmp/better"

counts "$tmp/more-skipped" a 10 2 4
counts "$tmp/more-skipped" b 5 0 0
expect 0 "a skip that costs no passes passes" "$tmp/more-skipped"

counts "$tmp/dropped" a 9 2 2
counts "$tmp/dropped" b 5 0 0
expect 1 "one case fewer passed fails" "$tmp/dropped"

counts "$tmp/failing" a 10 3 0
counts "$tmp/failing" b 5 0 0
expect 1 "one case more failed fails" "$tmp/failing"

counts "$tmp/missing" a 10 2 1
expect 1 "a suite that wrote no counts fails" "$tmp/missing"

counts "$tmp/new" a 10 2 1
counts "$tmp/new" b 5 0 0
counts "$tmp/new" c 1 7 0
expect 0 "a suite not in the baseline yet passes" "$tmp/new"
grep -q '| c | 1 | 7 | 0 | new' "$tmp/out" || { echo "FAIL: the new suite isn't in the table" >&2; failures=$((failures + 1)); }

counts "$tmp/no-passed" b 5 0 0
printf '{"suite": "a", "failed": 2, "skipped": 1}\n' > "$tmp/no-passed/a.json"
expect 1 "a counts file with no passed fails" "$tmp/no-passed"

counts "$tmp/not-json" b 5 0 0
printf 'not json\n' > "$tmp/not-json/a.json"
expect 1 "a counts file that isn't JSON fails" "$tmp/not-json"

[ "$failures" -eq 0 ]
