#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# Packages every crate Parsec publishes twice, from two fresh checkouts of
# one commit, and fails unless each pair of .crate files is the same byte
# for byte. It's how a release is built (release.yml), and how anyone can
# check one: run it at the release's tag and compare the printed SHA-256s
# with the release's SHA256SUMS.
#
#   scripts/package-twice.sh [COMMIT] [OUT]
#
# COMMIT defaults to HEAD and must be committed: cargo records the commit
# and whether the tree was dirty in the package. Each build gets a worktree
# of its own, which works in a shallow clone too, and no submodules; the
# first checks out with umask 022 and the second with 077, so file modes
# from a checkout can't leak into the package. OUT, when given, gets the
# .crate files.
set -euo pipefail

commit=$(git rev-parse --verify "${1:-HEAD}^{commit}")
out=${2:-}
repo=$(git rev-parse --show-toplevel)
work=$(mktemp -d)
trap 'git -C "$repo" worktree remove --force "$work/a" 2>/dev/null; git -C "$repo" worktree remove --force "$work/b" 2>/dev/null; rm -rf "$work"' EXIT

# The commit's time, so anything that reads it gets the same value twice.
SOURCE_DATE_EPOCH=$(git log -1 --format=%ct "$commit")
export SOURCE_DATE_EPOCH

build() { # name umask
	(
		umask "$2"
		git -C "$repo" worktree add --quiet --detach "$work/$1" "$commit"
	)
	# The C API isn't on crates.io; its libraries go with the release.
	cargo package --quiet --manifest-path "$work/$1/Cargo.toml" --workspace \
		--exclude parsec-capi --locked --no-verify --target-dir "$work/target-$1"
}

build a 022
build b 077
(cd "$work/target-a/package" && sha256sum -- *.crate) > "$work/sums-a"
(cd "$work/target-b/package" && sha256sum -- *.crate) > "$work/sums-b"
if ! diff "$work/sums-a" "$work/sums-b" >&2; then
	echo "the packages differ between two builds of $commit" >&2
	exit 1
fi
if [ -n "$out" ]; then
	mkdir -p "$out"
	cp "$work/target-a/package/"*.crate "$out/"
fi
cat "$work/sums-a"
