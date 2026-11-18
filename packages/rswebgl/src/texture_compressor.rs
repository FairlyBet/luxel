//! Compress a texture on the GPU, after it was uploaded uncompressed. On
//! desktops BC (S3TC / RGTC): BC1 for opaque color, BC3 with alpha, BC5 for
//! two channels (normal maps). Where those are missing — phones — ETC2 /
//! EAC: ETC2 RGB, ETC2 RGBA, RG11. ASTC 4×4 where a device has neither
//! (see `preference`). HDR (half-float) textures and cube maps — the
//! environment's — BC6H where the device has BPTC. Encoded by fragment shaders
//! (`compress/bc.frag`, `compress/etc.frag`, `compress/astc.frag`).
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
use std::rc::Rc;

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
const ASTC_FRAG: &str = include_str!("compress/astc.frag");
const BC6H_FRAG: &str = include_str!("compress/bc6h.frag");
const VERT: &str = "#version 300 es
void main() {
    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2)) * 2.0 - 1.0;
    gl_Position = vec4(p, 0.0, 1.0);
}
";

const RGBA8: u32 = Gl::RGBA8;
const SRGB8_ALPHA8: u32 = Gl::SRGB8_ALPHA8;
// Float formats BC6H reads.
const R11F_G11F_B10F: u32 = Gl::R11F_G11F_B10F;
const RGBA16F: u32 = Gl::RGBA16F;
const RGBA32F: u32 = Gl::RGBA32F;
const MAX_ANISOTROPY: u32 = crate::sampler::TEXTURE_MAX_ANISOTROPY_EXT;
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
    /// HDR color (an R11F_G11F_B10F, RGBA16F or RGBA32F source, 2D or a
    /// cube map): BC6H, 8 bits a texel. Needs `EXT_texture_compression_bptc`.
    Hdr = 3,
}

// A compressed format and how it is encoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BlockFormat {
    internal: u32,
    block_bytes: i32,
    kind: CompressKind,
    srgb: bool,
    family: Family,
}

impl BlockFormat {
    // The format the encoders write as `internal` for `kind`, if any.
    fn written(internal: u32, kind: CompressKind) -> Option<Self> {
        FORMATS
            .iter()
            .find(|f| f.internal == internal && f.kind == kind)
            .copied()
    }

    // Its encoder's key (for a cube map's faces or a 2D texture).
    fn key(&self, cube: bool) -> EncoderKey {
        (self.internal, self.kind, cube)
    }
}

type EncoderKey = (u32, CompressKind, bool);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Family {
    Bc,
    Etc,
    Astc,
}

