// GPU texture compression (TextureCompressor): a texture compressed in place
// samples close to what it held — every level, under the same handle, with
// a texture unit the renderer had it bound on before. Skips the formats the
// device lacks. Run with: wasm-pack test --headless --chrome packages/luxel-gl
#![cfg(target_arch = "wasm32")]

use crate::compress_util;

use wasm_bindgen_test::*;
use web_sys::WebGl2RenderingContext as Gl;

use luxel_gl::draw::{DrawCommand, DrawMode, Viewport};
use luxel_gl::ids::TextureId;
use luxel_gl::param::ParamLayoutBuilder;
use luxel_gl::pass::{Batch, Pass};
use luxel_gl::render_state::RenderState;
use luxel_gl::texture::{TextureFormat, TextureMagFilter, TextureMinFilter, TextureTarget};
use luxel_gl::texture_compressor::{CompressKind, TextureCompressor};
use luxel_gl::uniform_buffer::UniformType;

use compress_util::*;

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
