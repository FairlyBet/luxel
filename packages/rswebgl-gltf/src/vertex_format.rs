//! Vertex attributes in the smallest GPU format that keeps them: what a
//! quantized file (`KHR_mesh_quantization`, gltfpack / meshopt output)
//! stores goes up as it is — the vertex fetch dequantizes, the node
//! matrix does the rest, as in three.js and Babylon; float attributes
//! whose range allows it are quantized here:
//!
//! - normals and tangents → 16-bit signed normalized (glTF's own
//!   `KHR_mesh_quantization` type; gltfpack defaults to 8 bits, but its
//!   ~0.5° steps show as faceting in mirror-like reflections);
//! - texture coordinates in [0, 1] → 16-bit unsigned normalized (1/65535:
//!   under a tenth of a texel at 8192²); outside it (tiling) they stay f32;
//! - colors in [0, 1] → 16-bit unsigned normalized (8 bits would band in
//!   the darks: vertex colors are linear);
//! - joint indices → their own 8 / 16-bit integers, read as floats;
//! - positions and skin weights stay f32.
//!
//! Each vertex is padded to 4 bytes (glTF 2.0 §3.6.2.4 asks the same of
//! files; GPUs fetch aligned vertices faster).

use std::cell::Cell;

use rswebgl::vao::{AttrType, VertexAttr};
use wasm_bindgen::prelude::*;

thread_local! {
    static ENABLED: Cell<bool> = const { Cell::new(true) };
}

/// Whether models loaded from now on store float vertex attributes in
/// smaller GPU formats (on by default): 16-bit normals, tangents, texture
/// coordinates in [0, 1] and colors, half-float morph targets — a fifth
/// to a quarter less geometry memory on plain files (DamagedHelmet 0.75 →
/// 0.59 MB, Sponza 9.1 → 6.6 MB), under 1/32767 of error. Off: they stay 32-bit
/// floats (full precision). Attributes a file stores quantized
/// (`KHR_mesh_quantization`) go up as they are either way: nothing more to
/// keep.
#[wasm_bindgen(js_name = setVertexCompression)]
pub fn set_vertex_compression(on: bool) {
    ENABLED.with(|e| e.set(on));
}

/// See `setVertexCompression`.
#[wasm_bindgen(js_name = vertexCompression)]
pub fn vertex_compression() -> bool {
    enabled()
}

/// The format generated unit vectors (MikkTSpace tangents) go up in.
pub(crate) fn direction() -> Format {
    if enabled() {
        Format::SNORM16
    } else {
        Format::F32
    }
}

pub(crate) fn enabled() -> bool {
    ENABLED.with(Cell::get)
}

/// A component type an attribute is stored with on the GPU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Comp {
    F32,
    I8,
    U8,
    I16,
    U16,
}

impl Comp {
    fn size(self) -> usize {
        match self {
            Comp::F32 => 4,
            Comp::I8 | Comp::U8 => 1,
            Comp::I16 | Comp::U16 => 2,
        }
    }

    fn attr_type(self) -> AttrType {
        match self {
            Comp::F32 => AttrType::Float,
            Comp::I8 => AttrType::Byte,
            Comp::U8 => AttrType::UnsignedByte,
            Comp::I16 => AttrType::Short,
            Comp::U16 => AttrType::UnsignedShort,
        }
    }

    /// The type an accessor stores, if one a vertex attribute can be.
    pub(crate) fn of(ty: gltf::accessor::DataType) -> Option<Comp> {
        use gltf::accessor::DataType;
        Some(match ty {
            DataType::F32 => Comp::F32,
            DataType::I8 => Comp::I8,
            DataType::U8 => Comp::U8,
            DataType::I16 => Comp::I16,
            DataType::U16 => Comp::U16,
            DataType::U32 => return None,
        })
    }
}

/// An attribute's GPU storage: component type, and whether it is read
/// normalized (else as its integer values).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Format {
    pub comp: Comp,
    pub normalized: bool,
}

impl Format {
    pub(crate) const F32: Format = Format {
        comp: Comp::F32,
        normalized: false,
    };
    pub(crate) const SNORM16: Format = Format {
        comp: Comp::I16,
        normalized: true,
    };
    pub(crate) const UNORM16: Format = Format {
        comp: Comp::U16,
        normalized: true,
    };
}

/// The GPU format for float `values` of an attribute the file stores as
/// `file` (`normalized` as the accessor says) under `kind`.
pub(crate) fn choose(kind: Kind, file: Option<Comp>, normalized: bool, values: &[f32]) -> Format {
    if !enabled() {
        return match file {
            Some(comp) if comp != Comp::F32 => Format { comp, normalized },
            _ => Format::F32,
        };
    }
    match file {
        // Stored quantized: kept as it is (re-encoding the dequantized
        // values gives the file's integers back).
        Some(comp) if comp != Comp::F32 => Format { comp, normalized },
        _ => {
            let within = |lo: f32| values.iter().all(|v| (lo..=1.0).contains(v));
            match kind {
                Kind::Direction => Format::SNORM16,
                Kind::Unit if within(0.0) => Format::UNORM16,
                Kind::Unit | Kind::Exact => Format::F32,
            }
        }
    }
}

