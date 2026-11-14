// GPU texture compression (TextureCompressor): a texture compressed in place
// samples close to what it held — every level, under the same handle, with
// a texture unit the renderer had it bound on before. Skips the formats the
// device lacks. Run with: wasm-pack test --headless --chrome packages/rswebgl
#![cfg(target_arch = "wasm32")]

use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use web_sys::{HtmlCanvasElement, WebGl2RenderingContext as Gl};

use rswebgl::context::{Context, ContextOptions};
use rswebgl::draw::{DrawCommand, DrawMode, Viewport};
use rswebgl::ids::TextureId;
use rswebgl::param::ParamLayoutBuilder;
use rswebgl::pass::{Batch, Pass};
use rswebgl::render_state::RenderState;
use rswebgl::texture::{TextureFormat, TextureMagFilter, TextureMinFilter, TextureTarget};
use rswebgl::texture_compressor::{CompressKind, TextureCompressor};
use rswebgl::uniform_buffer::UniformType;

wasm_bindgen_test_configure!(run_in_browser);

const SIZE: i32 = 32;

const VERT: &str = "#version 300 es
void main() {
    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2)) * 2.0 - 1.0;
    gl_Position = vec4(p, 0.0, 1.0);
}
";

// The texture's level u_level, texel for pixel (raw values: an sRGB
// texture's are encoded back to its stored bytes).
const SHOW_FRAG: &str = "#version 300 es
precision highp float;
uniform sampler2D u_tex;
uniform int u_level;
uniform int u_srgb;
out vec4 frag_color;
void main() {
    ivec2 size = textureSize(u_tex, u_level);
    vec4 c = texelFetch(u_tex, min(ivec2(gl_FragCoord.xy), size - 1), u_level);
    if (u_srgb == 1) {
        c.rgb = mix(c.rgb * 12.92, 1.055 * pow(c.rgb, vec3(1.0 / 2.4)) - 0.055,
                    step(0.0031308, c.rgb));
    }
    frag_color = c;
}
";

struct Env {
    ctx: Context,
    gl: Gl,
}

fn env() -> Env {
    let canvas: HtmlCanvasElement = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .create_element("canvas")
        .unwrap()
        .dyn_into()
        .unwrap();
    canvas.set_width(SIZE as u32);
    canvas.set_height(SIZE as u32);
    let mut options = ContextOptions::new();
    options.antialias = false;
    let ctx = Context::from_canvas(&canvas, &options).expect("ctx");
    let gl = canvas
        .get_context("webgl2")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    Env { ctx, gl }
}

// What BC formats hold well: colors along one line (a diagonal gradient
// from one color to another), alpha a ramp of its own.
fn image() -> Vec<u8> {
    let mut px = Vec::new();
    for y in 0..SIZE {
        for x in 0..SIZE {
            let t = (x + y) as f32 / (2 * SIZE - 2) as f32;
            let r = (t * 255.0).round() as u8;
            let g = (255.0 - t * 200.0).round() as u8;
            let b = (50.0 + t * 100.0).round() as u8;
            let a = (255 - y * 4).clamp(0, 255) as u8;
            px.extend_from_slice(&[r, g, b, a]);
        }
    }
    px
}

fn texture(ctx: &Context, srgb: bool) -> TextureId {
    let format = if srgb {
        TextureFormat::srgb8_alpha8()
    } else {
        TextureFormat::rgba8()
    };
    let t = ctx
        .create_texture(
            TextureTarget::Texture2D,
            TextureMinFilter::LinearMipmapLinear,
            TextureMagFilter::Linear,
        )
        .unwrap();
    ctx.texture_storage_2d(t, 6, &format, SIZE, SIZE);
    ctx.texture_sub_image_2d(t, 0, 0, 0, SIZE, SIZE, &format, &image());
    ctx.texture_generate_mipmaps(t);
    t
}

