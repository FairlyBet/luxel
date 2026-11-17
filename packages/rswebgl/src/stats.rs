// Live GPU objects and an estimate of the memory they take: for finding
// leaks (a count that keeps growing) and budgets. Sizes come from what the
// library allocated (storage extents and formats); the driver's own padding,
// mip tails and the canvas aren't counted.

use wasm_bindgen::prelude::*;
use web_sys::WebGl2RenderingContext as Gl;

use crate::context::Context;
use crate::texture::TextureTarget;

#[wasm_bindgen(typescript_custom_section)]
const TS_STATS: &str = r#"
/** Live GPU objects of a context and the memory they take, estimated from
 *  their formats and sizes (`ctx.resourceStats()`). */
export interface ResourceStats {
  buffers: number;
  textures: number;
  renderbuffers: number;
  framebuffers: number;
  programs: number;
  vertexArrays: number;
  uniformBuffers: number;
  paramLayouts: number;
  paramGroups: number;
  renderStates: number;
  samplers: number;
  queries: number;
  transformFeedbacks: number;
  /** Bytes of buffer storage. */
  bufferBytes: number;
  /** Bytes of texture storage, every level, layer and face. */
  textureBytes: number;
  /** Bytes of renderbuffer storage, every sample. */
  renderbufferBytes: number;
}
"#;

// A format's storage: bytes per texel, or per block for a compressed one.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Footprint {
    Texel(u32),
    Block { w: u32, h: u32, bytes: u32 },
}

pub(crate) fn footprint(internal: u32) -> Footprint {
    use Footprint::*;
    match internal {
        Gl::R8 | Gl::R8I | Gl::R8UI | Gl::R8_SNORM | Gl::STENCIL_INDEX8 => Texel(1),
        Gl::RG8
        | Gl::RG8I
        | Gl::RG8UI
        | Gl::RG8_SNORM
        | Gl::R16F
        | Gl::R16I
        | Gl::R16UI
        | Gl::DEPTH_COMPONENT16
        | Gl::RGB565
        | Gl::RGBA4
        | Gl::RGB5_A1 => Texel(2),
        Gl::RGB8 | Gl::SRGB8 | Gl::RGB8I | Gl::RGB8UI | Gl::RGB8_SNORM => Texel(3),
        Gl::DEPTH_COMPONENT24 => Texel(4),
        Gl::RGB16F | Gl::RGB16I | Gl::RGB16UI => Texel(6),
        Gl::RGB32F | Gl::RGB32I | Gl::RGB32UI => Texel(12),
        Gl::RGBA16F
        | Gl::RGBA16I
        | Gl::RGBA16UI
        | Gl::RG32F
        | Gl::RG32I
        | Gl::RG32UI
        | Gl::DEPTH32F_STENCIL8 => Texel(8),
        Gl::RGBA32F | Gl::RGBA32I | Gl::RGBA32UI => Texel(16),
        // S3TC / RGTC-free BC1, ETC2 RGB, EAC R11: 8 bytes a 4×4 block.
        0x83F0 | 0x83F1 | 0x8C4C | 0x8C4D | 0x9270 | 0x9271 | 0x9274..=0x9277 | 0x8DBB | 0x8DBC => {
            Block {
                w: 4,
                h: 4,
                bytes: 8,
            }
        }
        // BC2/3, RGTC2, BPTC, ETC2 RGBA, EAC RG11: 16 bytes a 4×4 block.
        0x83F2
        | 0x83F3
        | 0x8C4E
        | 0x8C4F
        | 0x9272
        | 0x9273
        | 0x9278
        | 0x9279
        | 0x8DBD
        | 0x8DBE
        | 0x8E8C..=0x8E8F => Block {
            w: 4,
            h: 4,
            bytes: 16,
        },
        // ASTC: 16 bytes a block of the size the format names.
        0x93B0..=0x93BD | 0x93D0..=0x93DD => {
            const DIMS: [(u32, u32); 14] = [
                (4, 4),
                (5, 4),
                (5, 5),
                (6, 5),
                (6, 6),
                (8, 5),
                (8, 6),
                (8, 8),
                (10, 5),
                (10, 6),
                (10, 8),
                (10, 10),
                (12, 10),
                (12, 12),
            ];
            let (w, h) = DIMS[(internal & 0xF) as usize % DIMS.len()];
            Block { w, h, bytes: 16 }
        }
        // RGBA8, sRGB8_A8, RGB10_A2, R11G11B10F, RGB9_E5, R32F / R32I / R32UI,
        // RG16*, DEPTH24_STENCIL8, DEPTH32F, and anything else: 4.
        _ => Texel(4),
    }
}

