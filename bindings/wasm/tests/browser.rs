// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

// api.rs's tests in a browser: `wasm-pack test --headless --firefox --test
// browser`, as CI runs them; `--chrome` and `--safari` work too.

wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

include!("api.rs");
