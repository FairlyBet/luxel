//! Compress a texture on the GPU, after it was uploaded uncompressed. On
//! desktops BC (S3TC / RGTC): BC1 for opaque color, BC3 with alpha, BC5 for
//! two channels (normal maps). Where those are missing — phones — ETC2 /
//! EAC: ETC2 RGB, ETC2 RGBA, RG11. Encoded by fragment shaders
//! (`compress/bc.frag`, `compress/etc.frag`).
//!
//! Each mip level is encoded into an RGBA32UI target whose texels are the
//! level's blocks in order, read into a pixel-pack buffer, and uploaded from
//! that same buffer into compressed storage (`compressedTexSubImage2D` from
//! a PIXEL_UNPACK_BUFFER): the data never comes back to the CPU. The
//! texture then holds the compressed storage under the same handle (param
//! groups, materials keep working) and the uncompressed one is freed — a
//! 2048² RGBA8 texture with mips, ~21 MB, becomes ~2.8 MB (BC1, ETC2 RGB)
//! or ~5.6 MB (the rest).
//!
//! ```ignore
//! let compressor = TextureCompressor::new(&ctx)?;
//! // after the image loaded (and its mips were made):
//! compressor.compress(base_color, CompressKind::Rgb);
//! ```

#[path = "compress/tables.rs"]
mod tables;

use std::cell::RefCell;
use std::collections::HashMap;

use wasm_bindgen::prelude::*;
use web_sys::WebGl2RenderingContext as Gl;

use crate::console;
use crate::context::Context;
use crate::draw::{DrawCommand, DrawMode, Viewport};
use crate::error::Error;
use crate::extension::Extension;
use crate::ids::{FramebufferId, ParamGroupId, ParamLayoutId, ProgramId, TextureId};
use crate::param::{ParamField, ParamLayoutBuilder};
use crate::pass::{Batch, Pass};
use crate::render_state::RenderState;
use crate::texture::{TextureFormat, TextureMagFilter, TextureMinFilter, TextureTarget};
use crate::uniform_buffer::UniformType;

const BC_FRAG: &str = include_str!("compress/bc.frag");
const ETC_FRAG: &str = include_str!("compress/etc.frag");
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CompressKind {
    /// Color (or data) without alpha: BC1 or ETC2 RGB, 4 bits a texel.
    /// sRGB stays sRGB.
    Rgb = 0,
    /// Color with alpha: BC3 or ETC2 RGBA (EAC alpha), 8 bits a texel.
    Rgba = 1,
    /// Two channels (a normal map's X and Y; the shader rebuilds Z): BC5
    /// or RG11 EAC, 8 bits a texel.
    Rg = 2,
}

// A compressed format and how it is encoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BlockFormat {
    internal: u32,
    block_bytes: i32,
    kind: CompressKind,
    srgb: bool,
    etc: bool,
}

// Every format the encoders write (BC first: preferred where both exist).
const FORMATS: [BlockFormat; 10] = {
    const fn f(
        internal: u32,
        bytes: i32,
        kind: CompressKind,
        srgb: bool,
        etc: bool,
    ) -> BlockFormat {
        BlockFormat {
            internal,
            block_bytes: bytes,
            kind,
            srgb,
            etc,
        }
    }
    use CompressKind::*;
    [
        f(0x83F0, 8, Rgb, false, false),   // RGB_S3TC_DXT1
        f(0x8C4C, 8, Rgb, true, false),    // SRGB_S3TC_DXT1
        f(0x83F3, 16, Rgba, false, false), // RGBA_S3TC_DXT5
        f(0x8C4F, 16, Rgba, true, false),  // SRGB_ALPHA_S3TC_DXT5
        f(0x8DBD, 16, Rg, false, false),   // RG_RGTC2
        f(0x9274, 8, Rgb, false, true),    // RGB8_ETC2
        f(0x9275, 8, Rgb, true, true),     // SRGB8_ETC2
        f(0x9278, 16, Rgba, false, true),  // RGBA8_ETC2_EAC
        f(0x9279, 16, Rgba, true, true),   // SRGB8_ALPHA8_ETC2_EAC
        f(0x9272, 16, Rg, false, true),    // RG11_EAC
    ]
};

// One encoder program.
struct Encoder {
    program: ProgramId,
    params: ParamGroupId,
    f_source: ParamField,
    f_level: ParamField,
    f_blocks_x: ParamField,
    f_blocks: ParamField,
    f_width: ParamField,
}

