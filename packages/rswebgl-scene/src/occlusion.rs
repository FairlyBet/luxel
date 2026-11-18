//! Screen-space occlusion over the opaque scene's depth, as a post-pass
//! (three.js' GTAOPass): ambient occlusion (GTAO) and one light's contact
//! shadows (Filament's), computed together at half resolution, smoothed
//! with a depth-aware blur, multiplied into the opaque scene before TAA,
//! glass and transparent meshes. It darkens the whole lit image there —
//! direct light and reflections too — where a depth prepass would let each
//! darken only its own light (TODO.md: for later).

use glam::Mat4;
use rswebgl::context::Context;
use rswebgl::error::Error;
use rswebgl::ids::{
    FramebufferId, ParamGroupId, ParamLayoutId, ProgramId, StateId, TextureId,
};
use rswebgl::param::ParamLayoutBuilder;
use rswebgl::render_state::{BlendFactor, RenderState};
use rswebgl::shader_source::ShaderSource;
use rswebgl::texture::{
    TextureFormat, TextureMagFilter, TextureMinFilter, TextureTarget, TextureWrap,
};
use rswebgl::uniform_buffer::UniformType;


const GTAO_FRAG: &str = include_str!("../shaders/gtao.frag");
const DENOISE_FRAG: &str = include_str!("../shaders/gtao_denoise.frag");
const APPLY_FRAG: &str = include_str!("../shaders/gtao_apply.frag");

/// How the ambient occlusion looks. Lengths in world units (view space).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AmbientOcclusion {
    /// How far around a point occluders count (0: a twentieth of the
    /// scene's size — its shadow casters' box).
    pub radius: f32,
    /// How deep an occluder may be in front of a point and still count
    /// (0: four radii); thin things in front of a far wall don't darken it.
    pub thickness: f32,
    /// How much of the darkening shows (0..1).
    pub intensity: f32,
    /// The visibility raised to this (above 1: darker creases).
    pub power: f32,
    /// Computed at half the target's resolution (cheaper, softer).
    pub half_resolution: bool,
}

impl Default for AmbientOcclusion {
    fn default() -> Self {
        Self {
            radius: 0.0,
            thickness: 0.0,
            intensity: 1.0,
            power: 1.0,
            half_resolution: true,
        }
    }
}

/// One light's contact shadows: what its shadow map is too coarse to show
/// (a cup on a table, a ring's prongs) found by a short ray toward it
/// through the scene's depth. Lengths in world units (view space).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContactShadows {
    /// How far toward the light the ray looks (0: a fiftieth of the
    /// scene's size).
    pub length: f32,
    /// How deep a surface in front of the ray may be and still shadow it
    /// (0: a quarter of the length).
    pub thickness: f32,
    /// How much of the shadow shows (0..1).
    pub intensity: f32,
    /// Steps along the ray (1..64).
    pub steps: u32,
}

impl Default for ContactShadows {
    fn default() -> Self {
        Self {
            length: 0.0,
            thickness: 0.0,
            intensity: 1.0,
            steps: 16,
        }
    }
}

/// The occlusion's targets and passes.
pub(crate) struct Occlusion {
    gtao: (ProgramId, ParamLayoutId, ParamGroupId),
    denoise: (ProgramId, ParamLayoutId, ParamGroupId),
    apply: (ProgramId, ParamLayoutId, ParamGroupId),
    plain: StateId,
    multiply: StateId,
    // The raw and smoothed visibility, and their size.
    raw: (TextureId, FramebufferId),
    smooth: (TextureId, FramebufferId),
    size: (i32, i32),
    generation: u32,
}

