#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# Fails if a crate Parsec publishes pulls in a Parsec crate it shouldn't.
# For each one, `cargo tree` lists every workspace crate it reaches with
# all its features on, directly or through another crate, and each must
# be in the crate's row below. parsec-base must reach nothing at all, not
# even a crate from crates.io.
#
#   scripts/check-crate-deps.sh
#
# The rows follow the wiki's Architecture page: each language keeps its own
# tree, and what they share is parsec-base. A new edge means a new row
# entry here, in a pull request that says why.
set -euo pipefail

allowed() {
  case $1 in
    parsec-base) ;;
    parsec-url) ;;
    parsec-web-core) ;;
    parsec-js) echo parsec-base ;;
    # XPath's own tree is generic; the HTML implementation is a feature.
    parsec-xpath) echo parsec-web-core ;;
    # Never the HTML tree, even through parsec-xpath.
    parsec-xml) echo parsec-base parsec-xpath ;;
    # Two edges the wiki calls out as breaking the rule: the cascade matches
    # selectors against a Document, and parse_bytes lives with the decoder.
    # Allowed until they move behind features (#246); until then, CSS syntax
    # alone still compiles the HTML tree.
    parsec-web-css) echo parsec-web-core ;;
    parsec-web-encoding) echo parsec-web-core ;;
    parsec-web) echo parsec-web-core parsec-web-css parsec-web-encoding parsec-url parsec-xpath ;;
    *) return 1 ;;
  esac
}

failed=0
published=$(cargo metadata --no-deps --format-version 1 |
  jq -r '.packages[] | select(.publish != []) | .name')
members=$(cargo metadata --no-deps --format-version 1 | jq -r '.packages[].name')

for crate in $published; do
  if ! ok=$(allowed "$crate"); then
    echo "$crate is published but has no row in $0"
    failed=1
    continue
  fi
  reached=$(cargo tree -q -p "$crate" -e normal --all-features --prefix none --format '{p}' |
    awk '{print $1}' | sort -u | grep -vx "$crate" || true)
  if [ "$crate" = parsec-base ] && [ -n "$reached" ]; then
    echo "parsec-base must depend on nothing, and reaches: ${reached//$'\n'/ }"
    failed=1
    continue
  fi
  for dep in $reached; do
    grep -qx "$dep" <<<"$members" || continue
    if [[ " $ok " != *" $dep "* ]]; then
      echo "$crate reaches $dep, which its row in $0 doesn't allow"
      failed=1
    fi
  done
done

[ "$failed" = 0 ] && echo "Each published crate reaches only the Parsec crates its row allows."
exit "$failed"
