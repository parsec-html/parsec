#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 James Conway
#
# Downloads the W3C XML Conformance Test Suite (xmlconf), release
# 20130923, the last one, into crates/parsec-xml/tests/xmlconf, and checks
# it against the SHA-256 below. The suite has no git home to pin as a
# submodule, so the tarball's address and hash are the pin. Its files are
# under the W3C Test Suite License, and aren't in the repository.
#
#   crates/parsec-xml/scripts/fetch-xmlconf.sh
set -euo pipefail

release=xmlts20130923
url="https://www.w3.org/XML/Test/$release.tar.gz"
sum=9b61db9f5dbffa545f4b8d78422167083a8568c59bd1129f94138f936cf6fc1f

crate=$(cd "$(dirname "$0")/.." && pwd)
dest="$crate/tests/xmlconf"
if [[ -f "$dest/COMMIT" && "$(cat "$dest/COMMIT")" == "$release" ]]; then
    exit 0
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
curl -fsSL --retry 3 --max-time 300 -o "$tmp/$release.tar.gz" "$url"
if command -v sha256sum > /dev/null; then
    got=$(sha256sum "$tmp/$release.tar.gz" | cut -c1-64)
else
    got=$(shasum -a 256 "$tmp/$release.tar.gz" | cut -c1-64)
fi
if [[ "$got" != "$sum" ]]; then
    echo "$url: SHA-256 $got, expected $sum" >&2
    exit 1
fi
mkdir "$tmp/out"
tar -xzf "$tmp/$release.tar.gz" -C "$tmp/out"
rm -rf "$dest"
mv "$tmp/out/xmlconf" "$dest"
echo "$release" > "$dest/COMMIT"
