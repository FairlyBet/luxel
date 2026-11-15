// GPU texture compression, the formats this desktop can't sample: ASTC 4×4
// (decoded here per the spec) and BC6H for HDR textures and cube maps
// (sampled back where the device has BPTC). Split from compress.rs to stay
// within the runner's time per file: their encoders take seconds to compile
// on ANGLE / D3D. Run with: wasm-pack test --headless --chrome packages/rswebgl
#![cfg(target_arch = "wasm32")]

mod compress_util;

use wasm_bindgen_test::*;

use rswebgl::draw::{DrawCommand, DrawMode, Viewport};
use rswebgl::ids::TextureId;
use rswebgl::param::ParamLayoutBuilder;
use rswebgl::pass::{Batch, Pass};
use rswebgl::render_state::RenderState;
use rswebgl::texture::{TextureFormat, TextureMagFilter, TextureMinFilter, TextureTarget};
use rswebgl::texture_compressor::{CompressKind, TextureCompressor};
use rswebgl::uniform_buffer::UniformType;

use compress_util::*;

wasm_bindgen_test_configure!(run_in_browser);

// --- ASTC 4×4, the subset the encoder writes (one partition, LDR RGB / RGBA
// direct, 8-bit endpoints, a 4×4 grid of trit-coded weights), decoded per
// the Khronos Data Format spec's ASTC chapter ---

// Five trits from their 8-bit packing (spec C.2.12, "Decoding trits").
fn decode_trits(t: u32) -> [u32; 5] {
    let bit = |v: u32, i: u32| (v >> i) & 1;
    let (c, t4, t3);
    if (t >> 2) & 7 == 7 {
        c = (((t >> 5) & 7) << 2) | (t & 3);
        t4 = 2;
        t3 = 2;
    } else {
        c = t & 31;
        if (t >> 5) & 3 == 3 {
            t4 = 2;
            t3 = bit(t, 7);
        } else {
            t4 = bit(t, 7);
            t3 = (t >> 5) & 3;
        }
    }
    let (t2, t1, t0);
    if c & 3 == 3 {
        t2 = 2;
        t1 = bit(c, 4);
        t0 = (bit(c, 3) << 1) | (bit(c, 2) & !bit(c, 3) & 1);
    } else if (c >> 2) & 3 == 3 {
        t2 = 2;
        t1 = 2;
        t0 = c & 3;
    } else {
        t2 = bit(c, 4);
        t1 = (c >> 2) & 3;
        t0 = (bit(c, 1) << 1) | (bit(c, 0) & !bit(c, 1) & 1);
    }
    [t0, t1, t2, t3, t4]
}

