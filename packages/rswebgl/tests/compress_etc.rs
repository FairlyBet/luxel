// GPU texture compression to ETC2 / EAC (phones' formats): encoded on any
// device, decoded here per the Khronos Data Format spec — desktop Chrome
// has no ETC to sample. Its own file: its encoders take seconds to compile
// on ANGLE / D3D, past the runner's time with the other tests. Run with:
// wasm-pack test --headless --chrome packages/rswebgl
#![cfg(target_arch = "wasm32")]

use crate::compress_util;

use wasm_bindgen_test::*;

use rswebgl::texture_compressor::{CompressKind, TextureCompressor};

use compress_util::*;

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
    for (f, k) in [
        (0x9274, CompressKind::Rgb),
        (0x9278, CompressKind::Rgba),
        (0x9272, CompressKind::Rg),
    ] {
        let ready = compressor.prepare_format(f, k).unwrap();
        wasm_bindgen_futures::JsFuture::from(ready).await.unwrap();
    }
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

    let rgb = compressor
        .encode_level(tex, 0x9274, CompressKind::Rgb, 0)
        .expect("ETC2 RGB");
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
    let rgba = compressor
        .encode_level(tex, 0x9278, CompressKind::Rgba, 0)
        .expect("ETC2 RGBA");
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
    let rg = compressor
        .encode_level(tex, 0x9272, CompressKind::Rg, 0)
        .expect("RG11");
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