impl Occlusion {
    pub fn new(ctx: &Context) -> Result<Self, Error> {
        let mut undo = rswebgl::undo::Undo::new(ctx);
        let prefix = "#version 300 es\n";
        let vert = crate::fullscreen::vertex(prefix);
        let mut make = |name: &str, frag: &str, layout: ParamLayoutBuilder| {
            let f = ShaderSource::plain(name, &format!("{prefix}{frag}"));
            let program = ctx.create_program_from(&vert, &f)?;
            undo.push(move |c| c.destroy_program(program));
            let layout = ctx.create_param_layout(&layout)?;
            undo.push(move |c| c.destroy_param_layout(layout));
            let group = ctx.create_param_group(layout)?;
            undo.push(move |c| c.destroy_param_group(group));
            Ok::<_, Error>((program, layout, group))
        };
        let gtao = make(
            "gtao.frag",
            GTAO_FRAG,
            ParamLayoutBuilder::new()
                .texture("u_depth")
                .uniform("u_proj", UniformType::Mat4)
                .uniform("u_inv_proj", UniformType::Mat4)
                .uniform("u_gtao", UniformType::Vec4)
                .uniform("u_gtao_frame", UniformType::Vec4)
                .uniform("u_contact", UniformType::Vec4)
                .uniform("u_contact_steps", UniformType::Vec4),
        )?;
        let denoise = make(
            "gtao_denoise.frag",
            DENOISE_FRAG,
            ParamLayoutBuilder::new()
                .texture("u_ao")
                .texture("u_depth")
                .uniform("u_denoise", UniformType::Vec4),
        )?;
        let apply = make(
            "gtao_apply.frag",
            APPLY_FRAG,
            ParamLayoutBuilder::new()
                .texture("u_ao")
                .texture("u_key")
                .uniform("u_apply", UniformType::Vec4),
        )?;
        let mut plain = RenderState::new();
        plain.depth_test = false;
        plain.depth_mask = false;
        // color × source color; alpha (coverage) kept.
        let mut multiply = plain.clone();
        multiply.blend = true;
        multiply.blend_src_rgb = BlendFactor::Zero;
        multiply.blend_dst_rgb = BlendFactor::SrcColor;
        multiply.blend_src_alpha = BlendFactor::Zero;
        multiply.blend_dst_alpha = BlendFactor::One;
        let occlusion = Self {
            gtao,
            denoise,
            apply,
            plain: ctx.render_state(&plain),
            multiply: ctx.render_state(&multiply),
            raw: (TextureId::NULL, FramebufferId::NULL),
            smooth: (TextureId::NULL, FramebufferId::NULL),
            size: (0, 0),
            generation: ctx.generation(),
        };
        undo.keep();
        Ok(occlusion)
    }

    /// Whether its GPU objects belong to an earlier context generation.
    pub fn is_lost(&self, ctx: &Context) -> bool {
        self.generation != ctx.generation()
    }