// A block's 16 texels (RGBA, 0..255), row by row.
#[allow(clippy::needless_range_loop)]
fn decode_astc(b: &[u8]) -> Vec<[i32; 4]> {
    let bits = u128::from_le_bytes(b.try_into().unwrap());
    let get = |v: u128, at: u32, n: u32| ((v >> at) & ((1u128 << n) - 1)) as u32;
    let mode = get(bits, 0, 11);
    // Table C.2.8, the row with bits [1:0] nonzero.
    assert_ne!(mode & 3, 0, "block mode row");
    let r = ((mode & 3) << 1) | ((mode >> 4) & 1);
    let h = (mode >> 9) & 1;
    let (grid_w, grid_h) = (((mode >> 7) & 3) + 4, ((mode >> 5) & 3) + 2);
    assert_eq!((grid_w, grid_h), (4, 4), "weight grid");
    assert_eq!((mode >> 10) & 1, 0, "one plane");
    assert_eq!(get(bits, 11, 2), 0, "one partition");
    // Weight ranges (Table C.2.7): H = 0 → 2, 3, 4, 5, 6, 8; H = 1 → 10, 12, …
    let range = [[0, 0, 2, 3, 4, 5, 6, 8], [0, 0, 10, 12, 16, 20, 24, 32]][h as usize][r as usize];
    let wbits = match range {
        12 => 2,
        6 => 1,
        _ => panic!("weight range {range}"),
    };
    let cem = get(bits, 13, 4);
    let channels = match cem {
        8 => 3,
        12 => 4,
        _ => panic!("endpoint mode {cem}"),
    };
    let mut e = [[0i32; 4]; 2];
    for c in 0..channels {
        for k in 0..2 {
            e[k][c] = get(bits, 17 + (c as u32 * 2 + k as u32) * 8, 8) as i32;
        }
    }
    if channels == 3 {
        e[0][3] = 255;
        e[1][3] = 255;
    }
    // LDR RGB(A) direct: a second endpoint dimmer than the first means
    // blue contraction — the encoder never writes that.
    assert!(
        e[1][0] + e[1][1] + e[1][2] >= e[0][0] + e[0][1] + e[0][2],
        "blue contraction"
    );
    // Weights: from the top of the block down, bit-reversed.
    let rev = bits.reverse_bits();
    let mut weights = Vec::new();
    let mut pos = 0;
    while weights.len() < 16 {
        let mut m = [0u32; 5];
        let mut t = 0u32;
        let mut tpos = 0;
        for (i, tb) in [2u32, 2, 1, 2, 1].iter().enumerate() {
            m[i] = get(rev, pos, wbits);
            pos += wbits;
            t |= get(rev, pos, *tb) << tpos;
            pos += tb;
            tpos += tb;
        }
        let trits = decode_trits(t);
        for i in 0..5 {
            weights.push((trits[i] << wbits) | m[i]);
        }
    }
    weights.truncate(16);
    // Unquantize (C.2.17): the trit ranges' 0..64 values, in ASTC's order
    // (the trit is the high digit, the bits the low ones, bit-scrambled).
    let unq = |q: u32| -> i32 {
        let (scr12, scr6) = ([0, 4, 8, 2, 6, 10, 11, 7, 3, 9, 5, 1], [0, 2, 4, 5, 3, 1]);
        let (pos, n) = match range {
            12 => (scr12.iter().position(|&v| v == q).unwrap() as i32, 11),
            _ => (scr6.iter().position(|&v| v == q).unwrap() as i32, 5),
        };
        (pos * 64 + n / 2) / n
    };
    (0..16)
        .map(|i| {
            let w = unq(weights[i]);
            let mut t = [0; 4];
            for c in 0..4 {
                // 16-bit endpoints (×257), interpolated, back to 8 bits.
                let (a, b) = (e[0][c] * 257, e[1][c] * 257);
                t[c] = (((a * (64 - w) + b * w + 32) >> 6) + 128) / 257;
            }
            t
        })
        .collect()
}

#[wasm_bindgen_test]
async fn astc_blocks_decode_close_to_the_original() {
    let e = env();
    let compressor = TextureCompressor::new(&e.ctx).expect("compressor");
    for k in [CompressKind::Rgb, CompressKind::Rgba, CompressKind::Rg] {
        let ready = compressor.prepare_format(0x93B0, k).unwrap();
        wasm_bindgen_futures::JsFuture::from(ready).await.unwrap();
    }
    let tex = texture(&e.ctx, false);
    let src = image();
    let blocks = (SIZE / 4) as usize;
    for (kind, channels, tolerance) in [
        (CompressKind::Rgb, &[0usize, 1, 2][..], 8),
        (CompressKind::Rgba, &[0, 1, 2, 3][..], 16),
        (CompressKind::Rg, &[0, 1][..], 8),
    ] {
        let bytes = compressor.encode_level(tex, 0x93B0, kind, 0).expect("ASTC");
        assert_eq!(bytes.len(), blocks * blocks * 16);
        let (mut worst, mut sum, mut n) = (0, 0, 0);
        for by in 0..blocks {
            for bx in 0..blocks {
                let texels = decode_astc(&bytes[(by * blocks + bx) * 16..][..16]);
                for (i, t) in texels.iter().enumerate() {
                    let (x, y) = (bx * 4 + i % 4, by * 4 + i / 4);
                    let s = &src[(y * SIZE as usize + x) * 4..][..4];
                    for &c in channels {
                        let d = (t[c] - s[c] as i32).abs();
                        worst = worst.max(d);
                        sum += d;
                        n += 1;
                    }
                    if kind == CompressKind::Rg {
                        assert_eq!(t[2], 0, "a two-channel map reads blue 0");
                    }
                }
            }
        }
        let mean = sum as f32 / n as f32;
        web_sys::console::log_1(&format!("ASTC {kind:?}: worst {worst}, mean {mean:.2}").into());
        assert!(
            worst <= tolerance,
            "ASTC {kind:?}: off by {worst} (≤ {tolerance})"
        );
    }
}