// Every format the encoders write (BC first: preferred where both exist).
const FORMATS: [BlockFormat; 16] = {
    const fn f(
        internal: u32,
        bytes: i32,
        kind: CompressKind,
        srgb: bool,
        family: Family,
    ) -> BlockFormat {
        BlockFormat {
            internal,
            block_bytes: bytes,
            kind,
            srgb,
            family,
        }
    }
    use CompressKind::*;
    use Family::*;
    [
        f(0x83F0, 8, Rgb, false, Bc),     // RGB_S3TC_DXT1
        f(0x8C4C, 8, Rgb, true, Bc),      // SRGB_S3TC_DXT1
        f(0x83F3, 16, Rgba, false, Bc),   // RGBA_S3TC_DXT5
        f(0x8C4F, 16, Rgba, true, Bc),    // SRGB_ALPHA_S3TC_DXT5
        f(0x8DBD, 16, Rg, false, Bc),     // RG_RGTC2
        f(0x9274, 8, Rgb, false, Etc),    // RGB8_ETC2
        f(0x9275, 8, Rgb, true, Etc),     // SRGB8_ETC2
        f(0x9278, 16, Rgba, false, Etc),  // RGBA8_ETC2_EAC
        f(0x9279, 16, Rgba, true, Etc),   // SRGB8_ALPHA8_ETC2_EAC
        f(0x9272, 16, Rg, false, Etc),    // RG11_EAC
        f(0x93B0, 16, Rgb, false, Astc),  // RGBA_ASTC_4x4
        f(0x93D0, 16, Rgb, true, Astc),   // SRGB8_ALPHA8_ASTC_4x4
        f(0x93B0, 16, Rgba, false, Astc), // RGBA_ASTC_4x4
        f(0x93D0, 16, Rgba, true, Astc),  // SRGB8_ALPHA8_ASTC_4x4
        f(0x93B0, 16, Rg, false, Astc),   // RGBA_ASTC_4x4
        f(0x8E8F, 16, Hdr, false, Bc),    // RGB_BPTC_UNSIGNED_FLOAT
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
    f_face: Option<ParamField>,
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
    // By the format's GL enum and kind (ASTC writes all three into one).
    encoders: RefCell<HashMap<EncoderKey, Encoder>>,
    cube_layout: ParamLayoutId,
    // Exact texel reads at an explicit level from a cube.
    cube_sampler: crate::ids::SamplerId,
    s3tc: bool,
    s3tc_srgb: bool,
    rgtc: bool,
    etc: bool,
    astc: bool,
    bptc: bool,
}

#[wasm_bindgen]
impl TextureCompressor {
    /// Enables the compressed-texture extensions it uses (S3TC, S3TC sRGB,
    /// RGTC, BPTC, ETC, ASTC) where the device has them and starts compiling its
    /// encoders.
    #[wasm_bindgen(constructor)]
    pub fn new(ctx: &Context) -> Result<TextureCompressor, Error> {
        let s3tc = ctx.enable_extension_quietly(Extension::WebglCompressedTextureS3tc);
        let s3tc_srgb = ctx.enable_extension_quietly(Extension::WebglCompressedTextureS3tcSrgb);
        let rgtc = ctx.enable_extension_quietly(Extension::ExtTextureCompressionRgtc);
        let etc = ctx.enable_extension_quietly(Extension::WebglCompressedTextureEtc);
        let astc = ctx.enable_extension_quietly(Extension::WebglCompressedTextureAstc);
        let bptc = ctx.enable_extension_quietly(Extension::ExtTextureCompressionBptc);
        let layout = |tables: bool, face: bool| {
            let mut b = ParamLayoutBuilder::new().texture("u_source");
            if tables {
                b = b.texture("u_tables");
            }
            if face {
                b = b.uniform("u_face", UniformType::I32);
            }
            ctx.create_param_layout(
                &b.uniform("u_level", UniformType::I32)
                    .uniform("u_blocks_x", UniformType::I32)
                    .uniform("u_blocks", UniformType::I32)
                    .uniform("u_width", UniformType::I32),
            )
        };
        let bc_layout = layout(true, false)?;
        let etc_layout = layout(false, false)?;
        let cube_layout = layout(false, true)?;
        let mut nearest = crate::sampler::SamplerDesc::nearest_clamp();
        nearest.min_filter = TextureMinFilter::NearestMipmapNearest;
        let cube_sampler = ctx.create_sampler(&nearest)?;
        let tables = ctx.create_texture(
            TextureTarget::Texture2D,
            TextureMinFilter::Nearest,
            TextureMagFilter::Nearest,
        )?;
        let rg8ui = TextureFormat {
            internal: Gl::RG8UI as i32,
            format: Gl::RG_INTEGER,
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
            cube_layout,
            cube_sampler,
            s3tc,
            s3tc_srgb,
            rgtc,
            etc,
            astc,
            bptc,
        };
        for kind in [CompressKind::Rgb, CompressKind::Rgba, CompressKind::Rg] {
            for srgb in [false, true] {
                if let Some(format) = me.format(kind, srgb) {
                    me.encoder(format, false)?;
                }
            }
        }
        // HDR: the environment's cubes.
        if let Some(format) = me.format(CompressKind::Hdr, false) {
            me.encoder(format, true)?;
        }
        Ok(me)
    }

    /// Whether this device can compress `kind` (its extension is there).
    pub fn supports(&self, kind: CompressKind) -> bool {
        self.format(kind, false).is_some()
    }

    /// The format `kind` compresses to here ("BC1", "ETC2 RGB", …), or
    /// none.
    #[wasm_bindgen(js_name = formatName)]
    pub fn format_name(&self, kind: CompressKind) -> Option<String> {
        self.format(kind, false).map(|f| {
            match (f.family, f.kind) {
                (Family::Bc, CompressKind::Rgb) => "BC1",
                (Family::Bc, CompressKind::Rgba) => "BC3",
                (Family::Bc, CompressKind::Rg) => "BC5",
                (Family::Etc, CompressKind::Rgb) => "ETC2 RGB",
                (Family::Etc, CompressKind::Rgba) => "ETC2 RGBA",
                (Family::Etc, CompressKind::Rg) => "RG11 EAC",
                (Family::Astc, _) => "ASTC 4x4",
                (_, CompressKind::Hdr) => "BC6H",
            }
            .to_string()
        })
    }

    /// Resolves once the encoders have compiled; `compress` before then
    /// waits for them (blocking).
    #[wasm_bindgen(js_name = whenReady)]
    #[wasm_bindgen(unchecked_return_type = "Promise<void>")]
    pub fn when_ready(&self) -> js_sys::Promise {
        let programs = self.encoders.borrow().values().map(|e| e.program).collect();
        let ready = self.ctx.programs_ready(programs);
        wasm_bindgen_futures::future_to_promise(async move {
            ready.await;
            Ok(JsValue::UNDEFINED)
        })
    }

    /// Compresses `tex`, its levels filled (mips made), in place: the same
    /// handle then samples the compressed texture, with its sampler
    /// settings. Takes a 2D RGBA8 or SRGB8_ALPHA8 texture (`Rgb`, `Rgba`,
    /// `Rg`), or a 2D or cube R11F_G11F_B10F / RGBA16F / RGBA32F one
    /// (`Hdr`). False (and the texture left as it was) where it can't be: no
    /// format for `kind` here, the texture isn't one of those, its size
    /// isn't a multiple of 4 (whole blocks at level 0), or the context is
    /// lost.
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

    /// Makes the encoder for `internal` / `kind` (as `encode_level`
    /// would, compiling in the background) and resolves once it has
    /// compiled. For tests.
    #[doc(hidden)]
    #[wasm_bindgen(js_name = prepareFormat)]
    #[wasm_bindgen(unchecked_return_type = "Promise<void>")]
    pub fn prepare_format(
        &self,
        internal: u32,
        kind: CompressKind,
    ) -> Result<js_sys::Promise, Error> {
        let format = BlockFormat::written(internal, kind)
            .ok_or("prepareFormat: not a format the encoders write")?;
        self.encoder(format, false)?;
        let program = self.encoders.borrow()[&format.key(false)].program;
        let ready = self.ctx.programs_ready(vec![program]);
        Ok(wasm_bindgen_futures::future_to_promise(async move {
            ready.await;
            Ok(JsValue::UNDEFINED)
        }))
    }

    /// Level `level` of `tex` encoded as `internal` for `kind` (any format
    /// the encoders write, whether or not the device samples it), its
    /// blocks' bytes. For tests: a format the device lacks can still be
    /// decoded on the CPU.
    #[doc(hidden)]
    #[wasm_bindgen(js_name = encodeLevel)]
    pub fn encode_level(
        &self,
        tex: TextureId,
        internal: u32,
        kind: CompressKind,
        level: i32,
    ) -> Result<Vec<u8>, Error> {
        let format = BlockFormat::written(internal, kind)
            .ok_or("encodeLevel: not a format the encoders write")?;
        let extent = self
            .ctx
            .res()
            .textures
            .get(tex.0)
            .map(|t| t.extent.get())
            .ok_or("encodeLevel: the texture is destroyed")?;
        let plan = plan_levels(&format, extent[1], extent[2], level + 1, 1);
        let l = plan.last().ok_or("encodeLevel: no such level")?;
        let (fbo, scratch) = self.scratch(l.target)?;
        let out = (|| {
            self.encode(tex, format, false, l, fbo)?;
            let gl = self.ctx.gl();
            let mut words = vec![0u32; (l.target.0 * l.target.1 * 4) as usize];
            // SAFETY: the view lives only for the call.
            let view = unsafe { js_sys::Uint32Array::view(&words) };
            gl.read_pixels_with_opt_array_buffer_view(
                0,
                0,
                l.target.0,
                l.target.1,
                Gl::RGBA_INTEGER,
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
            res.param_layouts.contains(self.cube_layout.0),
            res.samplers.contains(self.cube_sampler.0),
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
        if live.3 {
            self.ctx.destroy_param_layout(self.cube_layout);
        }
        if live.4 {
            self.ctx.destroy_sampler(self.cube_sampler);
        }
    }
}

thread_local! {
    // One compressor per context, shared by whoever compresses on it (its
    // encoders compile once), made again after a context loss.
    static SHARED: RefCell<Option<(Context, u32, Rc<TextureCompressor>)>> =
        const { RefCell::new(None) };
}

impl TextureCompressor {
    /// The compressor of `ctx`, made on first use and kept for the
    /// context's life (a new one after a loss).
    pub fn shared(ctx: &Context) -> Result<Rc<TextureCompressor>, Error> {
        SHARED.with(|slot| {
            let mut slot = slot.borrow_mut();
            if let Some((c, generation, comp)) = slot.as_ref()
                && c.is_same_context(ctx)
                && *generation == ctx.generation()
            {
                return Ok(comp.clone());
            }
            let comp = Rc::new(TextureCompressor::new(ctx)?);
            *slot = Some((ctx.clone(), ctx.generation(), comp.clone()));
            Ok(comp)
        })
    }

    /// Lets go of `ctx`'s shared compressor (its encoders, and the context
    /// it holds): when what compressed on it is torn down. The next
    /// `shared` makes one again.
    pub fn release_shared(ctx: &Context) {
        let taken = SHARED.with(|slot| {
            let mut slot = slot.borrow_mut();
            match slot.as_ref() {
                Some((c, _, _)) if c.is_same_context(ctx) => slot.take(),
                _ => None,
            }
        });
        // Dropped outside the borrow: its destroy may call back into shared().
        drop(taken);
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
    level: i32,
    face: i32,
    width: i32,
    height: i32,
    blocks_x: i32,
    blocks: i32,
    target: (i32, i32),
    offset: i32,
}

fn plan_levels(
    format: &BlockFormat,
    width: i32,
    height: i32,
    levels: i32,
    faces: i32,
) -> Vec<Level> {
    let per_texel = 16 / format.block_bytes; // blocks an RGBA32UI texel holds
    let mut plan = Vec::new();
    let mut offset = 0;
    for (l, face) in (0..levels).flat_map(|l| (0..faces).map(move |f| (l, f))) {
        let (w, h) = ((width >> l).max(1), (height >> l).max(1));
        let (bx, by) = ((w + 3) / 4, (h + 3) / 4);
        let blocks = bx * by;
        let texels = (blocks + per_texel - 1) / per_texel;
        let tw = texels.min(TARGET_WIDTH);
        let th = (texels + tw - 1) / tw;
        plan.push(Level {
            level: l,
            face,
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
    // ETC2 / EAC, else ASTC (`Self::FAMILY_ORDER`).
    fn format(&self, kind: CompressKind, srgb: bool) -> Option<BlockFormat> {
        Self::FAMILY_ORDER.into_iter().find_map(|family| {
            FORMATS
                .iter()
                .copied()
                .find(|f| f.kind == kind && f.srgb == srgb && f.family == family && self.has(f))
        })
    }

    // Families in the order every kind takes them. ASTC last: this encoder
    // (one partition, one weight plane) loses to ETC2 / EAC of the same
    // size on busy blocks — measured mean error on a block of independent
    // waves: RG 28.6 vs RG11 5.2, RGBA 34.7 vs 26.5 — and its RGB is twice
    // ETC2 RGB's size for little gain (29.9 vs 33.4). Those ETC numbers
    // predate the ETC1 selector fix, which only made ETC better. It is for
    // devices with neither BC nor ETC.
    const FAMILY_ORDER: [Family; 3] = [Family::Bc, Family::Etc, Family::Astc];

    // Whether the device samples `f`.
    fn has(&self, f: &BlockFormat) -> bool {
        match (f.family, f.kind, f.srgb) {
            (Family::Etc, _, _) => self.etc,
            (Family::Astc, _, _) => self.astc,
            (Family::Bc, CompressKind::Rg, _) => self.rgtc,
            (Family::Bc, CompressKind::Hdr, _) => self.bptc,
            (Family::Bc, _, false) => self.s3tc,
            (Family::Bc, _, true) => self.s3tc_srgb,
        }
    }

    // The encoder for `format` (made on first use).
    fn encoder(&self, format: BlockFormat, cube: bool) -> Result<(), Error> {
        if self.encoders.borrow().contains_key(&format.key(cube)) {
            return Ok(());
        }
        let (mode, source, layout) = match (format.family, format.kind) {
            (Family::Bc, CompressKind::Rgb) => ("MODE_BC1", BC_FRAG, self.bc_layout),
            (Family::Bc, CompressKind::Rgba) => ("MODE_BC3", BC_FRAG, self.bc_layout),
            (Family::Bc, CompressKind::Rg) => ("MODE_BC5", BC_FRAG, self.bc_layout),
            (Family::Etc, CompressKind::Rgb) => ("MODE_ETC2_RGB", ETC_FRAG, self.etc_layout),
            (Family::Etc, CompressKind::Rgba) => ("MODE_ETC2_RGBA", ETC_FRAG, self.etc_layout),
            (Family::Etc, CompressKind::Rg) => ("MODE_EAC_RG", ETC_FRAG, self.etc_layout),
            // Same uniforms as ETC's (no tables).
            (Family::Astc, CompressKind::Rgb) => ("MODE_ASTC_RGB", ASTC_FRAG, self.etc_layout),
            (Family::Astc, CompressKind::Rgba) => ("MODE_ASTC_RGBA", ASTC_FRAG, self.etc_layout),
            (Family::Astc, CompressKind::Rg) => ("MODE_ASTC_RG", ASTC_FRAG, self.etc_layout),
            (_, CompressKind::Hdr) if cube => ("CUBE", BC6H_FRAG, self.cube_layout),
            (_, CompressKind::Hdr) => ("BC6H", BC6H_FRAG, self.etc_layout),
        };
        let srgb = if format.srgb { "#define SRGB\n" } else { "" };
        let frag = format!("#version 300 es\n#define {mode}\n{srgb}{source}");
        let program = self.ctx.create_program(VERT, &frag)?;
        let params = self.ctx.create_param_group(layout)?;
        let field = |name: &str| self.ctx.param_field(layout, name);
        if format.family == Family::Bc && format.kind != CompressKind::Hdr {
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
            f_face: if cube { Some(field("u_face")?) } else { None },
        };
        if cube {
            self.ctx
                .param_set_sampler(params, encoder.f_source, self.cube_sampler);
        }
        self.encoders.borrow_mut().insert(format.key(cube), encoder);
        Ok(())
    }

    // An RGBA32UI target of `size` and a framebuffer over it.
    fn scratch(&self, size: (i32, i32)) -> Result<(FramebufferId, TextureId), Error> {
        let ctx = &self.ctx;
        let rgba32ui = TextureFormat {
            internal: Gl::RGBA32UI as i32,
            format: Gl::RGBA_INTEGER,
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
        cube: bool,
        l: &Level,
        fbo: FramebufferId,
    ) -> Result<(), Error> {
        let ctx = &self.ctx;
        self.encoder(format, cube)?;
        let encoders = self.encoders.borrow();
        let encoder = &encoders[&format.key(cube)];
        // A failed encoder would draw nothing: all-zero blocks (black, or
        // transparent black) swapped in as if compressed.
        if !ctx.program_linked(encoder.program) {
            return Err("a texture encoder shader failed to link".into());
        }
        ctx.param_set_texture(encoder.params, encoder.f_source, tex);
        ctx.param_set_i32(encoder.params, encoder.f_level, l.level);
        if let Some(f) = encoder.f_face {
            ctx.param_set_i32(encoder.params, f, l.face);
        }
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
                .map(|t| (t.target, t.internal.get(), t.extent.get()))
        }) else {
            return Err("compress: the texture is destroyed".into());
        };
        let srgb = match (internal, kind) {
            (RGBA8, CompressKind::Rgb | CompressKind::Rgba | CompressKind::Rg) => false,
            (SRGB8_ALPHA8, CompressKind::Rgb | CompressKind::Rgba | CompressKind::Rg) => true,
            (R11F_G11F_B10F | RGBA16F | RGBA32F, CompressKind::Hdr) => false,
            _ => return Ok(false),
        };
        let cube = target == TextureTarget::TextureCubeMap;
        if !(cube && kind == CompressKind::Hdr || target == TextureTarget::Texture2D)
            || width % 4 != 0
            || height % 4 != 0
            || levels < 1
        {
            return Ok(false);
        }
        let (bind_target, faces) = if cube {
            (Gl::TEXTURE_CUBE_MAP, 6)
        } else {
            (Gl::TEXTURE_2D, 1)
        };
        // Two-channel formats have no sRGB form: a normal map is data anyway.
        let srgb = srgb && kind != CompressKind::Rg;
        let Some(format) = self.format(kind, srgb) else {
            return Ok(false);
        };
        let plan = plan_levels(&format, width, height, levels, faces);
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
            for l in &plan {
                self.encode(tex, format, cube, l, fbo)?;
                // The rendered blocks into the buffer.
                gl.bind_buffer(Gl::PIXEL_PACK_BUFFER, Some(&pbo));
                let read = gl.read_pixels_with_i32(
                    0,
                    0,
                    l.target.0,
                    l.target.1,
                    Gl::RGBA_INTEGER,
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
        gl.bind_texture(bind_target, Some(&raw));
        gl.tex_storage_2d(bind_target, levels, format.internal, width, height);
        gl.bind_buffer(Gl::PIXEL_UNPACK_BUFFER, Some(&pbo));
        for l in &plan {
            let image_target = if cube {
                Gl::TEXTURE_CUBE_MAP_POSITIVE_X + l.face as u32
            } else {
                Gl::TEXTURE_2D
            };
            gl.compressed_tex_sub_image_2d_with_i32_and_i32(
                image_target,
                l.level,
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
                    gl.tex_parameterf(bind_target, pname, v as f32);
                }
            } else if let Some(v) = value.as_f64() {
                gl.tex_parameteri(bind_target, pname, v as i32);
            }
        }
        gl.bind_texture(bind_target, None);
        // Freed once the uploads that read it have run.
        gl.delete_buffer(Some(&pbo));
        ctx.replace_texture_storage(tex, raw, format.internal, [levels, width, height, 1]);
        Ok(true)
    }
}
