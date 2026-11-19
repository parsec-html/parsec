#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 James Conway
#
# Fetches web-platform-tests' CSS parsing tests (css/*/parsing) at a
# pinned commit and turns their static cases into
# tests/wpt-values/cases.json with scripts/wpt-values-cases.js (needs
# node and git). Run from crates/parsec-css to update them, then
# UPDATE_KNOWN_FAILURES=1 cargo test --test wpt_values:
#
#   scripts/fetch-wpt-values.sh COMMIT
set -eu
commit=${1:?usage: scripts/fetch-wpt-values.sh WPT_COMMIT}
dir=tests/wpt-values
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

git -C "$tmp" init -q wpt
cd "$tmp/wpt"
git remote add origin https://github.com/web-platform-tests/wpt.git
git sparse-checkout set --no-cone '/css/*/parsing/' '/css/support/' '/LICENSE.md'
git fetch -q --depth 1 --filter=blob:none origin "$commit"
git checkout -q FETCH_HEAD
git rev-parse HEAD > COMMIT
cd - > /dev/null

# known-failures.txt stays: the test rewrites it when asked.
mkdir -p "$dir"
node scripts/wpt-values-cases.js "$tmp/wpt" "$dir"
cp "$tmp/wpt/LICENSE.md" "$tmp/wpt/COMMIT" "$dir/"