// Level `level` of `tex`, drawn to the canvas and read back (RGBA bytes,
// the level's size).
fn show(e: &Env, tex: TextureId, level: i32, srgb: bool) -> Vec<u8> {
    let ctx = &e.ctx;
    let program = ctx.create_program(VERT, SHOW_FRAG).unwrap();
    let layout = ctx
        .create_param_layout(
            &ParamLayoutBuilder::new()
                .texture("u_tex")
                .uniform("u_level", UniformType::I32)
                .uniform("u_srgb", UniformType::I32),
        )
        .unwrap();
    let group = ctx.create_param_group(layout).unwrap();
    ctx.param_set_texture(group, ctx.param_field(layout, "u_tex").unwrap(), tex);
    ctx.param_set_i32(group, ctx.param_field(layout, "u_level").unwrap(), level);
    ctx.param_set_i32(
        group,
        ctx.param_field(layout, "u_srgb").unwrap(),
        srgb as i32,
    );
    let mut batch = Batch::new(program, ctx.render_state(&RenderState::new()));
    batch.set_group(0, group);
    batch.draw_vertexless(&DrawCommand::arrays(DrawMode::Triangles, 0, 3));
    let mut pass = Pass::to_default();
    pass.set_viewport(&Viewport::new(0, 0, SIZE, SIZE));
    pass.add(&batch);
    ctx.render(&pass);
    let n = (SIZE >> level).max(1);
    let mut px = vec![0u8; (n * n * 4) as usize];
    e.gl.read_pixels_with_opt_u8_array(0, 0, n, n, Gl::RGBA, Gl::UNSIGNED_BYTE, Some(&mut px))
        .unwrap();
    ctx.destroy_param_group(group);
    ctx.destroy_param_layout(layout);
    ctx.destroy_program(program);
    px
}

// The largest difference over the channels in `channels` (indices 0..3).
fn worst(a: &[u8], b: &[u8], channels: &[usize]) -> i32 {
    let mut m = 0;
    for (pa, pb) in a.chunks(4).zip(b.chunks(4)) {
        for &c in channels {
            m = m.max((pa[c] as i32 - pb[c] as i32).abs());
        }
    }
    m
}

#[wasm_bindgen_test]
async fn compressed_textures_sample_close_to_the_original() {
    let e = env();
    let compressor = TextureCompressor::new(&e.ctx).expect("compressor");
    wasm_bindgen_futures::JsFuture::from(compressor.when_ready())
        .await
        .unwrap();
    let cases = [
        (CompressKind::Rgb, false, &[0usize, 1, 2][..], 12),
        (CompressKind::Rgb, true, &[0, 1, 2][..], 12),
        (CompressKind::Rgba, false, &[0, 1, 2, 3][..], 12),
        (CompressKind::Rgba, true, &[0, 1, 2, 3][..], 12),
        // BC4 splits a block's range into 7 steps: half a step at a small level.
        (CompressKind::Rg, false, &[0, 1][..], 6),
    ];
    let mut ran = 0;
    for (kind, srgb, channels, tolerance) in cases {
        if !compressor.supports(kind) {
            continue;
        }
        let tex = texture(&e.ctx, srgb);
        // Sampled once first: the renderer's binding cache holds it on a
        // unit, which compressing must not leave stale.
        let before: Vec<Vec<u8>> = (0..3).map(|l| show(&e, tex, l, srgb)).collect();
        if !compressor.compress(tex, kind) {
            // sRGB S3TC can be missing where S3TC is there.
            assert!(srgb, "{kind:?} compresses");
            continue;
        }
        ran += 1;
        for (level, want) in before.iter().enumerate() {
            let got = show(&e, tex, level as i32, srgb);
            let d = worst(want, &got, channels);
            assert!(
                d <= tolerance,
                "{kind:?} srgb {srgb} level {level}: off by {d} (≤ {tolerance})"
            );
        }
        e.ctx.destroy_texture(tex);
    }
    // A device with WebGL2 on desktop has S3TC; elsewhere, nothing to check.
    web_sys::console::log_1(&format!("compressed {ran} cases").into());
}

#[wasm_bindgen_test]
fn sizes_that_are_not_whole_blocks_stay_uncompressed() {
    let e = env();
    let compressor = TextureCompressor::new(&e.ctx).expect("compressor");
    let t = e
        .ctx
        .create_texture(
            TextureTarget::Texture2D,
            TextureMinFilter::Linear,
            TextureMagFilter::Linear,
        )
        .unwrap();
    e.ctx
        .texture_storage_2d(t, 1, &TextureFormat::rgba8(), 6, 8);
    assert!(!compressor.compress(t, CompressKind::Rgb));
}