/// Compresses textures on the GPU (see the module docs). Holds an encoder
/// program per format the device has (compiling from creation on) and a
/// small table texture.
#[wasm_bindgen]
pub struct TextureCompressor {
    ctx: Context,
    bc_layout: ParamLayoutId,
    etc_layout: ParamLayoutId,
    tables: TextureId,
    // By the format's GL enum.
    encoders: RefCell<HashMap<u32, Encoder>>,
    s3tc: bool,
    s3tc_srgb: bool,
    rgtc: bool,
    etc: bool,
}

#[wasm_bindgen]
impl TextureCompressor {
    /// Enables the compressed-texture extensions it uses (S3TC, S3TC sRGB,
    /// RGTC, ETC) where the device has them and starts compiling its
    /// encoders.
    #[wasm_bindgen(constructor)]
    pub fn new(ctx: &Context) -> Result<TextureCompressor, Error> {
        let s3tc = ctx.enable_extension_quietly(Extension::WebglCompressedTextureS3tc);
        let s3tc_srgb = ctx.enable_extension_quietly(Extension::WebglCompressedTextureS3tcSrgb);
        let rgtc = ctx.enable_extension_quietly(Extension::ExtTextureCompressionRgtc);
        let etc = ctx.enable_extension_quietly(Extension::WebglCompressedTextureEtc);
        let layout = |tables: bool| {
            let mut b = ParamLayoutBuilder::new().texture("u_source");
            if tables {
                b = b.texture("u_tables");
            }
            ctx.create_param_layout(
                &b.uniform("u_level", UniformType::I32)
                    .uniform("u_blocks_x", UniformType::I32)
                    .uniform("u_blocks", UniformType::I32)
                    .uniform("u_width", UniformType::I32),
            )
        };
        let bc_layout = layout(true)?;
        let etc_layout = layout(false)?;
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
        let me = Self {
            ctx: ctx.clone(),
            bc_layout,
            etc_layout,
            tables,
            encoders: RefCell::new(HashMap::new()),
            s3tc,
            s3tc_srgb,
            rgtc,
            etc,
        };
        for kind in [CompressKind::Rgb, CompressKind::Rgba, CompressKind::Rg] {
            for srgb in [false, true] {
                if let Some(format) = me.format(kind, srgb) {
                    me.encoder(format)?;
                }
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
        let programs = self.encoders.borrow().values().map(|e| e.program).collect();
        let ready = self.ctx.programs_ready(programs);
        wasm_bindgen_futures::future_to_promise(async move {
            ready.await;
            Ok(JsValue::UNDEFINED)
        })
    }

    /// Compresses `tex` — a 2D RGBA8 or SRGB8_ALPHA8 texture, its levels
    /// filled (mips made) — in place: the same handle then samples the
    /// compressed texture, with its sampler settings. False (and the
    /// texture left as it was) where it can't be: no format for `kind`
    /// here, the texture isn't 2D RGBA8 / sRGB, its size isn't a multiple
    /// of 4 (S3TC needs whole blocks at level 0), or the context is lost.
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

    /// Level `level` of `tex` encoded as `internal` (any format the
    /// encoders write, whether or not the device samples it), its blocks'
    /// bytes. For tests: a format the device lacks can still be decoded on
    /// the CPU.
    #[doc(hidden)]
    #[wasm_bindgen(js_name = encodeLevel)]
    pub fn encode_level(
        &self,
        tex: TextureId,
        internal: u32,
        level: i32,
    ) -> Result<Vec<u8>, Error> {
        let format = FORMATS
            .iter()
            .find(|f| f.internal == internal)
            .copied()
            .ok_or("encodeLevel: not a format the encoders write")?;
        let extent = self
            .ctx
            .res()
            .textures
            .get(tex.0)
            .map(|t| t.extent.get())
            .ok_or("encodeLevel: the texture is destroyed")?;
        let plan = plan_levels(&format, extent[1], extent[2], level + 1);
        let l = plan.last().ok_or("encodeLevel: no such level")?;
        let (fbo, scratch) = self.scratch(l.target)?;
        let out = (|| {
            self.encode(tex, format, level, l, fbo)?;
            let gl = self.ctx.gl();
            let mut words = vec![0u32; (l.target.0 * l.target.1 * 4) as usize];
            // SAFETY: the view lives only for the call.
            let view = unsafe { js_sys::Uint32Array::view(&words) };
            gl.read_pixels_with_opt_array_buffer_view(
                0,
                0,
                l.target.0,
                l.target.1,
                0x8D99, // RGBA_INTEGER
                Gl::UNSIGNED_INT,
                Some(&view),
            )
            .map_err(|e| format!("readPixels: {e:?}"))?;
            drop(view);
            words.truncate((l.blocks * format.block_bytes / 4) as usize);
            Ok(words.iter().flat_map(|w| w.to_le_bytes()).collect())
        })();
        self.ctx.destroy_framebuffer(fbo);
        self.ctx.destroy_texture(scratch);
        out
    }

    /// Frees the encoders and the table texture.
    pub fn destroy(&self) {
        let res = self.ctx.res();
        let live = (
            res.textures.contains(self.tables.0),
            res.param_layouts.contains(self.bc_layout.0),
            res.param_layouts.contains(self.etc_layout.0),
        );
        drop(res);
        for (_, e) in self.encoders.borrow_mut().drain() {
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
            self.ctx.destroy_param_layout(self.bc_layout);
        }
        if live.2 {
            self.ctx.destroy_param_layout(self.etc_layout);
        }
    }
}

impl Drop for TextureCompressor {
    fn drop(&mut self) {
        self.destroy();
    }
}

// A level's blocks, the target rectangle that holds them, and where they go
// in the buffer.
struct Level {
    width: i32,
    height: i32,
    blocks_x: i32,
    blocks: i32,
    target: (i32, i32),
    offset: i32,
}

fn plan_levels(format: &BlockFormat, width: i32, height: i32, levels: i32) -> Vec<Level> {
    let per_texel = 16 / format.block_bytes; // blocks an RGBA32UI texel holds
    let mut plan = Vec::new();
    let mut offset = 0;
    for l in 0..levels {
        let (w, h) = ((width >> l).max(1), (height >> l).max(1));
        let (bx, by) = ((w + 3) / 4, (h + 3) / 4);
        let blocks = bx * by;
        let texels = (blocks + per_texel - 1) / per_texel;
        let tw = texels.min(TARGET_WIDTH);
        let th = (texels + tw - 1) / tw;
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
    plan
}

impl TextureCompressor {
    // The format `kind` compresses to here: BC where the device has it, else
    // ETC2 / EAC.
    fn format(&self, kind: CompressKind, srgb: bool) -> Option<BlockFormat> {
        FORMATS.iter().copied().find(|f| {
            f.kind == kind
                && f.srgb == srgb
                && match (f.etc, f.kind, f.srgb) {
                    (true, _, _) => self.etc,
                    (false, CompressKind::Rg, _) => self.rgtc,
                    (false, _, false) => self.s3tc,
                    (false, _, true) => self.s3tc_srgb,
                }
        })
    }

    // The encoder for `format` (made on first use).
    fn encoder(&self, format: BlockFormat) -> Result<(), Error> {
        if self.encoders.borrow().contains_key(&format.internal) {
            return Ok(());
        }
        let (mode, source, layout) = match (format.etc, format.kind) {
            (false, CompressKind::Rgb) => ("MODE_BC1", BC_FRAG, self.bc_layout),
            (false, CompressKind::Rgba) => ("MODE_BC3", BC_FRAG, self.bc_layout),
            (false, CompressKind::Rg) => ("MODE_BC5", BC_FRAG, self.bc_layout),
            (true, CompressKind::Rgb) => ("MODE_ETC2_RGB", ETC_FRAG, self.etc_layout),
            (true, CompressKind::Rgba) => ("MODE_ETC2_RGBA", ETC_FRAG, self.etc_layout),
            (true, CompressKind::Rg) => ("MODE_EAC_RG", ETC_FRAG, self.etc_layout),
        };
        let srgb = if format.srgb { "#define SRGB\n" } else { "" };
        let frag = format!("#version 300 es\n#define {mode}\n{srgb}{source}");
        let program = self.ctx.create_program(VERT, &frag)?;
        let params = self.ctx.create_param_group(layout)?;
        let field = |name: &str| self.ctx.param_field(layout, name);
        if !format.etc {
            self.ctx
                .param_set_texture(params, field("u_tables")?, self.tables);
        }
        let encoder = Encoder {
            program,
            params,
            f_source: field("u_source")?,
            f_level: field("u_level")?,
            f_blocks_x: field("u_blocks_x")?,
            f_blocks: field("u_blocks")?,
            f_width: field("u_width")?,
        };
        self.encoders.borrow_mut().insert(format.internal, encoder);
        Ok(())
    }

    // An RGBA32UI target of `size` and a framebuffer over it.
    fn scratch(&self, size: (i32, i32)) -> Result<(FramebufferId, TextureId), Error> {
        let ctx = &self.ctx;
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
        ctx.texture_storage_2d(scratch, 1, &rgba32ui, size.0, size.1);
        let fbo = match ctx.create_framebuffer(size.0, size.1) {
            Ok(fbo) => fbo,
            Err(e) => {
                ctx.destroy_texture(scratch);
                return Err(e);
            }
        };
        ctx.framebuffer_attach_color_texture_2d(fbo, 0, scratch, 0);
        if let Err(e) = ctx.check_framebuffer(fbo) {
            ctx.destroy_framebuffer(fbo);
            ctx.destroy_texture(scratch);
            return Err(e);
        }
        Ok((fbo, scratch))
    }

    // Encodes `level` of `tex` into `fbo` and binds it for reading.
    fn encode(
        &self,
        tex: TextureId,
        format: BlockFormat,
        level: i32,
        l: &Level,
        fbo: FramebufferId,
    ) -> Result<(), Error> {
        let ctx = &self.ctx;
        self.encoder(format)?;
        let encoders = self.encoders.borrow();
        let encoder = &encoders[&format.internal];
        ctx.param_set_texture(encoder.params, encoder.f_source, tex);
        ctx.param_set_i32(encoder.params, encoder.f_level, level);
        ctx.param_set_i32(encoder.params, encoder.f_blocks_x, l.blocks_x);
        ctx.param_set_i32(encoder.params, encoder.f_blocks, l.blocks);
        ctx.param_set_i32(encoder.params, encoder.f_width, l.target.0);
        let mut batch = Batch::new(encoder.program, ctx.render_state(&RenderState::new()));
        batch.set_group(0, encoder.params);
        batch.draw_vertexless(&DrawCommand::arrays(DrawMode::Triangles, 0, 3));
        let mut pass = Pass::new(fbo);
        pass.set_viewport(&Viewport::new(0, 0, l.target.0, l.target.1));
        pass.add(&batch);
        ctx.render(&pass);
        ctx.bind_read_source(
            fbo,
            0,
            crate::readback::Region {
                x: 0,
                y: 0,
                width: l.target.0,
                height: l.target.1,
            },
        )
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
        // Two-channel formats have no sRGB form: a normal map is data anyway.
        let srgb = srgb && kind != CompressKind::Rg;
        let Some(format) = self.format(kind, srgb) else {
            return Ok(false);
        };
        let plan = plan_levels(&format, width, height, levels);
        let size = plan
            .iter()
            .fold((1, 1), |(w, h), l| (w.max(l.target.0), h.max(l.target.1)));
        let bytes = plan
            .last()
            .map_or(0, |l| l.offset + l.target.0 * l.target.1 * 16);

        let (fbo, scratch) = self.scratch(size)?;
        let gl = ctx.gl().clone();
        let pbo = gl.create_buffer().ok_or("createBuffer failed")?;
        gl.bind_buffer(Gl::PIXEL_PACK_BUFFER, Some(&pbo));
        gl.buffer_data_with_i32(Gl::PIXEL_PACK_BUFFER, bytes, Gl::STREAM_COPY);
        gl.bind_buffer(Gl::PIXEL_PACK_BUFFER, None);
        let encoded = (|| -> Result<(), Error> {
            for (i, l) in plan.iter().enumerate() {
                self.encode(tex, format, i as i32, l, fbo)?;
                // The rendered blocks into the buffer.
                gl.bind_buffer(Gl::PIXEL_PACK_BUFFER, Some(&pbo));
                let read = gl.read_pixels_with_i32(
                    0,
                    0,
                    l.target.0,
                    l.target.1,
                    0x8D99, // RGBA_INTEGER
                    Gl::UNSIGNED_INT,
                    l.offset,
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
        for (i, l) in plan.iter().enumerate() {
            gl.compressed_tex_sub_image_2d_with_i32_and_i32(
                Gl::TEXTURE_2D,
                i as i32,
                0,
                0,
                l.width,
                l.height,
                format.internal,
                l.blocks * format.block_bytes,
                l.offset,
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
