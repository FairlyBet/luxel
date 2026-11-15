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

// ETC1 holds one hue per half block (only brightness varies inside), so a
// gradient across hues errs a little more than BC1.
// --- ETC2 / EAC: encoded on any device, decoded here (Khronos Data Format
// spec, "ETC2" / "EAC" sections) ---

const ETC1_MODIFIERS: [[i32; 4]; 8] = [
    [2, 8, -2, -8],
    [5, 17, -5, -17],
    [9, 29, -9, -29],
    [13, 42, -13, -42],
    [18, 60, -18, -60],
    [24, 80, -24, -80],
    [33, 106, -33, -106],
    [47, 183, -47, -183],
];

const EAC_MODIFIERS: [[i32; 8]; 16] = [
    [-3, -6, -9, -15, 2, 5, 8, 14],
    [-3, -7, -10, -13, 2, 6, 9, 12],
    [-2, -5, -8, -13, 1, 4, 7, 12],
    [-2, -4, -6, -13, 1, 3, 5, 12],
    [-3, -6, -8, -12, 2, 5, 7, 11],
    [-3, -7, -9, -11, 2, 6, 8, 10],
    [-4, -7, -8, -11, 3, 6, 7, 10],
    [-3, -5, -8, -11, 2, 4, 7, 10],
    [-2, -6, -8, -10, 1, 5, 7, 9],
    [-2, -5, -8, -10, 1, 4, 7, 9],
    [-2, -4, -8, -10, 1, 3, 7, 9],
    [-2, -5, -7, -10, 1, 4, 6, 9],
    [-3, -4, -7, -10, 2, 3, 6, 9],
    [-1, -2, -3, -10, 0, 1, 2, 9],
    [-4, -6, -8, -9, 3, 5, 7, 8],
    [-3, -5, -7, -9, 2, 4, 6, 8],
];

// An ETC1-mode block's 16 colors, indexed y * 4 + x. Panics on the ETC2
// modes (T, H, P): the encoder doesn't write them.
fn decode_etc1(b: &[u8]) -> [[i32; 3]; 16] {
    let diff = b[3] & 2 != 0;
    let flip = b[3] & 1 != 0;
    let tables = [(b[3] >> 5) as usize, ((b[3] >> 2) & 7) as usize];
    let mut base = [[0i32; 3]; 2];
    for c in 0..3 {
        if diff {
            let c1 = (b[c] >> 3) as i32;
            let d = ((b[c] & 7) as i32) << 29 >> 29; // 3-bit signed
            let c2 = c1 + d;
            assert!(
                (0..32).contains(&c2),
                "a differential block overflows: an ETC2 mode"
            );
            base[0][c] = (c1 << 3) | (c1 >> 2);
            base[1][c] = (c2 << 3) | (c2 >> 2);
        } else {
            let (c1, c2) = ((b[c] >> 4) as i32, (b[c] & 15) as i32);
            base[0][c] = c1 * 17;
            base[1][c] = c2 * 17;
        }
    }
    let msb = u16::from_be_bytes([b[4], b[5]]);
    let lsb = u16::from_be_bytes([b[6], b[7]]);
    let mut out = [[0; 3]; 16];
    for y in 0..4 {
        for x in 0..4 {
            let bit = x * 4 + y;
            let idx = ((((msb >> bit) & 1) << 1) | ((lsb >> bit) & 1)) as usize;
            let sub = if flip {
                (y >= 2) as usize
            } else {
                (x >= 2) as usize
            };
            let m = ETC1_MODIFIERS[tables[sub]][idx];
            for c in 0..3 {
                out[y * 4 + x][c] = (base[sub][c] + m).clamp(0, 255);
            }
        }
    }
    out
}

// An EAC block's 16 values (0..255), indexed y * 4 + x; `r11` decodes the
// 11-bit form and scales it back to 8 bits.
fn decode_eac(b: &[u8], r11: bool) -> [i32; 16] {
    let base = b[0] as i32;
    let mul = (b[1] >> 4) as i32;
    let table = (b[1] & 15) as usize;
    let idx = b[2..8].iter().fold(0u64, |a, &v| (a << 8) | v as u64);
    let mut out = [0; 16];
    for i in 0..16 {
        let j = ((idx >> (45 - 3 * i)) & 7) as usize;
        let m = EAC_MODIFIERS[table][j];
        let (x, y) = (i / 4, i % 4);
        out[y * 4 + x] = if r11 {
            let mul = if mul == 0 { 1 } else { mul * 8 };
            let v = (base * 8 + 4 + m * mul).clamp(0, 2047);
            (v as f32 * 255.0 / 2047.0).round() as i32
        } else {
            (base + m * mul).clamp(0, 255)
        };
    }
    out
}

#[wasm_bindgen_test]
async fn etc2_and_eac_blocks_decode_close_to_the_original() {
    let e = env();
    let compressor = TextureCompressor::new(&e.ctx).expect("compressor");
    let tex = texture(&e.ctx, false);
    let src = image();
    let blocks = (SIZE / 4) as usize;
    // Every block's decoded texels against the source.
    let check = |name: &str,
                 bytes: &[u8],
                 block_bytes: usize,
                 decode: &dyn Fn(&[u8]) -> Vec<[i32; 4]>,
                 channels: &[usize],
                 tolerance: i32| {
        assert_eq!(bytes.len(), blocks * blocks * block_bytes, "{name}: size");
        let (mut worst, mut sum, mut n) = (0, 0, 0);
        for by in 0..blocks {
            for bx in 0..blocks {
                let block = &bytes[(by * blocks + bx) * block_bytes..][..block_bytes];
                let texels = decode(block);
                for (i, t) in texels.iter().enumerate() {
                    let (x, y) = (bx * 4 + i % 4, by * 4 + i / 4);
                    let s = &src[(y * SIZE as usize + x) * 4..][..4];
                    for &c in channels {
                        let d = (t[c] - s[c] as i32).abs();
                        worst = worst.max(d);
                        sum += d;
                        n += 1;
                    }
                }
            }
        }
        let mean = sum as f32 / n as f32;
        web_sys::console::log_1(&format!("{name}: worst {worst}, mean {mean:.2}").into());
        assert!(worst <= tolerance, "{name}: off by {worst} (≤ {tolerance})");
        assert!(mean <= 4.0, "{name}: off by {mean} on average");
    };

    let rgb = compressor.encode_level(tex, 0x9274, 0).expect("ETC2 RGB");
    check(
        "ETC2 RGB",
        &rgb,
        8,
        &|b| {
            decode_etc1(b)
                .iter()
                .map(|c| [c[0], c[1], c[2], 255])
                .collect()
        },
        &[0, 1, 2],
        16,
    );
    let rgba = compressor.encode_level(tex, 0x9278, 0).expect("ETC2 RGBA");
    check(
        "ETC2 RGBA",
        &rgba,
        16,
        &|b| {
            let a = decode_eac(&b[..8], false);
            decode_etc1(&b[8..])
                .iter()
                .zip(a)
                .map(|(c, a)| [c[0], c[1], c[2], a])
                .collect()
        },
        &[0, 1, 2, 3],
        16,
    );
    let rg = compressor.encode_level(tex, 0x9272, 0).expect("RG11");
    check(
        "RG11",
        &rg,
        16,
        &|b| {
            let (r, g) = (decode_eac(&b[..8], true), decode_eac(&b[8..], true));
            (0..16).map(|i| [r[i], g[i], 0, 255]).collect()
        },
        &[0, 1],
        6,
    );
}
