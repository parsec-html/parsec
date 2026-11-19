#!/bin/sh
# Copies the crates Parsec shares with Netweir from a Netweir checkout:
# the parser core, crates/netweir-html into crates/parsec, and XPath,
# crates/netweir-xpath into crates/parsec-xpath. Netweir is the source of
# both: a change is made there first, then synced here. Everything else in
# this repo (bindings, the C API, other modules, benchmarks) is Parsec's
# own, and so are both crates' manifests.
#
#   scripts/sync-from-netweir.sh ../netweir          copy, and record the commit
#   scripts/sync-from-netweir.sh --check ../netweir  fail if either crate differs
#
# The check runs in CI against the commit in NETWEIR_COMMIT.
set -eu
check=
if [ "${1:-}" = "--check" ]; then
  check=1
  shift
fi
netweir=${1:?usage: scripts/sync-from-netweir.sh [--check] NETWEIR_CHECKOUT}
for crate in netweir-html netweir-xpath; do
  [ -d "$netweir/crates/$crate/src" ] || { echo "no crates/$crate in $netweir" >&2; exit 1; }
done
here=$(cd "$(dirname "$0")/.." && pwd)

# Every Rust file carries Parsec's licence and copyright in place of any
# header Netweir's copy has (Netweir is AGPL), so the drift check compares
# like with like. scripts/check-headers.sh checks the result.
header='// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway'

names() {
  sed -e 's/netweir_xpath/parsec_xpath/g' -e 's/netweir-xpath/parsec-xpath/g' \
    -e 's/netweir_html/parsec/g' -e 's/netweir-html/parsec-html/g'
}

rename() {
  case "$1" in
    *.rs)
      printf '%s\n\n' "$header"
      # Leading SPDX and copyright lines, and the blank line after them, go.
      awk 'BEGIN { top = 1 }
           top && /^\/\/ (SPDX-License-Identifier|Copyright)/ { dropped = 1; next }
           top && dropped && /^$/ { top = 0; next }
           { top = 0; print }' "$1" | names |
        # A rename can shorten a line enough for rustfmt to join it, so
        # the copy is formatted again as cargo fmt would leave it.
        rustfmt --edition 2024 --emit stdout
      ;;
    *) names < "$1" ;;
  esac
}

out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT
status=0

# sync FROM TO ITEMS: copies ITEMS (paths in crates/FROM) to crates/TO, or
# with --check compares them.
sync() {
  from="$netweir/crates/$1"
  to="$here/crates/$2"
  stage="$out/$2"
  for item in $3; do
    if [ -d "$from/$item" ]; then
      (cd "$from" && find "$item" -type f) | while read -r file; do
        mkdir -p "$stage/$(dirname "$file")"
        rename "$from/$file" > "$stage/$file"
      done
    else
      mkdir -p "$stage/$(dirname "$item")"
      rename "$from/$item" > "$stage/$item"
    fi
  done
  if [ -n "$check" ]; then
    for item in $3; do
      if ! diff -r "$stage/$item" "$to/$item"; then
        echo "crates/$2 has drifted from Netweir's crates/$1 at $(cat "$here/NETWEIR_COMMIT")." >&2
        status=1
      fi
    done
    return
  fi
  for item in $3; do
    rm -rf "${to:?}/$item"
    mkdir -p "$(dirname "$to/$item")"
    cp -R "$stage/$item" "$to/$item"
  done
}

# The core: source, tests, examples, scripts and fuzz target. Every test
# and its data comes along except the html5lib-tests submodule and the
# pinned tree-construction files, which are Parsec's own, so a new test
# file in Netweir doesn't need adding here by hand.
tests=
for t in "$netweir"/crates/netweir-html/tests/*; do
  t=$(basename "$t")
  case "$t" in
    html5lib-tests | tree-construction) ;;
    *) tests="$tests tests/$t" ;;
  esac
done
sync netweir-html parsec "src examples scripts fuzz/fuzz_targets $tests"

# XPath: its source and tests, the vendored libxml2 cases included.
sync netweir-xpath parsec-xpath "src tests"

if [ -n "$check" ]; then
  if [ $status -ne 0 ]; then
    echo "Make the change in Netweir first, then run scripts/sync-from-netweir.sh." >&2
  fi
  exit $status
fi
git -C "$netweir" rev-parse HEAD > "$here/NETWEIR_COMMIT"
echo "synced crates/parsec and crates/parsec-xpath from Netweir $(cat "$here/NETWEIR_COMMIT")"