    /// Computes the occlusion of the opaque scene whose depth is `depth`
    /// (`size`, drawn with projection `proj`) and multiplies it into
    /// `target`: ambient occlusion with `ao`, a light's contact shadows
    /// with `contact` (and its direction toward the light, view space).
    /// `scene_size`: the scene's (for the auto lengths). `frame` moves the
    /// noise.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        ctx: &Context,
        ao: Option<&AmbientOcclusion>,
        // With the key light in view space: its position (w 1) or the
        // direction toward it (w 0).
        contact: Option<(&ContactShadows, glam::Vec4)>,
        depth: TextureId,
        target: FramebufferId,
        key: TextureId,
        size: (i32, i32),
        proj: &Mat4,
        scene_size: f32,
        frame: u32,
    ) {
        if ao.is_none() && contact.is_none() {
            return;
        }
        let settings = ao.copied().unwrap_or_default();
        let (w, h) = size;
        let scale = if settings.half_resolution { 2 } else { 1 };
        let ao_size = (
            ((w + scale - 1) / scale).max(1),
            ((h + scale - 1) / scale).max(1),
        );
        if (self.size != ao_size || self.raw.0.is_null())
            && let Err(e) = self.allocate(ctx, ao_size)
        {
            rswebgl::console::error_once(&format!("[rswebgl-scene] ambient occlusion: {e}"));
            return;
        }
        let reverse = if ctx.reverse_z() { 1.0 } else { 0.0 };
        let radius = if settings.radius > 0.0 {
            settings.radius
        } else {
            scene_size / 20.0
        };
        let thickness = if settings.thickness > 0.0 {
            settings.thickness
        } else {
            radius * 4.0
        };

        let (program, _, g) = self.gtao;
        ctx.param_set_texture_by_name(g, "u_depth", depth);
        ctx.param_set_floats_by_name(g, "u_proj", &proj.to_cols_array());
        ctx.param_set_floats_by_name(g, "u_inv_proj", &proj.inverse().to_cols_array());
        ctx.param_set_floats_by_name(g, "u_gtao", &[radius, thickness, 1.0, 1.0]);
        let with_ao = if ao.is_some() { 1.0 } else { 0.0 };
        ctx.param_set_floats_by_name(
            g,
            "u_gtao_frame",
            &[settings.power, reverse, (frame % 64) as f32, with_ao],
        );
        let (c, toward) = match contact {
            Some((c, l)) if l.w > 0.5 => (*c, l),
            Some((c, l)) => (*c, l.truncate().normalize_or_zero().extend(0.0)),
            None => (ContactShadows::default(), glam::Vec4::Z),
        };
        let length = match contact {
            None => 0.0,
            Some(_) if c.length > 0.0 => c.length,
            Some(_) => scene_size / 50.0,
        };
        let thickness = if c.thickness > 0.0 {
            c.thickness
        } else {
            length * 0.25
        };
        ctx.param_set_floats_by_name(g, "u_contact", &[toward.x, toward.y, toward.z, length]);
        ctx.param_set_floats_by_name(
            g,
            "u_contact_steps",
            &[thickness, c.steps.clamp(1, 64) as f32, toward.w, 0.0],
        );
        self.fullscreen(ctx, program, g, self.plain, self.raw.1, ao_size);

        let (program, _, g) = self.denoise;
        ctx.param_set_texture_by_name(g, "u_ao", self.raw.0);
        ctx.param_set_texture_by_name(g, "u_depth", depth);
        ctx.param_set_floats_by_name(g, "u_denoise", &[reverse, 0.0, 0.0, 0.0]);
        self.fullscreen(ctx, program, g, self.plain, self.smooth.1, ao_size);

        let (program, _, g) = self.apply;
        ctx.param_set_texture_by_name(g, "u_ao", self.smooth.0);
        // The key light's share (contact shadows dim it alone); without
        // one, all of the pixel.
        let has_key = if key.is_null() { 0.0 } else { 1.0 };
        ctx.param_set_texture_by_name(g, "u_key", if key.is_null() { self.smooth.0 } else { key });
        let ao_intensity = ao.map_or(0.0, |a| a.intensity);
        let contact_intensity = contact.map_or(0.0, |(c, _)| c.intensity);
        ctx.param_set_floats_by_name(
            g,
            "u_apply",
            &[ao_intensity, contact_intensity, has_key, 0.0],
        );
        self.fullscreen(ctx, program, g, self.multiply, target, size);
    }

    fn fullscreen(
        &self,
        ctx: &Context,
        program: ProgramId,
        group: ParamGroupId,
        state: StateId,
        fb: FramebufferId,
        (w, h): (i32, i32),
    ) {
        ctx.render(&crate::fullscreen::pass(fb, (w, h), program, state, group));
    }

    fn allocate(&mut self, ctx: &Context, (w, h): (i32, i32)) -> Result<(), Error> {
        self.free_targets(ctx);
        for slot in [&mut self.raw, &mut self.smooth] {
            let t = ctx.create_texture(
                TextureTarget::Texture2D,
                TextureMinFilter::Linear,
                TextureMagFilter::Linear,
            )?;
            ctx.texture_storage_2d(t, 1, &TextureFormat::rg8(), w, h);
            ctx.texture_set_wrap_s(t, TextureWrap::ClampToEdge);
            ctx.texture_set_wrap_t(t, TextureWrap::ClampToEdge);
            let fb = match ctx.create_framebuffer(w, h) {
                Ok(fb) => fb,
                Err(e) => {
                    ctx.destroy_texture(t);
                    return Err(e);
                }
            };
            ctx.framebuffer_attach_color_texture_2d(fb, 0, t, 0);
            *slot = (t, fb);
        }
        self.size = (w, h);
        Ok(())
    }

    fn free_targets(&mut self, ctx: &Context) {
        for (t, fb) in [self.raw, self.smooth] {
            if !fb.is_null() {
                ctx.destroy_framebuffer(fb);
            }
            if !t.is_null() {
                ctx.destroy_texture(t);
            }
        }
        self.raw = (TextureId::NULL, FramebufferId::NULL);
        self.smooth = (TextureId::NULL, FramebufferId::NULL);
        self.size = (0, 0);
    }

    /// Frees it all (after a context loss, only the CPU-side groups).
    pub fn destroy(mut self, ctx: &Context) {
        let lost = self.is_lost(ctx);
        for (program, layout, group) in [self.gtao, self.denoise, self.apply] {
            ctx.destroy_param_group(group);
            ctx.destroy_param_layout(layout);
            if !lost {
                ctx.destroy_program(program);
            }
        }
        if !lost {
            self.free_targets(ctx);
        }
    }
}
