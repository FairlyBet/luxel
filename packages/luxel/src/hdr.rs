//! Radiance `.hdr` (RGBE) decoding: the usual format of HDR environment maps.

/// A decoded HDR image: linear RGB, rows top to bottom.
#[derive(Debug, Clone, PartialEq)]
pub struct HdrImage {
    pub width: u32,
    pub height: u32,
    /// `width * height` RGB triples.
    pub rgb: Vec<f32>,
}

/// A Radiance image with its run-length coding undone but its pixels still
/// RGBE (a shared exponent per pixel: value = byte × 2^(e − 136)), rows top
/// to bottom. A quarter the size of the decoded floats; the environment bake
/// uploads it as is and decodes on the GPU.
#[derive(Debug, Clone, PartialEq)]
pub struct RgbeImage {
    pub width: u32,
    pub height: u32,
    /// `width * height` RGBE quadruples.
    pub rgbe: Vec<u8>,
}

/// Decodes a Radiance `.hdr` file (`32-bit_rle_rgbe`; new-style RLE, old-style
/// RLE or flat scanlines). Only the standard `-Y h +X w` orientation.
pub fn decode_hdr(bytes: &[u8]) -> Result<HdrImage, String> {
    let RgbeImage {
        width,
        height,
        rgbe,
    } = decode_hdr_rgbe(bytes)?;
    let rgb = rgbe
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| {
            if p[3] == 0 {
                [0.0; 3]
            } else {
                let f = f32::powi(2.0, p[3] as i32 - 136);
                [p[0] as f32 * f, p[1] as f32 * f, p[2] as f32 * f]
            }
        })
        .collect();
    Ok(HdrImage { width, height, rgb })
}

/// `decode_hdr` up to the RGBE pixels: the run-length coding undone, no
/// floats made.
pub fn decode_hdr_rgbe(bytes: &[u8]) -> Result<RgbeImage, String> {
    let mut pos = 0;
    let mut line = || -> Result<&[u8], String> {
        let start = pos;
        let end = bytes[start..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|n| start + n)
            .ok_or("HDR: header ended early")?;
        pos = end + 1;
        Ok(&bytes[start..end])
    };
    let magic = line()?;
    if !magic.starts_with(b"#?RADIANCE") && !magic.starts_with(b"#?RGBE") {
        return Err("HDR: not a Radiance file (no #?RADIANCE header)".into());
    }
    loop {
        let l = line()?;
        if l.is_empty() {
            break;
        }
        if let Some(format) = l.strip_prefix(b"FORMAT=")
            && format != b"32-bit_rle_rgbe"
        {
            return Err(format!(
                "HDR: unsupported format {}",
                String::from_utf8_lossy(format)
            ));
        }
    }
    let size = String::from_utf8_lossy(line()?).into_owned();
    let parts: Vec<&str> = size.split_whitespace().collect();
    let (height, width) = match parts.as_slice() {
        ["-Y", h, "+X", w] => (
            h.parse::<u32>().map_err(|_| "HDR: bad height")?,
            w.parse::<u32>().map_err(|_| "HDR: bad width")?,
        ),
        _ => return Err(format!("HDR: unsupported orientation \"{size}\"")),
    };
    if width == 0 || height == 0 || width > 16384 || height > 16384 {
        return Err(format!("HDR: unsupported size {width}×{height}"));
    }

    let data = &bytes[pos..];
    let (w, h) = (width as usize, height as usize);
    // A header can claim more than memory holds: an error, not an abort.
    let mut rgbe = Vec::new();
    rgbe.try_reserve_exact(w * h * 4)
        .map_err(|_| format!("HDR: {width}×{height} doesn't fit in memory"))?;
    rgbe.resize(w * h * 4, 0u8);
    let mut at = 0;
    for row in rgbe.chunks_exact_mut(w * 4) {
        at = read_scanline(data, at, row, w)?;
    }
    Ok(RgbeImage {
        width,
        height,
        rgbe,
    })
}

// One scanline of `w` RGBE pixels into `out`; returns the read position after
// it.
fn read_scanline(data: &[u8], mut at: usize, out: &mut [u8], w: usize) -> Result<usize, String> {
    let short = || "HDR: pixel data ended early".to_string();
    let head = data.get(at..at + 4).ok_or_else(short)?;
    // New-style RLE: 2, 2, width (big endian), then each channel separately.
    if (8..0x8000).contains(&w) && head[0] == 2 && head[1] == 2 && head[2] & 0x80 == 0 {
        if (head[2] as usize) << 8 | head[3] as usize != w {
            return Err("HDR: scanline width mismatch".into());
        }
        at += 4;
        for channel in 0..4 {
            let mut x = 0;
            while x < w {
                let count = *data.get(at).ok_or_else(short)? as usize;
                at += 1;
                if count > 128 {
                    let run = count - 128;
                    let value = *data.get(at).ok_or_else(short)?;
                    at += 1;
                    if x + run > w {
                        return Err("HDR: run past the scanline".into());
                    }
                    for i in 0..run {
                        out[(x + i) * 4 + channel] = value;
                    }
                    x += run;
                } else {
                    if count == 0 || x + count > w {
                        return Err("HDR: bad literal run".into());
                    }
                    let src = data.get(at..at + count).ok_or_else(short)?;
                    for (i, &v) in src.iter().enumerate() {
                        out[(x + i) * 4 + channel] = v;
                    }
                    at += count;
                    x += count;
                }
            }
        }
        return Ok(at);
    }
    // Flat pixels, possibly with old-style runs (1,1,1,n repeats the last
    // pixel n << shift times).
    let mut x = 0;
    let mut shift = 0;
    while x < w {
        let p = data.get(at..at + 4).ok_or_else(short)?;
        at += 4;
        if p[0] == 1 && p[1] == 1 && p[2] == 1 {
            if x == 0 {
                return Err("HDR: run before any pixel".into());
            }
            if shift >= 24 {
                return Err("HDR: too many run markers in a row".into());
            }
            let run = (p[3] as usize) << shift;
            if x + run > w {
                return Err("HDR: run past the scanline".into());
            }
            let last = [
                out[(x - 1) * 4],
                out[(x - 1) * 4 + 1],
                out[(x - 1) * 4 + 2],
                out[(x - 1) * 4 + 3],
            ];
            for i in 0..run {
                out[(x + i) * 4..(x + i) * 4 + 4].copy_from_slice(&last);
            }
            x += run;
            shift += 8;
        } else {
            out[x * 4..x * 4 + 4].copy_from_slice(p);
            x += 1;
            shift = 0;
        }
    }
    Ok(at)
}

