#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 James Conway
#
# Fetches web-platform-tests' media query, computed-value and cascade
# tests at a pinned commit and turns their static cases into
# tests/wpt-media, tests/wpt-computed and tests/wpt-cascade, with
# scripts/wpt-media-cases.js, wpt-computed-cases.js and
# wpt-cascade-cases.js (needs node and git). Run from crates/parsec-css,
# then UPDATE_KNOWN_FAILURES=1 cargo test for each of wpt_media,
# wpt_computed and wpt_cascade:
#
#   scripts/fetch-wpt-cascade.sh COMMIT
set -eu
commit=${1:?usage: scripts/fetch-wpt-cascade.sh WPT_COMMIT}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

git -C "$tmp" init -q wpt
cd "$tmp/wpt"
git remote add origin https://github.com/web-platform-tests/wpt.git
git sparse-checkout set --no-cone '/css/*/parsing/' '/css/*/inheritance*.html' \
  '/css/mediaqueries/' '/css/css-cascade/' '/css/support/' '/LICENSE.md'
git fetch -q --depth 1 --filter=blob:none origin "$commit"
git checkout -q FETCH_HEAD
git rev-parse HEAD > COMMIT
cd - > /dev/null

for suite in media computed cascade; do
  dir=tests/wpt-$suite
  # known-failures.txt stays: the test rewrites it when asked.
  find "$dir" -mindepth 1 ! -name known-failures.txt -delete 2>/dev/null || true
  mkdir -p "$dir"
  node "scripts/wpt-$suite-cases.js" "$tmp/wpt" "$dir"
  cp "$tmp/wpt/LICENSE.md" "$tmp/wpt/COMMIT" "$dir/"
done
