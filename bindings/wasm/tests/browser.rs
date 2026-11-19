// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

// api.rs's tests in a browser: `wasm-pack test --headless --chrome`.

wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

include!("api.rs");
