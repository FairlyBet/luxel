//! Every integration test of the crate in one binary: one link and one
//! wasm file instead of one per test file, which keeps builds and `target/`
//! small. Each file is a module; browser ones are `#![cfg(wasm32)]`.

#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

mod loading;
mod resources;