// Bytes of one `w × h` image.
pub(crate) fn image_bytes(internal: u32, w: i32, h: i32) -> u64 {
    let (w, h) = (w.max(0) as u64, h.max(0) as u64);
    match footprint(internal) {
        Footprint::Texel(b) => w * h * b as u64,
        Footprint::Block {
            w: bw,
            h: bh,
            bytes,
        } => w.div_ceil(bw as u64) * h.div_ceil(bh as u64) * bytes as u64,
    }
}

#[wasm_bindgen]
impl Context {
    /// The context's live GPU objects by kind and an estimate of the memory
    /// they take (buffers, textures with every level, layer and face,
    /// renderbuffers with every sample): a count that keeps growing is a
    /// leak.
    #[wasm_bindgen(js_name = resourceStats, unchecked_return_type = "ResourceStats")]
    pub fn resource_stats(&self) -> JsValue {
        let res = self.res();
        let buffer_bytes: u64 = res.buffers.iter().map(|(_, b)| b.size as u64).sum();
        let texture_bytes: u64 = res
            .textures
            .iter()
            .map(|(_, t)| {
                let internal = t.internal.get();
                let [levels, w, h, d] = t.extent.get();
                if internal == 0 {
                    return 0;
                }
                let faces = if t.target == TextureTarget::TextureCubeMap {
                    6
                } else {
                    1
                };
                let layered = matches!(
                    t.target,
                    TextureTarget::Texture3D | TextureTarget::Texture2DArray
                );
                (0..levels.max(1))
                    .map(|l| {
                        let (lw, lh) = ((w >> l).max(1), (h >> l).max(1));
                        // A 3D texture's depth halves too; an array's layers don't.
                        let ld = match t.target {
                            TextureTarget::Texture3D => (d >> l).max(1),
                            _ if layered => d.max(1),
                            _ => 1,
                        };
                        image_bytes(internal, lw, lh) * ld as u64 * faces
                    })
                    .sum::<u64>()
            })
            .sum();
        let renderbuffer_bytes: u64 = res.renderbuffers.iter().map(|(_, r)| r.bytes()).sum();
        let o = js_sys::Object::new();
        let set = |k: &str, v: f64| {
            let _ = js_sys::Reflect::set(&o, &k.into(), &v.into());
        };
        set("buffers", res.buffers.len() as f64);
        set("textures", res.textures.len() as f64);
        set("renderbuffers", res.renderbuffers.len() as f64);
        set("framebuffers", res.framebuffers.len() as f64);
        set("programs", res.programs.len() as f64);
        set("vertexArrays", res.vertex_arrays.len() as f64);
        set("uniformBuffers", res.uniform_buffers.len() as f64);
        set("paramLayouts", res.param_layouts.len() as f64);
        set("paramGroups", res.param_groups.len() as f64);
        set("renderStates", res.render_states.len() as f64);
        set("samplers", res.samplers.len() as f64);
        set("queries", res.queries.len() as f64);
        set("transformFeedbacks", res.feedbacks.len() as f64);
        set("bufferBytes", buffer_bytes as f64);
        set("textureBytes", texture_bytes as f64);
        set("renderbufferBytes", renderbuffer_bytes as f64);
        o.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_take_their_bytes() {
        assert_eq!(image_bytes(Gl::RGBA8, 4, 4), 64);
        assert_eq!(image_bytes(Gl::RGBA16F, 2, 2), 32);
        assert_eq!(image_bytes(Gl::DEPTH_COMPONENT16, 3, 1), 6);
        // BC1: a 5×5 image is 2×2 blocks of 8 bytes.
        assert_eq!(image_bytes(0x83F0, 5, 5), 32);
        // BC7: 16 bytes a block.
        assert_eq!(image_bytes(0x8E8C, 4, 4), 16);
        // ASTC 6×6: a 12×12 image is 2×2 blocks of 16.
        assert_eq!(image_bytes(0x93B4, 12, 12), 64);
        assert_eq!(image_bytes(0x93D4, 12, 12), 64);
    }
}
