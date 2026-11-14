//! Compress a texture on the GPU, after it was uploaded uncompressed: BC1
//! (opaque color), BC3 (color with alpha), BC5 (two channels: normal maps),
//! encoded by fragment shaders (Betsy's encoders, see `compress/bc.frag`).
//!
//! Each mip level is encoded into an RGBA32UI target whose texels are the
//! level's blocks in order, read into a pixel-pack buffer, and uploaded from
//! that same buffer into compressed storage (`compressedTexSubImage2D` from
//! a PIXEL_UNPACK_BUFFER): the data never comes back to the CPU. The
//! texture then holds the compressed storage under the same handle (param
//! groups, materials keep working) and the uncompressed one is freed — a
//! 2048² RGBA8 texture with mips, ~21 MB, becomes ~2.8 MB (BC1) or ~5.6 MB
//! (BC3 / BC5).
//!
//! ```ignore
//! let compressor = TextureCompressor::new(&ctx)?;
//! // after the image loaded (and its mips were made):
//! compressor.compress(base_color, CompressKind::Rgb);
//! ```

#[path = "compress/tables.rs"]
mod tables;

use wasm_bindgen::prelude::*;
use web_sys::WebGl2RenderingContext as Gl;

use crate::console;
use crate::context::Context;
use crate::draw::{DrawCommand, DrawMode, Viewport};
use crate::error::Error;
use crate::extension::Extension;
use crate::ids::{ParamGroupId, ParamLayoutId, ProgramId, TextureId};
use crate::param::{ParamField, ParamLayoutBuilder};
use crate::pass::{Batch, Pass};
use crate::render_state::RenderState;
use crate::texture::{TextureFormat, TextureMagFilter, TextureMinFilter, TextureTarget};
use crate::uniform_buffer::UniformType;

const BC_FRAG: &str = include_str!("compress/bc.frag");
const VERT: &str = "#version 300 es
void main() {
    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2)) * 2.0 - 1.0;
    gl_Position = vec4(p, 0.0, 1.0);
}
";

const RGBA8: u32 = 0x8058;
const SRGB8_ALPHA8: u32 = 0x8C43;
const MAX_ANISOTROPY: u32 = 0x84FE;
// The widest encode target (texels; a row of blocks wraps onto the next).
const TARGET_WIDTH: i32 = 1024;

/// What a texture holds, which picks its compressed format.
#[wasm_bindgen]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompressKind {
    /// Color (or data) without alpha: BC1, 4 bits a texel. sRGB stays sRGB.
    Rgb = 0,
    /// Color with alpha: BC3, 8 bits a texel.
    Rgba = 1,
    /// Two channels (a normal map's X and Y; the shader rebuilds Z): BC5,
    /// 8 bits a texel. Needs `EXT_texture_compression_rgtc`.
    Rg = 2,
}

// One encoder variant.
struct Encoder {
    program: ProgramId,
    params: ParamGroupId,
    f_source: ParamField,
    f_level: ParamField,
    f_blocks_x: ParamField,
    f_blocks: ParamField,
    f_width: ParamField,
}

/// Compresses textures on the GPU (see the module docs). Holds three
/// encoder programs (BC1, BC3, BC5 — compiling from creation on) and a
/// small table texture.
#[wasm_bindgen]
pub struct TextureCompressor {
    ctx: Context,
    layout: ParamLayoutId,
    tables: TextureId,
    // [kind][srgb]
    encoders: Vec<Option<Encoder>>,
    s3tc: bool,
    s3tc_srgb: bool,
    rgtc: bool,
}

