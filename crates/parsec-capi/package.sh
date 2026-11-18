#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# Builds the C libraries in release mode and lays them out as a package:
#
#   OUT/include/parsec.h
#   OUT/lib/libparsec.a, and libparsec.so or libparsec.dylib
#      (Windows: OUT/lib/parsec.lib and parsec.dll.lib, OUT/bin/parsec.dll)
#   OUT/lib/pkgconfig/parsec.pc
#   OUT/lib/cmake/parsec/parsec-config.cmake, parsec-config-version.cmake
#   OUT/LICENSE, OUT/NOTICE
#
#   crates/parsec-capi/package.sh OUT [TARGET]
#
# TARGET is a Rust target triple, the host's by default, or
# universal-apple-darwin for one macOS package holding both arm64 and
# x86_64. It's what the release workflow ships, and what you can install
# from a checkout: point CMAKE_PREFIX_PATH or PKG_CONFIG_PATH at OUT.
set -euo pipefail

out=${1:?usage: package.sh OUT [TARGET]}
target=${2:-$(rustc -vV | sed -n 's/^host: //p')}
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -n 1)
target_dir=${CARGO_TARGET_DIR:-$root/target}

# Builds one triple and prints the system libraries a static link needs,
# which rustc only says while it compiles: hence the clean first.
build() {
  local log
  log=$(mktemp)
  cargo clean --quiet --manifest-path "$root/Cargo.toml" -p parsec-capi --release --target "$1"
  # --color never: with CARGO_TERM_COLOR=always, as CI sets it, the note
  # ends in an escape code that would stick to the last library's name.
  if ! cargo rustc --manifest-path "$root/Cargo.toml" -p parsec-capi --lib --release \
    --locked --color never --target "$1" -- --print native-static-libs > /dev/null 2> "$log"; then
    cat "$log" >&2
    exit 1
  fi
  sed -n 's/.*native-static-libs: //p' "$log" | tail -n 1
  rm -f "$log"
}

mkdir -p "$out/include" "$out/lib/pkgconfig" "$out/lib/cmake/parsec"
cp "$here/include/parsec.h" "$out/include/"
cp "$root/LICENSE" "$root/NOTICE" "$out/"

case "$target" in
  universal-apple-darwin)
    native=$(build aarch64-apple-darwin)
    build x86_64-apple-darwin >/dev/null
    for f in libparsec.a libparsec.dylib; do
      lipo -create -output "$out/lib/$f" \
        "$target_dir/aarch64-apple-darwin/release/$f" \
        "$target_dir/x86_64-apple-darwin/release/$f"
    done
    ;;
  *-windows-msvc)
    native=$(build "$target")
    mkdir -p "$out/bin"
    cp "$target_dir/$target/release/parsec.dll" "$out/bin/"
    cp "$target_dir/$target/release/parsec.dll.lib" "$target_dir/$target/release/parsec.lib" "$out/lib/"
    ;;
  *-apple-darwin)
    native=$(build "$target")
    cp "$target_dir/$target/release/libparsec.a" "$target_dir/$target/release/libparsec.dylib" "$out/lib/"
    ;;
  *)
    native=$(build "$target")
    cp "$target_dir/$target/release/libparsec.a" "$target_dir/$target/release/libparsec.so" "$out/lib/"
    ;;
esac
[ -n "$native" ] || { echo "rustc didn't print the native static libraries" >&2; exit 1; }

# The same list for CMake: -lfoo becomes foo, "-framework Foo" stays one
# item, and MSVC's /defaultlib (the C runtime CMake already picks) goes.
cmake_libs=
# shellcheck disable=SC2086 # split into words on purpose
set -- $native
while [ $# -gt 0 ]; do
  case "$1" in
    -framework) item="-framework $2"; shift ;;
    -l*) item=${1#-l} ;;
    /defaultlib:*) item= ;;
    *) item=$1 ;;
  esac
  [ -n "$item" ] && cmake_libs=${cmake_libs:+$cmake_libs;}$item
  shift
done

major=${version%%.*}
minor=${version#*.}
minor=${minor%%.*}
fill() {
  sed -e "s|@VERSION@|$version|g" -e "s|@MAJOR@|$major|g" -e "s|@MINOR@|$minor|g" \
    -e "s|@PC_PRIVATE@|$native|g" -e "s|@CMAKE_PRIVATE@|$cmake_libs|g" "$1" > "$2"
}
fill "$here/cmake/parsec.pc.in" "$out/lib/pkgconfig/parsec.pc"
fill "$here/cmake/parsec-config.cmake.in" "$out/lib/cmake/parsec/parsec-config.cmake"
fill "$here/cmake/parsec-config-version.cmake.in" "$out/lib/cmake/parsec/parsec-config-version.cmake"
echo "parsec $version for $target in $out"
