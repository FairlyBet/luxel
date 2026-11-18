//! Every integration test of the crate in one binary: one link and one
//! wasm file instead of one per test file, which keeps builds and `target/`
//! small. Each file is a module; browser ones are `#![cfg(wasm32)]`.
//!
//! `furnace.rs` is a binary of its own: run after the rest in one page, it
//! blocks the page past the test runner's patience (seen on headless Chrome;
//! likely the shaders earlier tests left compiling in the GPU process).

#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

mod animation;
mod api;
mod culling;
mod environment;
mod exposure;
mod framing;
mod fxaa;
mod gltf_nodes;
mod hdr;
mod hierarchy;
mod materials;
mod model;
mod pbr;
mod peeling;
mod pick;
mod present;
mod render;
mod see_through;
mod shadows;
mod taa;