// --- BC6H: HDR textures and cube maps, sampled back on this GPU ---

// Draws `tex` texel for texel into an RGBA32F target and reads it back:
// level 0 of a 2D texture, or face `face` of a cube (sampled along the
// direction to each texel's center, GL's cube layout).
fn show_float(e: &Env, tex: TextureId, cube: Option<i32>, size: i32) -> Vec<f32> {
    let ctx = &e.ctx;
    let frag = format!(
        "#version 300 es
precision highp float;
{}
uniform int u_face;
out vec4 frag_color;
void main() {{
    ivec2 p = ivec2(gl_FragCoord.xy);
#ifdef CUBE
    vec2 st = (vec2(p) + 0.5) / float({size}) * 2.0 - 1.0;
    vec3 d = u_face == 0 ? vec3(1.0, -st.y, -st.x) : u_face == 1 ? vec3(-1.0, -st.y, st.x)
           : u_face == 2 ? vec3(st.x, 1.0, st.y) : u_face == 3 ? vec3(st.x, -1.0, -st.y)
           : u_face == 4 ? vec3(st.x, -st.y, 1.0) : vec3(-st.x, -st.y, -1.0);
    frag_color = vec4(textureLod(u_tex, d, 0.0).rgb, 1.0);
#else
    frag_color = vec4(texelFetch(u_tex, p, 0).rgb, 1.0);
#endif
}}
",
        if cube.is_some() {
            "#define CUBE\nuniform highp samplerCube u_tex;"
        } else {
            "uniform highp sampler2D u_tex;"
        }
    );
    let program = ctx.create_program(VERT, &frag).unwrap();
    let layout = ctx
        .create_param_layout(
            &ParamLayoutBuilder::new()
                .texture("u_tex")
                .uniform("u_face", UniformType::I32),
        )
        .unwrap();
    let group = ctx.create_param_group(layout).unwrap();
    ctx.param_set_texture(group, ctx.param_field(layout, "u_tex").unwrap(), tex);
    let mut nearest = rswebgl::sampler::SamplerDesc::nearest_clamp();
    nearest.min_filter = TextureMinFilter::NearestMipmapNearest;
    let sampler = ctx.create_sampler(&nearest).unwrap();
    ctx.param_set_sampler(group, ctx.param_field(layout, "u_tex").unwrap(), sampler);
    ctx.param_set_i32(
        group,
        ctx.param_field(layout, "u_face").unwrap(),
        cube.unwrap_or(0),
    );
    let target = ctx
        .create_texture(
            TextureTarget::Texture2D,
            TextureMinFilter::Nearest,
            TextureMagFilter::Nearest,
        )
        .unwrap();
    ctx.texture_storage_2d(target, 1, &TextureFormat::rgba32f(), size, size);
    let fb = ctx.create_framebuffer(size, size).unwrap();
    ctx.framebuffer_attach_color_texture_2d(fb, 0, target, 0);
    let mut batch = Batch::new(program, ctx.render_state(&RenderState::new()));
    batch.set_group(0, group);
    batch.draw_vertexless(&DrawCommand::arrays(DrawMode::Triangles, 0, 3));
    let mut pass = Pass::new(fb);
    pass.set_viewport(&Viewport::new(0, 0, size, size));
    pass.add(&batch);
    ctx.render(&pass);
    let bytes = ctx
        .read_pixels(
            fb,
            0,
            0,
            0,
            size,
            size,
            rswebgl::readback::ReadFormat::RgbaF32,
        )
        .unwrap();
    let out: Vec<f32> = bytes
        .chunks(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    ctx.destroy_framebuffer(fb);
    ctx.destroy_texture(target);
    ctx.destroy_sampler(sampler);
    ctx.destroy_param_group(group);
    ctx.destroy_param_layout(layout);
    ctx.destroy_program(program);
    out
}

// HDR texels as a sky has them: a hue of its own per face, its brightness
// varying smoothly from ~0.1 to ~12 (within a block the colors lie near a
// line, which is what BC6H stores; channels varying independently inside
// a block — a plane of colors — it can't).
fn hdr_texels(face: i32, size: i32) -> Vec<f32> {
    let hues = [
        [1.0, 0.6, 0.3],
        [0.3, 0.6, 1.0],
        [1.0, 1.0, 1.0],
        [0.5, 1.0, 0.4],
        [1.0, 0.3, 0.7],
        [0.7, 0.7, 1.0],
    ];
    let hue = hues[face as usize];
    let mut v = Vec::new();
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = (x as f32 / size as f32, y as f32 / size as f32);
            let l = 0.1 + 10.0 * fx * fx + 2.0 * fy;
            v.extend_from_slice(&[l * hue[0], l * hue[1], l * hue[2], 1.0]);
        }
    }
    v
}

