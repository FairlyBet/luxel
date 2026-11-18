// Per-target blending (OES_draw_buffers_indexed): each color attachment of
// an MRT pass blends its own way, and the state cache keeps it straight
// when states switch. Run with: wasm-pack test --headless --chrome packages/luxel-gl
#![cfg(target_arch = "wasm32")]

use crate::common;

use wasm_bindgen_test::*;

use luxel_gl::framebuffer::ClearMask;
use luxel_gl::ids::{FramebufferId, ParamGroupId, StateId};
use luxel_gl::pass::Pass;
use luxel_gl::readback::ReadFormat;
use luxel_gl::render_state::{BlendFactor, RenderState, TargetBlend};
use luxel_gl::renderbuffer::RenderbufferFormat;

use common::*;

const NULL: ParamGroupId = ParamGroupId::NULL;

// Writes 0.5 gray to both outputs.
const MRT_FRAG: &str = "#version 300 es\nprecision highp float;\n\
    layout(location = 0) out vec4 a;\nlayout(location = 1) out vec4 b;\n\
    void main() { a = vec4(0.5); b = vec4(0.5); }\n";

fn mrt(f: &Fixture) -> FramebufferId {
    let fb = f.ctx.create_framebuffer(W, H).unwrap();
    for i in 0..2 {
        let rb = f
            .ctx
            .create_renderbuffer(RenderbufferFormat::Rgba8, W, H)
            .unwrap();
        f.ctx.framebuffer_attach_color_renderbuffer(fb, i, rb);
        // Both start at 0.25 gray.
        f.ctx
            .framebuffer_set_color_clear_f32(fb, i, 0.25, 0.25, 0.25, 0.25);
    }
    fb
}

// Draws the middle quad with `state` over a cleared target; reads both
// attachments' red there.
fn draw(f: &Fixture, fb: FramebufferId, state: StateId) -> [u8; 2] {
    let program = f.ctx.create_program(VERT, MRT_FRAG).unwrap();
    let mut pass = Pass::new(fb);
    pass.set_clear(&ClearMask::color());
    pass.push(
        program,
        state,
        f.quad,
        NULL,
        NULL,
        NULL,
        quad(f, 1),
        &quad_cmd(),
    );
    f.ctx.render(&pass);
    let read = |i| {
        f.ctx
            .read_pixels(fb, i, W / 2, H / 2, 1, 1, ReadFormat::Rgba8)
            .unwrap()[0]
    };
    [read(0), read(1)]
}

// Target 0 adds, target 1 replaces.
fn adding_on_zero(f: &Fixture) -> StateId {
    let mut add = TargetBlend::new();
    add.blend = true;
    add.src_rgb = BlendFactor::One;
    add.dst_rgb = BlendFactor::One;
    let mut rs = RenderState::new();
    rs.set_target_blend(0, &add);
    f.ctx.render_state(&rs)
}

#[wasm_bindgen_test]
fn each_target_blends_its_own_way() {
    let f = fixture();
    let fb = mrt(&f);
    let plain = f.ctx.render_state(&RenderState::new());
    // 0.25 + 0.5 = 0.75 → 191; replaced: 0.5 → 128.
    if !f.ctx.can_blend_per_target() {
        // Without the extension the override is dropped (reported): both
        // targets follow the state's own (non-)blending.
        let errors = capture_console("error", || {
            assert_near(draw(&f, fb, adding_on_zero(&f)), [128, 128]);
        });
        assert!(
            errors
                .iter()
                .any(|e| e.contains("OES_draw_buffers_indexed"))
        );
        return;
    }
    assert_near(draw(&f, fb, plain), [128, 128]);
    let per_target = adding_on_zero(&f);
    assert_near(draw(&f, fb, per_target), [191, 128]);
    // Switching back to a state without overrides undoes them (the diff
    // can't leave target 0 blending)…
    assert_near(draw(&f, fb, plain), [128, 128]);
    // …and a global blend still reaches every target.
    let mut all = RenderState::new();
    all.blend = true;
    all.blend_src_rgb = BlendFactor::One;
    all.blend_dst_rgb = BlendFactor::One;
    let all = f.ctx.render_state(&all);
    assert_near(draw(&f, fb, all), [191, 191]);
    assert_near(draw(&f, fb, per_target), [191, 128]);
    assert_eq!(f.gl.get_error(), web_sys::WebGl2RenderingContext::NO_ERROR);
}

#[wasm_bindgen_test]
fn overrides_past_the_draw_buffers_limit_are_dropped() {
    let f = fixture();
    if !f.ctx.can_blend_per_target() {
        return;
    }
    let mut rs = RenderState::new();
    rs.set_target_blend(1000, &TargetBlend::new());
    let errors = capture_console("error", || {
        let id = f.ctx.render_state(&rs);
        assert_eq!(
            id,
            f.ctx.render_state(&RenderState::new()),
            "same as no overrides"
        );
    });
    assert!(
        errors.iter().any(|e| e.contains("MAX_DRAW_BUFFERS")),
        "{errors:?}"
    );
}

// GPUs round 0.5 to 127 or 128.
#[track_caller]
fn assert_near(got: [u8; 2], want: [u8; 2]) {
    assert!(
        got.iter()
            .zip(want)
            .all(|(g, w)| (*g as i32 - w as i32).abs() <= 1),
        "{got:?} vs {want:?}"
    );
}

// Red into 0, green into 1.
const TWO_COLORS_FRAG: &str = "#version 300 es\nprecision highp float;\n\
    layout(location = 0) out vec4 a;\nlayout(location = 1) out vec4 b;\n\
    void main() { a = vec4(1.0, 0.0, 0.0, 1.0); b = vec4(0.0, 1.0, 0.0, 1.0); }\n";

