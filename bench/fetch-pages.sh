#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 James Conway
#
# Downloads the benchmark's real pages into bench/corpus/ and checks each
# against the SHA-256 in bench/pages.txt. The pages are someone else's, so
# the repository holds only their addresses: each one is a fixed snapshot
# in the Internet Archive, fetched raw (the `id_` form), so every run of
# this script gets the same bytes.
#
#   bench/fetch-pages.sh           # fetch what's missing, check everything
#   bench/fetch-pages.sh --print   # fetch, then print pages.txt lines with
#                                  # the hashes found (for adding a page)
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$here/corpus"
status=0

while read -r name stamp url sum; do
    [[ -z "$name" || "$name" == \#* ]] && continue
    file="$here/corpus/$name.html"
    if [[ ! -s "$file" ]]; then
        curl -fsSL --compressed --retry 3 --max-time 300 -o "$file.part" \
            "https://web.archive.org/web/${stamp}id_/$url"
        # Some snapshots were stored still gzipped, without the header that
        # says so.
        if [[ "$(head -c 2 "$file.part" | od -An -tx1 | tr -d ' ')" == 1f8b ]]; then
            gzip -dc < "$file.part" > "$file"
            rm "$file.part"
        else
            mv "$file.part" "$file"
        fi
    fi
    got=$(shasum -a 256 "$file" | cut -c1-64)
    if [[ "${1:-}" == --print ]]; then
        echo "$name $stamp $url $got"
    elif [[ "$got" != "$sum" ]]; then
        echo "$name: SHA-256 $got, expected $sum" >&2
        status=1
    fi
done < "$here/pages.txt"

exit $status
