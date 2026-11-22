#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 James Conway
#
# Downloads the XML benchmark's documents in bench/xml.txt into
# bench/corpus/xml/ and checks each against the SHA-256 in the list. The
# documents are someone else's, so the repository holds only their
# addresses, each pinned: an Internet Archive snapshot, a versioned npm
# file, or a dated Wikipedia dump. xml.txt says how each kind is read.
#
#   bench/fetch-xml.sh           # fetch what's missing, check everything
#   bench/fetch-xml.sh --print   # fetch, then print xml.txt lines with the
#                                # hashes found (for adding a document)
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
out="$here/corpus/xml"
mkdir -p "$out"
status=0

sha() {
    if command -v sha256sum > /dev/null; then
        sha256sum "$1" | cut -c1-64
    else
        shasum -a 256 "$1" | cut -c1-64
    fi
}

# Gzipped bytes, with or without a header that says so, come out plain.
ungzip() {
    if [[ "$(head -c 2 "$1" | od -An -tx1 | tr -d ' ')" == 1f8b ]]; then
        gzip -dc < "$1" > "$1.plain"
        mv "$1.plain" "$1"
    fi
}

while read -r name source url sum; do
    [[ -z "$name" || "$name" == \#* ]] && continue
    case "$source" in
    npm)
        file="$out/$name.tgz"
        [[ -s "$file" ]] || curl -fsSL --retry 3 --max-time 300 -o "$file" "$url"
        got=$(sha "$file")
        rm -rf "$out/$name"
        mkdir -p "$out/$name"
        tar -xzf "$file" -C "$out/$name" --strip-components 2 package/svg
        ;;
    bz2-slice:*)
        file="$out/$name.xml"
        if [[ ! -s "$file" ]]; then
            bytes=${source#bz2-slice:}
            curl -fsSL --retry 3 --max-time 600 -r "0-$((bytes - 1))" -o "$file.bz2" "$url"
            # The cut leaves the last block short, so bzip2 stops there.
            bzip2 -dc < "$file.bz2" > "$file.part" 2> /dev/null || true
            last=$(grep -bo '</page>' "$file.part" | tail -n 1 | cut -d: -f1)
            head -c "$((last + 7))" "$file.part" > "$file"
            printf '\n</mediawiki>\n' >> "$file"
            rm "$file.part" "$file.bz2"
        fi
        got=$(sha "$file")
        ;;
    *)
        file="$out/$name.xml"
        if [[ ! -s "$file" ]]; then
            if [[ "$source" == - ]]; then
                curl -fsSL --retry 3 --max-time 300 -o "$file.part" "$url"
            else
                curl -fsSL --retry 3 --max-time 300 -o "$file.part" \
                    "https://web.archive.org/web/${source}id_/$url"
            fi
            ungzip "$file.part"
            mv "$file.part" "$file"
        fi
        got=$(sha "$file")
        ;;
    esac
    if [[ "${1:-}" == --print ]]; then
        echo "$name $source $url $got"
    elif [[ "$got" != "$sum" ]]; then
        echo "$name: SHA-256 $got, expected $sum" >&2
        status=1
    fi
done < "$here/xml.txt"

exit $status