// A multisampled MRT pass resolves each listed attachment into its own:
// red into 0, green into 1 (before: color 0 into both).
#[wasm_bindgen_test]
fn a_multisampled_mrt_pass_resolves_each_attachment() {
    let f = fixture();
    let ms = f.ctx.create_framebuffer(W, H).unwrap();
    let out = f.ctx.create_framebuffer(W, H).unwrap();
    for i in 0..2 {
        let rb = f
            .ctx
            .create_renderbuffer_multisample(RenderbufferFormat::Rgba8, 4, W, H)
            .unwrap();
        f.ctx.framebuffer_attach_color_renderbuffer(ms, i, rb);
        let plain = f
            .ctx
            .create_renderbuffer(RenderbufferFormat::Rgba8, W, H)
            .unwrap();
        f.ctx.framebuffer_attach_color_renderbuffer(out, i, plain);
    }
    let program = f.ctx.create_program(VERT, TWO_COLORS_FRAG).unwrap();
    let state = f.ctx.render_state(&RenderState::new());
    let mut pass = Pass::new(ms);
    pass.set_clear(&ClearMask::color());
    pass.set_resolve(out);
    pass.set_resolve_attachments(&[0, 1]);
    pass.push(
        program,
        state,
        f.quad,
        NULL,
        NULL,
        NULL,
        quad(&f, 1),
        &quad_cmd(),
    );
    f.ctx.render(&pass);
    let read = |i| {
        f.ctx
            .read_pixels(out, i, W / 2, H / 2, 1, 1, ReadFormat::Rgba8)
            .unwrap()
    };
    assert_eq!(read(0)[..3], [255, 0, 0]);
    assert_eq!(read(1)[..3], [0, 255, 0]);
    // The resolve target's draw buffers were put back: a clear (which
    // writes only the enabled ones) reaches both.
    for i in 0..2 {
        f.ctx
            .framebuffer_set_color_clear_f32(out, i, 0.0, 0.0, 1.0, 1.0);
    }
    let mut again = Pass::new(out);
    again.set_clear(&ClearMask::color());
    f.ctx.render(&again);
    assert_eq!(read(0)[..3], [0, 0, 255]);
    assert_eq!(read(1)[..3], [0, 0, 255]);
}

// A resolve that can't run (an attachment the resolve target lacks; depth
// without a matching depth) is skipped without leaving the pass's target
// bound as if it had run: the next pass draws where it says.
#[wasm_bindgen_test]
fn a_skipped_resolve_leaves_the_bindings_right() {
    let f = fixture();
    let ms = f.ctx.create_framebuffer(W, H).unwrap();
    for i in 0..2 {
        let rb = f
            .ctx
            .create_renderbuffer_multisample(RenderbufferFormat::Rgba8, 4, W, H)
            .unwrap();
        f.ctx.framebuffer_attach_color_renderbuffer(ms, i, rb);
    }
    let out = f.ctx.create_framebuffer(W, H).unwrap();
    let plain = f
        .ctx
        .create_renderbuffer(RenderbufferFormat::Rgba8, W, H)
        .unwrap();
    f.ctx.framebuffer_attach_color_renderbuffer(out, 0, plain);
    let program = f.ctx.create_program(VERT, TWO_COLORS_FRAG).unwrap();
    let state = f.ctx.render_state(&RenderState::new());
    let mut pass = Pass::new(ms);
    pass.set_clear(&ClearMask::color());
    pass.set_resolve(out);
    pass.set_resolve_attachments(&[1]); // `out` has no attachment 1
    pass.set_resolve_depth(true); // and neither has depth
    pass.push(
        program,
        state,
        f.quad,
        NULL,
        NULL,
        NULL,
        quad(&f, 1),
        &quad_cmd(),
    );
    let errors = capture_console("error", || f.ctx.render(&pass));
    assert!(
        errors.iter().any(|e| e.contains("attachment 1")),
        "{errors:?}"
    );
    // The canvas next: drawn there, not into the multisampled target.
    let mut canvas = Pass::to_default();
    canvas.set_clear(&ClearMask::color());
    canvas.push(
        program,
        state,
        f.quad,
        NULL,
        NULL,
        NULL,
        quad(&f, 1),
        &quad_cmd(),
    );
    f.ctx.render(&canvas);
    assert_eq!(read_quads(&f.gl)[1], RED);
}

// A depth-only target (a shadow map) declares no color output: drawBuffers
// [NONE], not GL's default [COLOR_ATTACHMENT0] with nothing attached there.
#[wasm_bindgen_test]
fn a_depth_only_target_draws_to_no_color_buffer() {
    use web_sys::WebGl2RenderingContext as Gl;
    let f = fixture();
    let fb = f.ctx.create_framebuffer(W, H).unwrap();
    let depth = f
        .ctx
        .create_renderbuffer(RenderbufferFormat::Depth24, W, H)
        .unwrap();
    f.ctx.framebuffer_attach_depth_renderbuffer(fb, depth);
    assert!(f.ctx.framebuffer_is_complete(fb));

    let raw = f.ctx.raw_framebuffer(fb).unwrap();
    f.gl.bind_framebuffer(Gl::FRAMEBUFFER, Some(&raw));
    let draw0 = f.gl.get_parameter(Gl::DRAW_BUFFER0).unwrap();
    f.gl.bind_framebuffer(Gl::FRAMEBUFFER, None);
    f.ctx.invalidate_state();
    assert_eq!(draw0.as_f64(), Some(Gl::NONE as f64));
}
