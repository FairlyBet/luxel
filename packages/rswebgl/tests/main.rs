//! Every integration test of the crate in one binary: one link and one
//! wasm file instead of one per test file, which keeps builds and `target/`
//! small. Each file is a module; browser ones are `#![cfg(wasm32)]`.

#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

#[cfg(target_arch = "wasm32")]
mod common;
#[cfg(target_arch = "wasm32")]
mod compress_util;

mod auto_resize;
mod bindings;
mod compress;
mod compress_etc;
mod compress_more;
mod context_loss;
mod copy;
mod draw_buffers;
mod draw_list;
mod feedback;
mod float_targets;
mod images;
mod limits;
mod params;
mod programs;
mod queries;
mod readback;
mod resources;
mod reverse_z;
mod samplers;
mod trace;
