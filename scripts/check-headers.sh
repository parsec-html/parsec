#!/bin/sh
# Fails if a tracked Rust file doesn't start with Parsec's licence and
# copyright header. scripts/sync-from-netweir.sh writes it on the synced
# files; Parsec's own files carry it by hand.
set -eu
cd "$(dirname "$0")/.."
want='// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway'
status=0
for file in $(git ls-files '*.rs'); do
  if [ "$(head -n 2 "$file")" != "$want" ]; then
    echo "$file: doesn't start with the SPDX and copyright header" >&2
    status=1
  fi
done
exit $status
