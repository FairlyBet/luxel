// Program compile/link error reporting against a real WebGL2 context.
// Run with: wasm-pack test --headless --chrome packages/rswebgl
#![cfg(target_arch = "wasm32")]

use crate::common;

use wasm_bindgen_test::*;

use rswebgl::extension::Extension;
use rswebgl::pass::Batch;
use rswebgl::render_state::RenderState;

use common::*;

const RED_FRAG: &str = r#"#version 300 es
precision highp float;
out vec4 frag_color;
void main() {
    frag_color = vec4(1.0, 0.0, 0.0, 1.0);
}
"#;

// Assigns a float to a vec4: a compile error with a recognizable log.
const BROKEN_FRAG: &str = r#"#version 300 es
precision highp float;
out vec4 frag_color;
void main() {
    frag_color = 1.0;
}
"#;

// Without parallel compile the error comes straight back from create_program,
// with the fragment shader's compile log in it.
#[wasm_bindgen_test]
fn create_program_returns_compile_log() {
    let f = fixture();
    let err = f
        .ctx
        .create_program(VERT, BROKEN_FRAG)
        .expect_err("broken shader must not link");
    assert!(err.contains("link failed"), "{err}");
    assert!(err.contains("fragment shader:"), "{err}");
}

// With parallel compile create_program can't know yet, so it succeeds; the
// renderer then skips the broken batch (no GL error) and still draws the rest.
#[wasm_bindgen_test]
fn renderer_skips_program_that_failed_to_link() {
    let f = fixture();
    if !f.ctx.enable_extension(Extension::KhrParallelShaderCompile) {
        return; // Nothing parallel to test on this browser.
    }
    let broken = f
        .ctx
        .create_program(VERT, BROKEN_FRAG)
        .expect("deferred: no error until the link completes");
    let good = f.ctx.create_program(VERT, RED_FRAG).expect("program");

    let mut bad_batch = Batch::new(broken, f.ctx.render_state(&RenderState::new()));
    draw_quad(&mut bad_batch, &f, 0);
    let mut good_batch = Batch::new(good, f.ctx.render_state(&RenderState::new()));
    draw_quad(&mut good_batch, &f, 1);
    render(&f, vec![bad_batch, good_batch]);

    assert_eq!(read_quads(&f.gl), [BLACK, RED, BLACK]);
    assert!(f.ctx.program_is_ready(broken));
    assert!(!f.ctx.program_is_valid(broken));
    assert!(f.ctx.program_is_valid(good));
}

// Errors cross into JS as `Error` objects (message + stack), not bare strings.
#[wasm_bindgen_test]
fn errors_reach_js_as_error_objects() {
    let f = fixture();
    let err = f.ctx.create_program("not glsl", "not glsl").unwrap_err();
    assert!(!err.is_empty());
    let js: wasm_bindgen::JsValue = err.into();
    assert!(wasm_bindgen::JsCast::is_instance_of::<js_sys::Error>(&js));
}

// A shader error says so (kind and JS name), as does a lost context.
#[wasm_bindgen_test]
fn errors_say_their_kind() {
    use rswebgl::error::ErrorKind;
    let f = fixture();
    let err = f.ctx.create_program(VERT, BROKEN_FRAG).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Shader);
    let js: wasm_bindgen::JsValue = err.into();
    let name = js_sys::Reflect::get(&js, &"name".into()).unwrap();
    assert_eq!(name.as_string().as_deref(), Some("ShaderError"));

    assert!(f.ctx.simulate_context_loss());
    let lost = f.ctx.create_program(VERT, RED_FRAG).unwrap_err();
    assert_eq!(lost.kind(), ErrorKind::ContextLost, "{lost}");
}

// With parallel compile: programError has the log once the link is done,
// and programsReady rejects with a ShaderError.
#[wasm_bindgen_test]
async fn a_failed_parallel_link_is_reported() {
    let f = fixture();
    if !f.ctx.enable_extension(Extension::KhrParallelShaderCompile) {
        return;
    }
    let broken = f.ctx.create_program(VERT, BROKEN_FRAG).expect("deferred");
    let good = f.ctx.create_program(VERT, RED_FRAG).expect("program");
    let result = wasm_bindgen_futures::JsFuture::from(
        f.ctx.programs_ready_js(&[broken.to_raw(), good.to_raw()]),
    )
    .await;
    let err = result.expect_err("rejects");
    let name = js_sys::Reflect::get(&err, &"name".into()).unwrap();
    assert_eq!(name.as_string().as_deref(), Some("ShaderError"));
    let log = f.ctx.program_error(broken).expect("a log");
    assert!(log.contains("fragment shader:"), "{log}");
    assert_eq!(f.ctx.program_error(good), None);
    // Only good ones: resolves.
    wasm_bindgen_futures::JsFuture::from(f.ctx.programs_ready_js(&[good.to_raw()]))
        .await
        .expect("resolves");
}
