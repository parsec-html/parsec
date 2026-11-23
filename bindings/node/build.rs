// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

fn main() {
    // The linker flags an addon needs: on macOS, leave Node's symbols for
    // Node to supply when it loads the library.
    napi_build::setup();
}