/// How far an attribute may be quantized.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Unit vectors (and the tangent's ±1 sign): signed normalized.
    Direction,
    /// Values usually in [0, 1] (texture coordinates, colors).
    Unit,
    /// Kept as stored (positions, weights).
    Exact,
}

/// `values` (`n` components a vertex) encoded as `format`, each vertex
/// padded to 4 bytes, and the vertex attribute reading them.
pub(crate) fn pack(values: &[f32], n: usize, format: Format) -> (Vec<u8>, VertexAttr) {
    let size = format.comp.size();
    let stride = (n * size).div_ceil(4) * 4;
    let count = values.len() / n.max(1);
    let mut out = vec![0u8; count * stride];
    for (v, chunk) in values.chunks_exact(n).enumerate() {
        for (c, &x) in chunk.iter().enumerate() {
            let at = v * stride + c * size;
            encode(&mut out[at..at + size], x, format);
        }
    }
    let mut attr = VertexAttr::float_from(n as u8, format.comp.attr_type(), format.normalized);
    attr.stride = stride as i32;
    (out, attr)
}

// One component, rounded to the nearest representable value.
fn encode(out: &mut [u8], x: f32, f: Format) {
    let q = |scale: f32, lo: f32, hi: f32| {
        let v = if f.normalized { x * scale } else { x };
        v.round().clamp(lo, hi)
    };
    match f.comp {
        Comp::F32 => out.copy_from_slice(&x.to_le_bytes()),
        Comp::I8 => out[0] = q(127.0, -127.0, 127.0) as i8 as u8,
        Comp::U8 => out[0] = q(255.0, 0.0, 255.0) as u8,
        Comp::I16 => out.copy_from_slice(&(q(32767.0, -32767.0, 32767.0) as i16).to_le_bytes()),
        Comp::U16 => out.copy_from_slice(&(q(65535.0, 0.0, 65535.0) as u16).to_le_bytes()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Decoded as the GPU (and glTF 2.0 §3.11) does.
    fn decode(b: &[u8], f: Format) -> f32 {
        let (v, scale) = match f.comp {
            Comp::F32 => return f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            Comp::I8 => (f32::from(b[0] as i8), 127.0),
            Comp::U8 => (f32::from(b[0]), 255.0),
            Comp::I16 => (f32::from(i16::from_le_bytes([b[0], b[1]])), 32767.0),
            Comp::U16 => (f32::from(u16::from_le_bytes([b[0], b[1]])), 65535.0),
        };
        if f.normalized {
            (v / scale).max(-1.0)
        } else {
            v
        }
    }

    #[test]
    fn quantized_files_round_trip_and_floats_quantize() {
        // A file's own snorm8 values come back exactly.
        let snorm8 = Format {
            comp: Comp::I8,
            normalized: true,
        };
        let vals: Vec<f32> = (-127..=127).map(|c| c as f32 / 127.0).collect();
        let (bytes, attr) = pack(&vals, 1, snorm8);
        assert_eq!(attr.stride, 4);
        for (i, v) in vals.iter().enumerate() {
            assert_eq!(decode(&bytes[i * 4..], snorm8), *v);
        }
        // Non-normalized u16 (quantized positions): the integers.
        let ints = [0.0, 1.0, 1234.0, 65535.0];
        let u16_raw = Format {
            comp: Comp::U16,
            normalized: false,
        };
        let (bytes, attr) = pack(&ints, 2, u16_raw);
        assert_eq!(attr.stride, 4);
        for (i, v) in ints.iter().enumerate() {
            assert_eq!(decode(&bytes[i * 2..], u16_raw), *v);
        }
        // Float normals → snorm16 (vec3 padded to 8 bytes), within 1/32767.
        let n = [0.6f32, -0.8, 0.0, 0.0, 0.0, 1.0];
        let f = choose(Kind::Direction, Some(Comp::F32), false, &n);
        assert_eq!(f, Format::SNORM16);
        let (bytes, attr) = pack(&n, 3, f);
        assert_eq!(attr.stride, 8);
        for (i, v) in n.iter().enumerate() {
            let at = (i / 3) * 8 + (i % 3) * 2;
            assert!((decode(&bytes[at..], f) - v).abs() <= 0.5 / 32767.0);
        }
        // Texture coordinates: unorm16 in [0, 1], f32 when they tile.
        assert_eq!(
            choose(Kind::Unit, Some(Comp::F32), false, &[0.0, 0.5, 1.0]),
            Format::UNORM16
        );
        assert_eq!(
            choose(Kind::Unit, Some(Comp::F32), false, &[0.0, 2.5]),
            Format::F32
        );
        assert_eq!(
            choose(Kind::Exact, Some(Comp::F32), false, &[0.5]),
            Format::F32
        );
    }
}