fn f32_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

// The largest error relative to the texel's brightest channel (+ 0.25): a
// block holds one line of colors per region, so a dim channel beside a
// bright one errs by a share of the bright one, as it is seen.
fn worst_relative(a: &[f32], b: &[f32]) -> f32 {
    let mut m = 0f32;
    for (pa, pb) in a.chunks(4).zip(b.chunks(4)) {
        let scale = 0.25 + pa[0].max(pa[1]).max(pa[2]);
        for c in 0..3 {
            m = m.max((pa[c] - pb[c]).abs() / scale);
        }
    }
    m
}

#[wasm_bindgen_test]
async fn bc6h_textures_and_cubes_sample_close_to_the_original() {
    let e = env();
    if !e
        .ctx
        .enable_extension(rswebgl::extension::Extension::ExtColorBufferFloat)
    {
        return; // can't read floats back here
    }
    let compressor = TextureCompressor::new(&e.ctx).expect("compressor");
    if !compressor.supports(CompressKind::Hdr) {
        return; // no BPTC here
    }
    wasm_bindgen_futures::JsFuture::from(compressor.when_ready())
        .await
        .unwrap();
    let f16 = TextureFormat::rgba16f_from_float();

    // A 2D RGBA16F texture.
    let t = e
        .ctx
        .create_texture(
            TextureTarget::Texture2D,
            TextureMinFilter::Nearest,
            TextureMagFilter::Nearest,
        )
        .unwrap();
    e.ctx
        .texture_storage_2d(t, 1, &TextureFormat::rgba16f(), SIZE, SIZE);
    e.ctx.texture_sub_image_2d(
        t,
        0,
        0,
        0,
        SIZE,
        SIZE,
        &f16,
        &f32_bytes(&hdr_texels(0, SIZE)),
    );
    let before = show_float(&e, t, None, SIZE);
    assert!(compressor.compress(t, CompressKind::Hdr), "2D compresses");
    let after = show_float(&e, t, None, SIZE);
    let d = worst_relative(&before, &after);
    web_sys::console::log_1(&format!("BC6H 2D: worst relative {d:.3}").into());
    assert!(d < 0.12, "BC6H 2D: off by {d}");

    // A cube, every face its own pattern (a face read from the wrong one,
    // or flipped, would show).
    let n = 16;
    let c = e
        .ctx
        .create_texture(
            TextureTarget::TextureCubeMap,
            TextureMinFilter::NearestMipmapNearest,
            TextureMagFilter::Nearest,
        )
        .unwrap();
    e.ctx
        .texture_storage_2d(c, 2, &TextureFormat::rgba16f(), n, n);
    use rswebgl::texture::CubeMapFace as F;
    let faces = [
        F::PositiveX,
        F::NegativeX,
        F::PositiveY,
        F::NegativeY,
        F::PositiveZ,
        F::NegativeZ,
    ];
    for (i, f) in faces.iter().enumerate() {
        for level in 0..2 {
            let s = n >> level;
            e.ctx.texture_sub_image_cube_face(
                c,
                *f,
                level,
                0,
                0,
                s,
                s,
                &f16,
                &f32_bytes(&hdr_texels(i as i32, s)),
            );
        }
    }
    let before: Vec<_> = (0..6).map(|f| show_float(&e, c, Some(f), n)).collect();
    assert!(
        compressor.compress(c, CompressKind::Hdr),
        "the cube compresses"
    );
    for (f, want) in before.iter().enumerate() {
        let got = show_float(&e, c, Some(f as i32), n);
        let d = worst_relative(want, &got);
        web_sys::console::log_1(&format!("BC6H cube face {f}: worst relative {d:.3}").into());
        assert!(d < 0.12, "BC6H cube face {f}: off by {d}");
    }
}