#[wasm_bindgen]
impl TextureCompressor {
    /// Enables the compressed-texture extensions it uses (S3TC, S3TC sRGB,
    /// RGTC) where the device has them and starts compiling its encoders.
    #[wasm_bindgen(constructor)]
    pub fn new(ctx: &Context) -> Result<TextureCompressor, Error> {
        let s3tc = ctx.enable_extension_quietly(Extension::WebglCompressedTextureS3tc);
        let s3tc_srgb = ctx.enable_extension_quietly(Extension::WebglCompressedTextureS3tcSrgb);
        let rgtc = ctx.enable_extension_quietly(Extension::ExtTextureCompressionRgtc);
        let layout = ctx.create_param_layout(
            &ParamLayoutBuilder::new()
                .texture("u_source")
                .texture("u_tables")
                .uniform("u_level", UniformType::I32)
                .uniform("u_blocks_x", UniformType::I32)
                .uniform("u_blocks", UniformType::I32)
                .uniform("u_width", UniformType::I32),
        )?;
        let tables = ctx.create_texture(
            TextureTarget::Texture2D,
            TextureMinFilter::Nearest,
            TextureMagFilter::Nearest,
        )?;
        let rg8ui = TextureFormat {
            internal: 0x8238, // RG8UI
            format: 0x8228,   // RG_INTEGER
            data_type: Gl::UNSIGNED_BYTE,
        };
        ctx.texture_storage_2d(tables, 1, &rg8ui, 256, 2);
        let rows: Vec<u8> = tables::OMATCH5
            .iter()
            .chain(tables::OMATCH6.iter())
            .flatten()
            .copied()
            .collect();
        ctx.texture_sub_image_2d(tables, 0, 0, 0, 256, 2, &rg8ui, &rows);
        let mut me = Self {
            ctx: ctx.clone(),
            layout,
            tables,
            encoders: Vec::new(),
            s3tc,
            s3tc_srgb,
            rgtc,
        };
        for kind in [CompressKind::Rgb, CompressKind::Rgba, CompressKind::Rg] {
            for srgb in [false, true] {
                let encoder = if me.format(kind, srgb).is_some() {
                    Some(me.encoder(kind, srgb)?)
                } else {
                    None
                };
                me.encoders.push(encoder);
            }
        }
        Ok(me)
    }

    /// Whether this device can compress `kind` (its extension is there).
    pub fn supports(&self, kind: CompressKind) -> bool {
        self.format(kind, false).is_some()
    }

    /// Resolves once the encoders have compiled; `compress` before then
    /// waits for them (blocking).
    #[wasm_bindgen(js_name = whenReady)]
    pub fn when_ready(&self) -> js_sys::Promise {
        let programs = self.encoders.iter().flatten().map(|e| e.program).collect();
        let ready = self.ctx.programs_ready(programs);
        wasm_bindgen_futures::future_to_promise(async move {
            ready.await;
            Ok(JsValue::UNDEFINED)
        })
    }

    /// Compresses `tex` — a 2D RGBA8 or SRGB8_ALPHA8 texture, its levels
    /// filled (mips made) — in place: the same handle then samples the
    /// compressed texture, with its sampler settings. False (and the
    /// texture left as it was) where it can't be: the format isn't
    /// supported here, the texture isn't 2D RGBA8 / sRGB, its size isn't a
    /// multiple of 4 (S3TC needs whole blocks at level 0), or the context
    /// is lost.
    pub fn compress(&self, tex: TextureId, kind: CompressKind) -> bool {
        let _span = tracing::info_span!("textureCompressor.compress").entered();
        match self.try_compress(tex, kind) {
            Ok(done) => done,
            Err(e) => {
                console::error(&format!("[rswebgl] TextureCompressor: {e}"));
                false
            }
        }
    }

    /// Frees the encoders and the table texture.
    pub fn destroy(&self) {
        let res = self.ctx.res();
        let live = (
            res.textures.contains(self.tables.0),
            res.param_layouts.contains(self.layout.0),
        );
        drop(res);
        for e in self.encoders.iter().flatten() {
            if self.ctx.res().param_groups.contains(e.params.0) {
                self.ctx.destroy_param_group(e.params);
            }
            if self.ctx.res().programs.contains(e.program.0) {
                self.ctx.destroy_program(e.program);
            }
        }
        if live.0 {
            self.ctx.destroy_texture(self.tables);
        }
        if live.1 {
            self.ctx.destroy_param_layout(self.layout);
        }
    }
}

impl Drop for TextureCompressor {
    fn drop(&mut self) {
        self.destroy();
    }
}

// A compressed format: its GL enum and bytes a 4×4 block.
#[derive(Clone, Copy)]
struct BlockFormat {
    internal: u32,
    block_bytes: i32,
}

impl TextureCompressor {
    fn format(&self, kind: CompressKind, srgb: bool) -> Option<BlockFormat> {
        let (internal, block_bytes) = match (kind, srgb) {
            (CompressKind::Rgb, false) if self.s3tc => (0x83F0, 8), // RGB_S3TC_DXT1
            (CompressKind::Rgb, true) if self.s3tc_srgb => (0x8C4C, 8), // SRGB_S3TC_DXT1
            (CompressKind::Rgba, false) if self.s3tc => (0x83F3, 16), // RGBA_S3TC_DXT5
            (CompressKind::Rgba, true) if self.s3tc_srgb => (0x8C4F, 16), // SRGB_ALPHA_S3TC_DXT5
            (CompressKind::Rg, false) if self.rgtc => (0x8DBD, 16), // RG_RGTC2
            _ => return None,
        };
        Some(BlockFormat {
            internal,
            block_bytes,
        })
    }