/// Encodes linear RGB as a flat (uncompressed) Radiance file — for tests and
/// tools.
pub fn encode_hdr(width: u32, height: u32, rgb: &[f32]) -> Vec<u8> {
    let mut out =
        format!("#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y {height} +X {width}\n").into_bytes();
    for p in rgb.as_chunks::<3>().0 {
        let m = p[0].max(p[1]).max(p[2]);
        if m < 1e-32 {
            out.extend([0, 0, 0, 0]);
            continue;
        }
        // m = mantissa * 2^e with mantissa in [0.5, 1).
        let e = m.log2().floor() as i32 + 1;
        let scale = 256.0 / f32::powi(2.0, e);
        out.extend([
            (p[0] * scale) as u8,
            (p[1] * scale) as u8,
            (p[2] * scale) as u8,
            (e + 128) as u8,
        ]);
    }
    out
}

/// `image` at half its width and height (2×2 texels averaged, a side of 1
/// kept): for an environment larger than its bake needs or the GPU takes.
pub fn halve_rgbe(image: &RgbeImage) -> RgbeImage {
    let (w, h) = (image.width as usize, image.height as usize);
    let (hw, hh) = ((w / 2).max(1), (h / 2).max(1));
    let texel = |x: usize, y: usize| {
        let q = &image.rgbe[(y.min(h - 1) * w + x.min(w - 1)) * 4..][..4];
        if q[3] == 0 {
            return [0.0f32; 3];
        }
        // 2^(e − 128) / 256 per mantissa step.
        let f = f32::powi(2.0, q[3] as i32 - 136);
        [q[0] as f32 * f, q[1] as f32 * f, q[2] as f32 * f]
    };
    let mut rgbe = Vec::with_capacity(hw * hh * 4);
    for y in 0..hh {
        for x in 0..hw {
            let mut sum = [0.0f32; 3];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let t = texel(2 * x + dx, 2 * y + dy);
                for c in 0..3 {
                    sum[c] += t[c] * 0.25;
                }
            }
            let m = sum[0].max(sum[1]).max(sum[2]);
            if m < 1e-32 {
                rgbe.extend([0, 0, 0, 0]);
                continue;
            }
            let e = m.log2().floor() as i32 + 1;
            let scale = 256.0 / f32::powi(2.0, e);
            rgbe.extend([
                (sum[0] * scale).min(255.0) as u8,
                (sum[1] * scale).min(255.0) as u8,
                (sum[2] * scale).min(255.0) as u8,
                (e + 128).clamp(0, 255) as u8,
            ]);
        }
    }
    RgbeImage {
        width: hw as u32,
        height: hh as u32,
        rgbe,
    }
}

#[cfg(test)]
mod halve_tests {
    use super::*;

    #[test]
    fn halving_averages_and_keeps_light() {
        // 4×2: left half 1.0, right half 3.0 (grey).
        let rgb: Vec<f32> = (0..8)
            .flat_map(|i| {
                let v = if i % 4 < 2 { 1.0 } else { 3.0 };
                [v, v, v]
            })
            .collect();
        let img = decode_hdr_rgbe(&encode_hdr(4, 2, &rgb)).unwrap();
        let half = halve_rgbe(&img);
        assert_eq!((half.width, half.height), (2, 1));
        let back = decode_hdr(&encode_rgbe_file(&half)).unwrap();
        assert!(
            (back.rgb[0] - 1.0).abs() < 0.02 && (back.rgb[3] - 3.0).abs() < 0.05,
            "{:?}",
            back.rgb
        );
        let one = halve_rgbe(&halve_rgbe(&half));
        assert_eq!((one.width, one.height), (1, 1));
    }

    // A flat Radiance file of an RGBE image.
    fn encode_rgbe_file(img: &RgbeImage) -> Vec<u8> {
        let mut out = format!(
            "#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y {} +X {}\n",
            img.height, img.width
        )
        .into_bytes();
        out.extend(&img.rgbe);
        out
    }
}