    fn encoder(&self, kind: CompressKind, srgb: bool) -> Result<Encoder, Error> {
        let mode = match kind {
            CompressKind::Rgb => "MODE_BC1",
            CompressKind::Rgba => "MODE_BC3",
            CompressKind::Rg => "MODE_BC5",
        };
        let srgb = if srgb { "#define SRGB\n" } else { "" };
        let frag = format!("#version 300 es\n#define {mode}\n{srgb}{BC_FRAG}");
        let program = self.ctx.create_program(VERT, &frag)?;
        let params = self.ctx.create_param_group(self.layout)?;
        let field = |name: &str| self.ctx.param_field(self.layout, name);
        self.ctx
            .param_set_texture(params, field("u_tables")?, self.tables);
        Ok(Encoder {
            program,
            params,
            f_source: field("u_source")?,
            f_level: field("u_level")?,
            f_blocks_x: field("u_blocks_x")?,
            f_blocks: field("u_blocks")?,
            f_width: field("u_width")?,
        })
    }

    fn try_compress(&self, tex: TextureId, kind: CompressKind) -> Result<bool, Error> {
        let ctx = &self.ctx;
        if ctx.is_lost() {
            return Ok(false);
        }
        let Some((target, internal, [levels, width, height, _])) = ({
            let res = ctx.res();
            res.textures
                .get(tex.0)
                .map(|t| (t.target.clone(), t.internal.get(), t.extent.get()))
        }) else {
            return Err("compress: the texture is destroyed".into());
        };
        let srgb = match internal {
            RGBA8 => false,
            SRGB8_ALPHA8 => true,
            _ => return Ok(false),
        };
        if target != TextureTarget::Texture2D || width % 4 != 0 || height % 4 != 0 || levels < 1 {
            return Ok(false);
        }
        // BC5 has no sRGB form: a normal map is data anyway.
        let srgb = srgb && kind != CompressKind::Rg;
        let (Some(format), Some(Some(encoder))) = (
            self.format(kind, srgb),
            self.encoders.get(kind as usize * 2 + srgb as usize),
        ) else {
            return Ok(false);
        };
        let per_texel = 16 / format.block_bytes; // blocks an RGBA32UI texel holds

        // Each level's blocks, and where they go in the buffer.
        struct Level {
            width: i32,
            height: i32,
            blocks_x: i32,
            blocks: i32,
            target: (i32, i32),
            offset: i32,
        }
        let mut plan = Vec::new();
        let mut offset = 0;
        let mut widest = (1, 1);
        for l in 0..levels {
            let (w, h) = ((width >> l).max(1), (height >> l).max(1));
            let (bx, by) = ((w + 3) / 4, (h + 3) / 4);
            let blocks = bx * by;
            let texels = (blocks + per_texel - 1) / per_texel;
            let tw = texels.min(TARGET_WIDTH);
            let th = (texels + tw - 1) / tw;
            widest = (widest.0.max(tw), widest.1.max(th));
            plan.push(Level {
                width: w,
                height: h,
                blocks_x: bx,
                blocks,
                target: (tw, th),
                offset,
            });
            offset += tw * th * 16;
        }

        // The encode target, as large as the largest level needs.
        let rgba32ui = TextureFormat {
            internal: 0x8D70, // RGBA32UI
            format: 0x8D99,   // RGBA_INTEGER
            data_type: Gl::UNSIGNED_INT,
        };
        let scratch = ctx.create_texture(
            TextureTarget::Texture2D,
            TextureMinFilter::Nearest,
            TextureMagFilter::Nearest,
        )?;
        ctx.texture_storage_2d(scratch, 1, &rgba32ui, widest.0, widest.1);
        let fbo = match ctx.create_framebuffer(widest.0, widest.1) {
            Ok(fbo) => fbo,
            Err(e) => {
                ctx.destroy_texture(scratch);
                return Err(e);
            }
        };
        ctx.framebuffer_attach_color_texture_2d(fbo, 0, scratch, 0);
        let gl = ctx.gl().clone();
        let pbo = gl.create_buffer().ok_or("createBuffer failed")?;
        gl.bind_buffer(Gl::PIXEL_PACK_BUFFER, Some(&pbo));
        gl.buffer_data_with_i32(Gl::PIXEL_PACK_BUFFER, offset, Gl::STREAM_COPY);
        gl.bind_buffer(Gl::PIXEL_PACK_BUFFER, None);

        let state = ctx.render_state(&RenderState::new());
        ctx.param_set_texture(encoder.params, encoder.f_source, tex);
        let encoded = (|| -> Result<(), Error> {
            ctx.check_framebuffer(fbo)?;
            for (l, level) in plan.iter().enumerate() {
                ctx.param_set_i32(encoder.params, encoder.f_level, l as i32);
                ctx.param_set_i32(encoder.params, encoder.f_blocks_x, level.blocks_x);
                ctx.param_set_i32(encoder.params, encoder.f_blocks, level.blocks);
                ctx.param_set_i32(encoder.params, encoder.f_width, level.target.0);
                let mut batch = Batch::new(encoder.program, state);
                batch.set_group(0, encoder.params);
                batch.draw_vertexless(&DrawCommand::arrays(DrawMode::Triangles, 0, 3));
                let mut pass = Pass::new(fbo);
                pass.set_viewport(&Viewport::new(0, 0, level.target.0, level.target.1));
                pass.add(&batch);
                ctx.render(&pass);
                // The rendered blocks into the buffer.
                ctx.bind_read_source(
                    fbo,
                    0,
                    crate::readback::Region {
                        x: 0,
                        y: 0,
                        width: level.target.0,
                        height: level.target.1,
                    },
                )?;
                gl.bind_buffer(Gl::PIXEL_PACK_BUFFER, Some(&pbo));
                let read = gl.read_pixels_with_i32(
                    0,
                    0,
                    level.target.0,
                    level.target.1,
                    0x8D99, // RGBA_INTEGER
                    Gl::UNSIGNED_INT,
                    level.offset,
                );
                gl.bind_buffer(Gl::PIXEL_PACK_BUFFER, None);
                read.map_err(|e| format!("readPixels: {e:?}"))?;
            }
            Ok(())
        })();
        ctx.destroy_framebuffer(fbo);
        ctx.destroy_texture(scratch);
        if let Err(e) = encoded {
            gl.delete_buffer(Some(&pbo));
            return Err(e);
        }

        // The compressed texture, filled from the buffer; the old one's
        // sampler settings carried over.
        let raw = gl.create_texture().ok_or("createTexture failed")?;
        let anisotropic = ctx.is_extension_enabled(Extension::ExtTextureFilterAnisotropic);
        let settings = ctx.with_texture(tex, "compress", |gl, t, _| {
            let get = |p: u32| gl.get_tex_parameter(t, p);
            let mut s = vec![
                (Gl::TEXTURE_MIN_FILTER, get(Gl::TEXTURE_MIN_FILTER)),
                (Gl::TEXTURE_MAG_FILTER, get(Gl::TEXTURE_MAG_FILTER)),
                (Gl::TEXTURE_WRAP_S, get(Gl::TEXTURE_WRAP_S)),
                (Gl::TEXTURE_WRAP_T, get(Gl::TEXTURE_WRAP_T)),
                (Gl::TEXTURE_BASE_LEVEL, get(Gl::TEXTURE_BASE_LEVEL)),
                (Gl::TEXTURE_MAX_LEVEL, get(Gl::TEXTURE_MAX_LEVEL)),
            ];
            if anisotropic {
                s.push((MAX_ANISOTROPY, get(MAX_ANISOTROPY)));
            }
            s
        });
        let Some(settings) = settings else {
            gl.delete_buffer(Some(&pbo));
            gl.delete_texture(Some(&raw));
            return Ok(false);
        };
        ctx.gl_state().forget_texture_unit();
        gl.bind_texture(Gl::TEXTURE_2D, Some(&raw));
        gl.tex_storage_2d(Gl::TEXTURE_2D, levels, format.internal, width, height);
        gl.bind_buffer(Gl::PIXEL_UNPACK_BUFFER, Some(&pbo));
        for (l, level) in plan.iter().enumerate() {
            gl.compressed_tex_sub_image_2d_with_i32_and_i32(
                Gl::TEXTURE_2D,
                l as i32,
                0,
                0,
                level.width,
                level.height,
                format.internal,
                level.blocks * format.block_bytes,
                level.offset,
            );
        }
        gl.bind_buffer(Gl::PIXEL_UNPACK_BUFFER, None);
        for (pname, value) in settings {
            if pname == MAX_ANISOTROPY {
                if let Some(v) = value.as_f64() {
                    gl.tex_parameterf(Gl::TEXTURE_2D, pname, v as f32);
                }
            } else if let Some(v) = value.as_f64() {
                gl.tex_parameteri(Gl::TEXTURE_2D, pname, v as i32);
            }
        }
        gl.bind_texture(Gl::TEXTURE_2D, None);
        // Freed once the uploads that read it have run.
        gl.delete_buffer(Some(&pbo));
        ctx.replace_texture_storage(tex, raw, format.internal, [levels, width, height, 1]);
        Ok(true)
    }
}
