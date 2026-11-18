//! HDR output: the scene renders linear radiance into a half-float target
//! (multisampled when the device allows), and a display pass turns it into
//! the canvas's colors — exposure, tone mapping, sRGB, dither.
//!
//! Shading in linear HDR and finishing once at the end is what keeps
//! blending physically right (transparent surfaces mix radiance, not
//! tone-mapped colors), leaves the tone mapper a free choice, and gives later
//! passes (screen-space refraction, post effects) the scene as light.
//!
//! The target holds premultiplied radiance and coverage, cleared to the
//! canvas's clear color, so a canvas with a transparent clear color still
//! composites over the page.

use glam::Vec4;
use luxel_gl::context::Context;
use luxel_gl::draw::Viewport;
use luxel_gl::error::Error;
use luxel_gl::framebuffer::{ClearMask, InvalidateMask};
use luxel_gl::ids::{
    FramebufferId, ParamGroupId, ParamLayoutId, ProgramId, RenderbufferId, StateId, TextureId,
};
use luxel_gl::param::{ParamField, ParamLayoutBuilder};
use luxel_gl::pass::Pass;
use luxel_gl::profiler::GpuProfiler;
use luxel_gl::render_state::RenderState;
use luxel_gl::renderbuffer::RenderbufferFormat;
use luxel_gl::shader_source::ShaderSource;
use luxel_gl::texture::{
    TextureFormat, TextureMagFilter, TextureMinFilter, TextureTarget, TextureWrap,
};
use luxel_gl::uniform_buffer::UniformType;

use crate::shader::chunk;

const PRESENT_FRAG: &str = include_str!("../shaders/present.frag");
const TAA_FRAG: &str = include_str!("../shaders/taa.frag");
/// The resolved history back into the target: its rgb, and the resolved
/// coverage (the motion target's z) as alpha.
const TAA_COPY_FRAG: &str = "precision highp float;
precision highp sampler2D;
out vec4 frag_color;
uniform sampler2D u_source;
uniform sampler2D u_coverage;
void main() {
    ivec2 p = ivec2(gl_FragCoord.xy);
    frag_color = vec4(texelFetch(u_source, p, 0).rgb, texelFetch(u_coverage, p, 0).z);
}
";
const FXAA_FRAG: &str = include_str!("../shaders/fxaa.frag");
const GRAB_DOWN_FRAG: &str = include_str!("../shaders/grab_down.frag");
/// The velocity target's clear value: no motion vector here (taa.frag
/// reprojects the depth instead).
const VELOCITY_NONE: f32 = 1000.0;

/// Temporal antialiasing (ported from Bevy, see taa.frag): the opaque scene
/// renders jittered without MSAA, its depth into a texture; each frame
/// resolves into the next of two history textures, copied back into the
/// target for transmissive and transparent meshes to draw over, unjittered.
struct Taa {
    program: ProgramId,
    layout: ParamLayoutId,
    group: ParamGroupId,
    f_reproject: ParamField,
    f_taa: ParamField,
    f_jitter: ParamField,
    f_auto_grey: ParamField,
    // The history → target copy (its color, the target's coverage kept).
    copy: (ProgramId, ParamLayoutId, ParamGroupId),
    copy_state: StateId,
    // Sized with the target: (texture, framebuffer) each.
    history: [(TextureId, FramebufferId); 2],
    // The motion each history's resolve used, per pixel (its second color
    // attachment): the next resolve's velocity disocclusion.
    motions: [TextureId; 2],
    depth: TextureId,
    // Moving meshes' motion (uv now − uv last frame; elsewhere the clear
    // value, VELOCITY_NONE).
    velocity: (TextureId, FramebufferId),
    // The history the last resolve wrote; whether the next starts over.
    current: usize,
    reset: bool,
}

/// FXAA (ported from three.js, see fxaa.frag): the display pass draws into
/// an 8-bit texture the canvas's size, which this pass filters onto the
/// canvas.
struct Fxaa {
    program: ProgramId,
    layout: ParamLayoutId,
    group: ParamGroupId,
    f_texel: ParamField,
    // With TAA: 1, FXAA only where late meshes drew (the target differs
    // from the resolve there); else 0, everywhere.
    f_masked: ParamField,
    // The display pass's output (made at the canvas's size on need).
    ldr: (TextureId, FramebufferId),
    size: (i32, i32),
}

impl Fxaa {
    fn free_target(&mut self, ctx: &Context) {
        let (t, fb) = std::mem::replace(&mut self.ldr, (TextureId::NULL, FramebufferId::NULL));
        if !fb.is_null() {
            ctx.destroy_framebuffer(fb);
        }
        if !t.is_null() {
            ctx.destroy_texture(t);
        }
        self.size = (0, 0);
    }

    // The display pass's target at `w`×`h` (remade when the canvas resized).
    fn ensure_target(&mut self, ctx: &Context, w: i32, h: i32) -> Result<FramebufferId, Error> {
        if self.size == (w, h) && !self.ldr.1.is_null() {
            return Ok(self.ldr.1);
        }
        self.free_target(ctx);
        let t = ctx.create_texture(
            TextureTarget::Texture2D,
            TextureMinFilter::Linear,
            TextureMagFilter::Linear,
        )?;
        ctx.texture_storage_2d(t, 1, &TextureFormat::rgba8(), w, h);
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
        self.ldr = (t, fb);
        self.size = (w, h);
        Ok(fb)
    }
}

/// How linear radiance is squeezed into the display's range. (JS names it
/// by string: `setToneMapping`.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToneMapping {
    /// Exposure and a clamp: what fits shows exactly, the rest clips.
    None = 0,
    /// Khronos PBR Neutral: base colors stay true up to ~0.8, highlights
    /// roll off to white. The default; made for product rendering.
    #[default]
    Neutral = 1,
    /// ACES filmic (Hill's fit, three.js' exposure lift): contrasty, filmic.
    Aces = 2,
    /// AgX: gentle, desaturates highlights like film.
    Agx = 3,
}

impl ToneMapping {
    /// By name: "none", "neutral", "aces", "agx".
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "none" => Self::None,
            "neutral" => Self::Neutral,
            "aces" => Self::Aces,
            "agx" => Self::Agx,
            _ => return None,
        })
    }

    /// Its name, as `from_name` reads it.
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Neutral => "neutral",
            Self::Aces => "aces",
            Self::Agx => "agx",
        }
    }
}

/// The scene's HDR target (sized to the canvas on each frame) and the pass
/// that displays it.
pub struct HdrOutput {
    // What the scene draws into: multisampled renderbuffers, or the texture
    // directly when not multisampling.
    fb: FramebufferId,
    color_rb: RenderbufferId,
    depth_rb: RenderbufferId,
    // The resolved (or directly rendered) radiance the display pass reads.
    resolve_fb: FramebufferId,
    color: TextureId,
    // With contact shadows: the key light's share of each pixel (R8, the
    // opaque pass's second output), its multisampled renderbuffer, and the
    // opaque pass's own framebuffers — the target's color and depth with
    // the share beside (and its resolve). Only the opaque pass draws with
    // two outputs; every other pass keeps `fb` (a draw buffer a shader has
    // no output for is an error in WebGL).
    key_output: bool,
    key: TextureId,
    key_rb: RenderbufferId,
    opaque_fb: FramebufferId,
    opaque_resolve_fb: FramebufferId,
    // A copy of the opaque scene with mips, for transmission (`grab_scale` of the size).
    grab_fb: FramebufferId,
    // Its level 1 (drawn by `grab_down`, firefly-suppressed).
    grab_fb1: FramebufferId,
    // Its depth (the opaque scene's, for transparent meshes drawn into it).
    grab_depth: RenderbufferId,
    // Whether the grab framebuffer is complete with it (some devices
    // refuse the pair: transparent meshes behind glass then go without).
    grab_depth_complete: bool,
    grab: TextureId,
    grab_size: (i32, i32),
    // The copy's share of the target's size (`set_transmission_scale`).
    grab_scale: f32,
    size: (i32, i32),
    samples: i32,
    reverse_z: bool,
    program: ProgramId,
    layout: ParamLayoutId,
    group: ParamGroupId,
    f_exposure: ParamField,
    f_tone_mapping: ParamField,
    f_dither: ParamField,
    f_sharpness: ParamField,
    // The background fill (the renderer's `fill`; w 0: none): the target
    // clears to nothing under it, and the display pass lays it beneath.
    fill: [Vec4; 2],
    f_auto_grey: ParamField,
    // The scene copy's first mip: program, its layout and group.
    grab_down: (ProgramId, ParamLayoutId, ParamGroupId),
    state: StateId,
    generation: u32,
    // Depth peeling's targets, made on first use (`peel`).
    peel: Option<PeelTargets>,
    // The opaque scene's depth, single-sampled, for screen-space effects
    // (`screen_depth`): made on first use.
    screen_depth: Option<(TextureId, FramebufferId)>,
    // Temporal antialiasing, when on (instead of multisampling).
    taa: Option<Taa>,
    // FXAA after the display pass, when on.
    fxaa: Option<Fxaa>,
    // Auto exposure, when on.
    auto: Option<crate::exposure::AutoExposure>,
}

/// Depth peeling (Everitt 2001): the opaque scene's depth, single-sampled,
/// then per layer a depth texture holding the k-th nearest transmissive
/// surface (or the opaque one where there is none). Transmissive layers are
/// shaded back to front into the resolved color, each seeing everything
/// behind it; transparent meshes then draw over the nearest layer.
struct PeelTargets {
    layers: Vec<(TextureId, FramebufferId)>,
    opaque_depth: TextureId,
    opaque_fb: FramebufferId,
    // The resolved color alone (layers shade into it), and with the
    // nearest layer's depth (transparent meshes test against it).
    glass_fb: FramebufferId,
    transparent_fb: FramebufferId,
}

impl PeelTargets {
    fn free(self, ctx: &Context) {
        for (t, fb) in self.layers {
            ctx.destroy_framebuffer(fb);
            ctx.destroy_texture(t);
        }
        for fb in [self.opaque_fb, self.glass_fb, self.transparent_fb] {
            if !fb.is_null() {
                ctx.destroy_framebuffer(fb);
            }
        }
        if !self.opaque_depth.is_null() {
            ctx.destroy_texture(self.opaque_depth);
        }
    }
}

impl HdrOutput {
    /// Whether this device can render into half-float color.
    pub fn supported(ctx: &Context) -> bool {
        ctx.can_render_to(&TextureFormat::rgba16f())
    }

    /// `samples` per pixel for antialiasing (0 or 1 for none; lowered to what
    /// the device allows).
    pub fn new(ctx: &Context, samples: i32, taa: bool) -> Result<Self, Error> {
        if !Self::supported(ctx) {
            return Err("HDR output: this device can't render to half-float targets".into());
        }
        let prefix = "#version 300 es\n";
        let vert = crate::fullscreen::vertex(prefix);
        let frag = ShaderSource::assemble(prefix, &[], "present.frag", PRESENT_FRAG, chunk)?;
        // What is made, freed again if a later step fails.
        let mut undo = luxel_gl::undo::Undo::new(ctx);
        let (program, layout, group) = crate::fullscreen::program(
            ctx,
            &mut undo,
            &vert,
            &frag,
            &ParamLayoutBuilder::new()
                .texture("u_hdr")
                .texture("u_coverage")
                .uniform("u_exposure", UniformType::F32)
                .uniform("u_tone_mapping", UniformType::I32)
                .uniform("u_dither", UniformType::F32)
                .uniform("u_sharpness", UniformType::F32)
                .uniform("u_fill_a", UniformType::Vec4)
                .uniform("u_fill_b", UniformType::Vec4)
                .texture("u_auto_exposure")
                .uniform("u_auto_grey", UniformType::F32),
        )?;
        let taa = if taa {
            let frag = ShaderSource::plain("taa.frag", &format!("{prefix}{TAA_FRAG}"));
            let (program, layout, group) = crate::fullscreen::program(
                ctx,
                &mut undo,
                &vert,
                &frag,
                &ParamLayoutBuilder::new()
                    .texture("u_current")
                    .texture("u_history")
                    .texture("u_prev_motion")
                    .texture("u_depth")
                    .texture("u_velocity")
                    .uniform("u_reproject", UniformType::Mat4)
                    .uniform("u_taa", UniformType::Vec4)
                    .uniform("u_jitter", UniformType::Vec2)
                    .texture("u_auto_exposure")
                    .uniform("u_auto_grey", UniformType::F32),
            )?;
            let copy_frag =
                ShaderSource::plain("taa_copy.frag", &format!("{prefix}{TAA_COPY_FRAG}"));
            let copy = crate::fullscreen::program(
                ctx,
                &mut undo,
                &vert,
                &copy_frag,
                &ParamLayoutBuilder::new()
                    .texture("u_source")
                    .texture("u_coverage"),
            )?;
            let mut copy_state = RenderState::new();
            copy_state.depth_test = false;
            copy_state.depth_mask = false;
            Some(Taa {
                program,
                layout,
                group,
                f_reproject: ctx.param_field(layout, "u_reproject")?,
                f_taa: ctx.param_field(layout, "u_taa")?,
                f_jitter: ctx.param_field(layout, "u_jitter")?,
                f_auto_grey: ctx.param_field(layout, "u_auto_grey")?,
                copy,
                copy_state: ctx.render_state(&copy_state),
                history: [(TextureId::NULL, FramebufferId::NULL); 2],
                motions: [TextureId::NULL; 2],
                depth: TextureId::NULL,
                velocity: (TextureId::NULL, FramebufferId::NULL),
                current: 0,
                reset: true,
            })
        } else {
            None
        };
        let grab_down = {
            let frag = ShaderSource::plain("grab_down.frag", &format!("{prefix}{GRAB_DOWN_FRAG}"));
            crate::fullscreen::program(
                ctx,
                &mut undo,
                &vert,
                &frag,
                &ParamLayoutBuilder::new().texture("u_source"),
            )?
        };
        let mut state = RenderState::new();
        state.depth_test = false;
        state.depth_mask = false;
        let made = Self {
            fb: FramebufferId::NULL,
            color_rb: RenderbufferId::NULL,
            depth_rb: RenderbufferId::NULL,
            resolve_fb: FramebufferId::NULL,
            grab_fb: FramebufferId::NULL,
            grab: TextureId::NULL,
            grab_fb1: FramebufferId::NULL,
            grab_depth: RenderbufferId::NULL,
            grab_depth_complete: false,
            grab_size: (0, 0),
            grab_scale: 1.0,
            color: TextureId::NULL,
            size: (0, 0),
            samples: samples.clamp(0, luxel_gl::limits::max_samples() as i32),
            reverse_z: false,
            program,
            layout,
            group,
            f_exposure: ctx.param_field(layout, "u_exposure")?,
            f_tone_mapping: ctx.param_field(layout, "u_tone_mapping")?,
            f_dither: ctx.param_field(layout, "u_dither")?,
            f_sharpness: ctx.param_field(layout, "u_sharpness")?,
            fill: [Vec4::ZERO; 2],
            f_auto_grey: ctx.param_field(layout, "u_auto_grey")?,
            grab_down,
            state: ctx.render_state(&state),
            generation: ctx.generation(),
            peel: None,
            screen_depth: None,
            taa,
            fxaa: None,
            auto: None,
            key_output: false,
            key: TextureId::NULL,
            key_rb: RenderbufferId::NULL,
            opaque_fb: FramebufferId::NULL,
            opaque_resolve_fb: FramebufferId::NULL,
        };
        undo.keep();
        Ok(made)
    }

    /// Whether its GPU objects belong to an earlier context generation.
    pub fn is_lost(&self, ctx: &Context) -> bool {
        self.generation != ctx.generation()
    }

    /// Whether the target is allocated at `w`×`h` (holds a frame of that
    /// size once one was drawn).
    pub fn holds(&self, w: i32, h: i32) -> bool {
        !self.color.is_null() && self.size == (w.max(1), h.max(1))
    }

    /// The radiance the last frame left (resolved), for passes that read
    /// the scene; NULL before the first frame.
    pub fn color_texture(&self) -> TextureId {
        self.color
    }

    /// Whether temporal antialiasing is on.
    pub fn taa(&self) -> bool {
        self.taa.is_some()
    }

    /// The size of the opaque-scene copy transmission sees through, as a
    /// share of the target (0.25…1, default 1): lower saves memory and the
    /// copy's time, blurring what shows through glass.
    pub fn set_transmission_scale(&mut self, scale: f32) {
        let scale = if scale.is_finite() {
            scale.clamp(0.25, 1.0)
        } else {
            1.0
        };
        if scale != self.grab_scale {
            self.grab_scale = scale;
            self.size = (0, 0); // remade at the next prepare
        }
    }

    /// Auto exposure on (made now) or off (freed).
    pub fn set_auto_exposure(&mut self, ctx: &Context, on: bool) -> Result<(), Error> {
        if on == self.auto.is_some() {
            return Ok(());
        }
        if let Some(a) = self.auto.take() {
            a.destroy(ctx, false);
            return Ok(());
        }
        let vert = crate::fullscreen::vertex("#version 300 es\n");
        self.auto = Some(crate::exposure::AutoExposure::new(ctx, &vert)?);
        Ok(())
    }

    /// Auto exposure's target (`grey`: the frame's brightest-channel
    /// average shows as that), the luminance range it adapts within, and
    /// its speed (per second).
    pub fn set_auto_exposure_options(&mut self, grey: f32, min: f32, max: f32, speed: f32) {
        if let Some(a) = &mut self.auto {
            a.grey = grey.max(1e-4);
            a.min_luminance = min.max(1e-6);
            a.max_luminance = max.max(a.min_luminance);
            a.speed = speed.max(0.0);
        }
    }

    /// Meters this frame for auto exposure (after the last scene pass,
    /// before `present`); `now_ms` a clock in milliseconds.
    pub fn meter_exposure(&mut self, ctx: &Context, now_ms: f64) {
        let (w, h) = self.size;
        let (state, color) = (self.state, self.color);
        if let Some(a) = &mut self.auto
            && !color.is_null()
        {
            a.meter(ctx, state, color, w, h, now_ms);
        }
    }

    // Auto exposure's 1×1 result and grey for shaders (when off: any
    // texture, a sampler needs one, and grey 0).
    fn auto_exposure(&self) -> (TextureId, f32) {
        self.auto
            .as_ref()
            .map_or((self.color, 0.0), |a| (a.texture(), a.grey))
    }

    /// Whether FXAA runs after the display pass.
    pub fn fxaa(&self) -> bool {
        self.fxaa.is_some()
    }

    /// FXAA on (its shader compiled now) or off (its program and target
    /// freed).
    pub fn set_fxaa(&mut self, ctx: &Context, on: bool) -> Result<(), Error> {
        if on == self.fxaa.is_some() {
            return Ok(());
        }
        if let Some(mut f) = self.fxaa.take() {
            f.free_target(ctx);
            ctx.destroy_param_group(f.group);
            ctx.destroy_param_layout(f.layout);
            ctx.destroy_program(f.program);
            return Ok(());
        }
        let prefix = "#version 300 es\n";
        let vert = crate::fullscreen::vertex(prefix);
        let frag = ShaderSource::plain("fxaa.frag", &format!("{prefix}{FXAA_FRAG}"));
        let mut undo = luxel_gl::undo::Undo::new(ctx);
        let program = ctx.create_program_from(&vert, &frag)?;
        undo.push(move |c| c.destroy_program(program));
        let layout = ctx.create_param_layout(
            &ParamLayoutBuilder::new()
                .texture("u_ldr")
                .texture("u_opaque_only")
                .texture("u_final")
                .uniform("u_texel", UniformType::Vec2)
                .uniform("u_masked", UniformType::F32),
        )?;
        undo.push(move |c| c.destroy_param_layout(layout));
        let group = ctx.create_param_group(layout)?;
        undo.push(move |c| c.destroy_param_group(group));
        let fxaa = Fxaa {
            program,
            layout,
            group,
            f_texel: ctx.param_field(layout, "u_texel")?,
            f_masked: ctx.param_field(layout, "u_masked")?,
            ldr: (TextureId::NULL, FramebufferId::NULL),
            size: (0, 0),
        };
        undo.keep();
        self.fxaa = Some(fxaa);
        Ok(())
    }

    /// Starts the TAA history over (a camera cut: nothing to reuse).
    pub fn reset_taa(&mut self) {
        if let Some(taa) = &mut self.taa {
            taa.reset = true;
        }
    }

    /// With TAA, a pass into the velocity target (cleared to "no motion
    /// vector"), and the scene's depth to test motion draws against.
    pub fn motion_pass(&self, ctx: &Context) -> Option<(Pass, TextureId)> {
        let taa = self.taa.as_ref()?;
        let (_, fb) = taa.velocity;
        if fb.is_null() {
            return None;
        }
        ctx.framebuffer_set_clear_color(fb, VELOCITY_NONE, VELOCITY_NONE, 0.0, 0.0);
        let mut pass = Pass::new(fb);
        pass.set_viewport(&Viewport::new(0, 0, self.size.0, self.size.1));
        pass.set_clear(&ClearMask {
            color: true,
            depth: false,
            stencil: false,
        });
        Some((pass, taa.depth))
    }

    /// Resolves this frame's opaque scene into the TAA history: the
    /// resolved color, the last history reprojected — by the velocity
    /// target where `velocity` (`motion_pass` drew into it this frame), else
    /// by `reproject` (clip now, unjittered → clip last frame) through the
    /// depth — and clipped; then copies it back into the target (its
    /// coverage kept), for transmissive and transparent meshes to draw over
    /// unjittered and the display to show. `jitter` is this frame's
    /// projection offset (NDC), `exposure` the display's. After the opaque
    /// pass, before the late ones.
    pub fn taa_resolve(
        &mut self,
        ctx: &Context,
        reproject: &glam::Mat4,
        jitter: glam::Vec2,
        exposure: f32,
        velocity: bool,
    ) {
        let (w, h) = self.size;
        let reverse = self.reverse_z;
        let color = self.color;
        let resolve_fb = self.resolve_fb;
        let (auto_tex, auto_grey) = self.auto_exposure();
        let Some(taa) = &mut self.taa else {
            return;
        };
        let next = 1 - taa.current;
        let (out_tex, out_fb) = taa.history[next];
        ctx.param_set_texture_by_name(taa.group, "u_current", color);
        ctx.param_set_texture_by_name(taa.group, "u_history", taa.history[taa.current].0);
        ctx.param_set_texture_by_name(taa.group, "u_prev_motion", taa.motions[taa.current]);
        ctx.param_set_texture_by_name(taa.group, "u_depth", taa.depth);
        ctx.param_set_texture_by_name(taa.group, "u_velocity", taa.velocity.0);
        ctx.param_set_floats(taa.group, taa.f_reproject, &reproject.to_cols_array());
        let flags = [
            if taa.reset { 1.0 } else { 0.0 },
            if reverse { 1.0 } else { 0.0 },
            if velocity { 1.0 } else { 0.0 },
            exposure.max(1e-6),
        ];
        ctx.param_set_floats(taa.group, taa.f_taa, &flags);
        ctx.param_set_floats(taa.group, taa.f_jitter, &[jitter.x, jitter.y]);
        ctx.param_set_texture_by_name(taa.group, "u_auto_exposure", auto_tex);
        ctx.param_set_f32(taa.group, taa.f_auto_grey, auto_grey);
        ctx.render(&crate::fullscreen::pass(
            out_fb,
            (w, h),
            taa.program,
            self.state,
            taa.group,
        ));
        taa.current = next;
        taa.reset = false;
        // Back into the target: what the late meshes draw over.
        let (copy_program, _, copy_group) = taa.copy;
        ctx.param_set_texture_by_name(copy_group, "u_source", out_tex);
        ctx.param_set_texture_by_name(copy_group, "u_coverage", taa.motions[next]);
        ctx.render(&crate::fullscreen::pass(
            resolve_fb,
            (w, h),
            copy_program,
            taa.copy_state,
            copy_group,
        ));
    }

    /// Samples per pixel actually used (0: no multisampling).
    pub fn samples(&self) -> i32 {
        self.samples
    }

    /// Points `pass` at the HDR target (cleared; resolved and discarded
    /// after drawing when multisampled), sized `w`×`h` (the canvas):
    /// (re)allocates the target when the size or depth mode changed.
    pub fn prepare(&mut self, ctx: &Context, w: i32, h: i32, pass: &mut Pass) -> Result<(), Error> {
        self.prepare_split(ctx, w, h, pass, true)
    }

    /// `prepare` for the first of two passes into the target (`last`
    /// false): it resolves but keeps the multisampled data for the next
    /// (`prepare_late`).
    pub fn prepare_split(
        &mut self,
        ctx: &Context,
        w: i32,
        h: i32,
        pass: &mut Pass,
        last: bool,
    ) -> Result<(), Error> {
        let (w, h) = (w.max(1), h.max(1));
        let reverse_z = ctx.reverse_z();
        if self.size != (w, h) || self.reverse_z != reverse_z || self.fb.is_null() {
            self.allocate(ctx, w, h, reverse_z)?;
        }
        // The canvas's clear color, as premultiplied linear radiance (none
        // under a background fill: the display pass lays that beneath).
        let [r, g, b, a] = if self.fill[0].w > 0.5 {
            [0.0; 4]
        } else {
            ctx.default_framebuffer().clear_color_rgba()
        };
        let lin = |c: f32| {
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        ctx.framebuffer_set_color_clear_f32(self.fb, 0, lin(r) * a, lin(g) * a, lin(b) * a, a);
        pass.set_target(self.fb);
        pass.set_clear(&ClearMask::color_depth());
        self.finish_pass(pass, last);
        if !self.opaque_fb.is_null() {
            // With the key light's share beside (cleared to none).
            let fb = self.opaque_fb;
            ctx.framebuffer_set_color_clear_f32(fb, 0, lin(r) * a, lin(g) * a, lin(b) * a, a);
            ctx.framebuffer_set_color_clear_f32(fb, 1, 0.0, 0.0, 0.0, 0.0);
            pass.set_target(fb);
            if !self.opaque_resolve_fb.is_null() {
                pass.set_resolve(self.opaque_resolve_fb);
                pass.set_resolve_attachments(&[0, 1]);
            }
        }
        Ok(())
    }

    /// The second pass into the target: no clear, drawn over the first.
    pub fn prepare_late(&self, pass: &mut Pass) {
        pass.set_target(self.fb);
        pass.set_clear(&ClearMask::none());
        self.finish_pass(pass, true);
    }

    fn finish_pass(&self, pass: &mut Pass, last: bool) {
        let mut discard = InvalidateMask::none();
        // After the last pass depth isn't read again (except by TAA: its
        // motion pass and resolve), nor the multisampled color once
        // resolved: telling tiled (mobile) GPUs saves writing them back.
        discard.depth = last && self.taa.is_none();
        if self.fb != self.resolve_fb {
            pass.set_resolve(self.resolve_fb);
            discard.color = last;
        } else {
            pass.clear_resolve();
        }
        pass.set_invalidate(&discard);
    }

    /// The scene copy transmission reads, and its last mip — valid once
    /// `grab` has run this frame.
    pub fn scene_color(&self) -> Option<(TextureId, u32)> {
        let levels = mip_levels(self.grab_size.0, self.grab_size.1);
        (!self.grab.is_null()).then_some((self.grab, levels - 1))
    }

    /// Copies what the target holds (the resolved opaque scene) into the
    /// mipmapped texture transmissive surfaces see through; with
    /// `see_through` (transparent meshes), draws them into the copy too,
    /// tested against the opaque scene's depth — as Khronos' renderer puts
    /// its transparent drawables into its transmission framebuffer, so glass
    /// shows the alpha-blended things behind it.
    pub fn grab(&mut self, ctx: &Context, see_through: Option<&mut Pass>) {
        let (w, h) = self.size;
        let (gw, gh) = self.grab_size;
        ctx.blit_framebuffer(
            self.resolve_fb,
            &Viewport::new(0, 0, w, h),
            self.grab_fb,
            &Viewport::new(0, 0, gw, gh),
            &ClearMask::color(),
            TextureMagFilter::Linear,
        );
        if let Some(pass) = see_through
            && pass.draw_count() > 0
            && self.depth_into_grab(ctx)
        {
            pass.sort();
            pass.set_target(self.grab_fb);
            pass.set_viewport(&Viewport::new(0, 0, gw, gh));
            pass.set_clear(&ClearMask::none());
            pass.clear_resolve();
            pass.set_invalidate(&InvalidateMask::none());
            ctx.render(pass);
        }
        let levels = mip_levels(gw, gh) as i32;
        if levels < 2 || self.grab_fb1.is_null() {
            ctx.texture_generate_mipmaps(self.grab);
            return;
        }
        // Level 1 from level 0, fireflies suppressed (grab_down.frag); the
        // texture sampled at level 0 alone meanwhile (no feedback loop).
        let (program, _, group) = self.grab_down;
        ctx.texture_set_base_level(self.grab, 0);
        ctx.texture_set_max_level(self.grab, 0);
        ctx.param_set_texture_by_name(group, "u_source", self.grab);
        let half = ((gw / 2).max(1), (gh / 2).max(1));
        let mut pass = crate::fullscreen::pass(self.grab_fb1, half, program, self.state, group);
        pass.set_clear(&ClearMask::color()); // every texel written
        ctx.render(&pass);
        // The rest from level 1 (box), then sampled whole again.
        ctx.texture_set_base_level(self.grab, 1);
        ctx.texture_set_max_level(self.grab, levels - 1);
        ctx.texture_generate_mipmaps(self.grab);
        ctx.texture_set_base_level(self.grab, 0);
    }

    // The opaque scene's depth into the copy's target (its own renderbuffer,
    // made on first need), for meshes drawn into the copy. False where it
    // can't be: a multisampled target only blits to the same size.
    fn depth_into_grab(&mut self, ctx: &Context) -> bool {
        let (w, h) = self.size;
        let (gw, gh) = self.grab_size;
        if self.fb != self.resolve_fb && (gw, gh) != (w, h) {
            return false;
        }
        if self.grab_depth.is_null() {
            match ctx.create_renderbuffer(depth_renderbuffer_format(self.reverse_z), gw, gh) {
                Ok(rb) => {
                    // Kept either way (freed with the targets), so an
                    // incomplete pair isn't made again every frame.
                    ctx.framebuffer_attach_depth_renderbuffer(self.grab_fb, rb);
                    self.grab_depth = rb;
                    self.grab_depth_complete = ctx.framebuffer_is_complete(self.grab_fb);
                }
                Err(e) => {
                    luxel_gl::console::error_once(&format!(
                        "[luxel] transparent meshes behind glass: {e}"
                    ));
                    return false;
                }
            }
        }
        if !self.grab_depth_complete {
            return false;
        }
        ctx.blit_framebuffer(
            self.fb,
            &Viewport::new(0, 0, w, h),
            self.grab_fb,
            &Viewport::new(0, 0, gw, gh),
            &ClearMask::depth(),
            TextureMagFilter::Nearest,
        );
        true
    }

    // The depth format of the target (what depth blits copy between).
    fn depth_format(&self) -> TextureFormat {
        depth_texture_format(self.reverse_z)
    }

    /// Frees the peeling targets (peeling turned off).
    pub fn release_peel(&mut self, ctx: &Context) {
        if let Some(peel) = self.peel.take() {
            peel.free(ctx);
        }
    }

    /// Makes (or keeps) `layers` peeling targets at the current size.
    pub fn ensure_peel(&mut self, ctx: &Context, layers: usize) -> Result<(), Error> {
        if self.peel.as_ref().is_some_and(|p| p.layers.len() == layers) {
            return Ok(());
        }
        if let Some(peel) = self.peel.take() {
            peel.free(ctx);
        }
        let (w, h) = self.size;
        let format = self.depth_format();
        let depth = |ctx: &Context| -> Result<(TextureId, FramebufferId), Error> {
            let t = ctx.create_texture(
                TextureTarget::Texture2D,
                TextureMinFilter::Nearest,
                TextureMagFilter::Nearest,
            )?;
            ctx.texture_storage_2d(t, 1, &format, w, h);
            let fb = ctx.create_framebuffer(w, h)?;
            ctx.framebuffer_attach_depth_texture(fb, t, 0);
            Ok((t, fb))
        };
        // Built as it goes, so a failure part way frees what was made.
        let mut peel = PeelTargets {
            layers: Vec::with_capacity(layers),
            opaque_depth: TextureId::NULL,
            opaque_fb: FramebufferId::NULL,
            glass_fb: FramebufferId::NULL,
            transparent_fb: FramebufferId::NULL,
        };
        let made = (|| -> Result<(), Error> {
            (peel.opaque_depth, peel.opaque_fb) = depth(ctx)?;
            for _ in 0..layers {
                peel.layers.push(depth(ctx)?);
            }
            peel.glass_fb = ctx.create_framebuffer(w, h)?;
            ctx.framebuffer_attach_color_texture_2d(peel.glass_fb, 0, self.color, 0);
            peel.transparent_fb = ctx.create_framebuffer(w, h)?;
            ctx.framebuffer_attach_color_texture_2d(peel.transparent_fb, 0, self.color, 0);
            ctx.framebuffer_attach_depth_texture(peel.transparent_fb, peel.layers[0].0, 0);
            Ok(())
        })();
        if let Err(e) = made {
            peel.free(ctx);
            return Err(e);
        }
        self.peel = Some(peel);
        Ok(())
    }

    /// The opaque pass's depth as a single-sampled texture (resolved from the
    /// multisampled target), for screen-space effects reading it — after
    /// the opaque pass. None if it can't be made.
    pub fn screen_depth(&mut self, ctx: &Context) -> Option<TextureId> {
        // TAA draws into a depth texture already.
        if let Some(taa) = &self.taa
            && !taa.depth.is_null()
        {
            return Some(taa.depth);
        }
        let (w, h) = self.size;
        if self.screen_depth.is_none() {
            let format = self.depth_format();
            let t = ctx
                .create_texture(
                    TextureTarget::Texture2D,
                    TextureMinFilter::Nearest,
                    TextureMagFilter::Nearest,
                )
                .ok()?;
            ctx.texture_storage_2d(t, 1, &format, w, h);
            let Ok(fb) = ctx.create_framebuffer(w, h) else {
                ctx.destroy_texture(t);
                return None;
            };
            ctx.framebuffer_attach_depth_texture(fb, t, 0);
            self.screen_depth = Some((t, fb));
        }
        let (t, fb) = self.screen_depth?;
        let r = Viewport::new(0, 0, w, h);
        ctx.blit_framebuffer(
            self.fb,
            &r,
            fb,
            &r,
            &ClearMask::depth(),
            TextureMagFilter::Nearest,
        );
        Some(t)
    }

    /// What the scene draws into (multisampled or not), and its size.
    pub fn draw_target(&self) -> (FramebufferId, (i32, i32)) {
        (self.fb, self.size)
    }

    /// Where the occlusion is applied, and the key light's share (NULL
    /// without contact shadows' output).
    pub fn occlusion_target(&self) -> (FramebufferId, TextureId) {
        (
            self.fb,
            if self.opaque_fb.is_null() {
                TextureId::NULL
            } else {
                self.key
            },
        )
    }

    /// Whether the opaque pass also writes the key light's share (for
    /// contact shadows; every shader drawing in it must then have a second
    /// output): remade at the next frame when it changes.
    pub fn set_key_output(&mut self, ctx: &Context, on: bool) {
        if self.key_output != on {
            self.key_output = on;
            self.free_target(ctx);
        }
    }

    /// Resolves the multisampled target into the color again (after
    /// drawing into it past its pass's resolve); nothing without samples.
    pub fn resolve_again(&self, ctx: &Context) {
        if self.fb != self.resolve_fb {
            let (w, h) = self.size;
            let r = Viewport::new(0, 0, w, h);
            // The color only (the key light's share stays as resolved).
            ctx.blit_framebuffer_attachment(
                self.fb,
                0,
                &r,
                self.resolve_fb,
                0,
                &r,
                TextureMagFilter::Nearest,
            );
        }
    }

    /// Copies the opaque pass's depth (resolved from the multisampled
    /// target) for peeling. After the opaque pass, before `peel_layer`.
    pub fn keep_opaque_depth(&self, ctx: &Context) {
        if let Some(p) = &self.peel {
            let (w, h) = self.size;
            let r = Viewport::new(0, 0, w, h);
            ctx.blit_framebuffer(
                self.fb,
                &r,
                p.opaque_fb,
                &r,
                &ClearMask::depth(),
                TextureMagFilter::Nearest,
            );
        }
    }

    /// Readies layer `k`'s depth target, starting from the opaque depth,
    /// and returns it with the previous layer's depth texture (NULL for
    /// the first): what the peel pass discards against.
    pub fn peel_layer(&self, ctx: &Context, k: usize) -> Option<(FramebufferId, TextureId)> {
        let p = self.peel.as_ref()?;
        let (_, fb) = *p.layers.get(k)?;
        let (w, h) = self.size;
        let r = Viewport::new(0, 0, w, h);
        ctx.blit_framebuffer(
            p.opaque_fb,
            &r,
            fb,
            &r,
            &ClearMask::depth(),
            TextureMagFilter::Nearest,
        );
        let previous = k.checked_sub(1).map_or(TextureId::NULL, |j| p.layers[j].0);
        Some((fb, previous))
    }

    /// Layer `k`'s depth texture (what its shading pass matches).
    pub fn layer_depth(&self, k: usize) -> TextureId {
        self.peel
            .as_ref()
            .and_then(|p| p.layers.get(k))
            .map_or(TextureId::NULL, |l| l.0)
    }

    /// Where peeled layers shade (the resolved color, no depth), and where
    /// transparent meshes draw after them (with the nearest layer's depth).
    pub fn peel_color_targets(&self) -> Option<(FramebufferId, FramebufferId)> {
        self.peel.as_ref().map(|p| (p.glass_fb, p.transparent_fb))
    }

    /// The background fill (`SceneRenderer::fill`), from the next frame.
    pub fn set_fill(&mut self, fill: [Vec4; 2]) {
        self.fill = fill;
    }

    /// Draws the target to the canvas (through FXAA when on).
    pub fn present(
        &mut self,
        ctx: &Context,
        exposure: f32,
        tone_mapping: ToneMapping,
        dither: bool,
        sharpness: f32,
        profiler: Option<&mut GpuProfiler>,
    ) {
        // With TAA the target holds the resolve (copied back) and what drew
        // over it.
        ctx.param_set_texture_by_name(self.group, "u_hdr", self.color);
        ctx.param_set_texture_by_name(self.group, "u_coverage", self.color);
        ctx.param_set_f32(self.group, self.f_exposure, exposure);
        let (auto_tex, auto_grey) = self.auto_exposure();
        ctx.param_set_texture_by_name(self.group, "u_auto_exposure", auto_tex);
        ctx.param_set_f32(self.group, self.f_auto_grey, auto_grey);
        ctx.param_set_i32(self.group, self.f_tone_mapping, tone_mapping as i32);
        ctx.param_set_f32(self.group, self.f_dither, if dither { 1.0 } else { 0.0 });
        ctx.param_set_f32(self.group, self.f_sharpness, sharpness.clamp(0.0, 1.0));
        let [fa, fb] = self.fill;
        ctx.param_set_floats_by_name(self.group, "u_fill_a", &fa.to_array());
        ctx.param_set_floats_by_name(self.group, "u_fill_b", &fb.to_array());
        let fb = ctx.default_framebuffer();
        let (cw, ch) = (fb.width().max(1), fb.height().max(1));
        // With FXAA the display pass draws into its texture (it failing to
        // allocate: straight to the canvas, unfiltered).
        let ldr = match &mut self.fxaa {
            Some(f) => match f.ensure_target(ctx, cw, ch) {
                Ok(fb) => Some(fb),
                Err(e) => {
                    luxel_gl::console::error_once(&format!("[luxel] FXAA target: {e}"));
                    None
                }
            },
            None => None,
        };
        let mut pass = crate::fullscreen::pass(
            ldr.unwrap_or(FramebufferId::NULL),
            (cw, ch),
            self.program,
            self.state,
            self.group,
        );
        if ldr.is_some() {
            // Every pixel is written: nothing to load on tiled GPUs.
            pass.set_clear(&ClearMask::color());
        }
        let mut profiler = profiler;
        if let Some(p) = profiler.as_deref_mut() {
            p.time(&mut pass, "present");
        }
        ctx.render(&pass);
        let (Some(_), Some(f)) = (ldr, &self.fxaa) else {
            return;
        };
        ctx.param_set_texture_by_name(f.group, "u_ldr", f.ldr.0);
        ctx.param_set_floats(f.group, f.f_texel, &[1.0 / cw as f32, 1.0 / ch as f32]);
        // With TAA, FXAA for what TAA leaves out (transmissive, transparent):
        // where the target differs from the resolve.
        let resolved = match &self.taa {
            Some(taa) if !taa.history[taa.current].0.is_null() => Some(taa.history[taa.current].0),
            _ => None,
        };
        ctx.param_set_texture_by_name(f.group, "u_opaque_only", resolved.unwrap_or(self.color));
        ctx.param_set_texture_by_name(f.group, "u_final", self.color);
        ctx.param_set_f32(
            f.group,
            f.f_masked,
            if resolved.is_some() { 1.0 } else { 0.0 },
        );
        let mut pass = crate::fullscreen::pass(
            FramebufferId::NULL,
            (cw, ch),
            f.program,
            self.state,
            f.group,
        );
        if let Some(p) = profiler {
            p.time(&mut pass, "fxaa");
        }
        ctx.render(&pass);
    }

    fn allocate(&mut self, ctx: &Context, w: i32, h: i32, reverse_z: bool) -> Result<(), Error> {
        self.free_target(ctx);
        self.size = (w, h);
        self.reverse_z = reverse_z;
        let depth = depth_renderbuffer_format(reverse_z);
        let color = ctx.create_texture(
            TextureTarget::Texture2D,
            TextureMinFilter::Nearest,
            TextureMagFilter::Nearest,
        )?;
        ctx.texture_storage_2d(color, 1, &TextureFormat::rgba16f(), w, h);
        self.color = color;
        let resolve_fb = ctx.create_framebuffer(w, h)?;
        ctx.framebuffer_attach_color_texture_2d(resolve_fb, 0, color, 0);
        self.resolve_fb = resolve_fb;
        if self.key_output {
            let key = ctx.create_texture(
                TextureTarget::Texture2D,
                TextureMinFilter::Nearest,
                TextureMagFilter::Nearest,
            )?;
            ctx.texture_storage_2d(key, 1, &TextureFormat::r8(), w, h);
            self.key = key;
        }

        let gw = ((w as f32 * self.grab_scale).round() as i32).max(1);
        let gh = ((h as f32 * self.grab_scale).round() as i32).max(1);
        let grab = ctx.create_texture(
            TextureTarget::Texture2D,
            TextureMinFilter::LinearMipmapLinear,
            TextureMagFilter::Linear,
        )?;
        ctx.texture_storage_2d(
            grab,
            mip_levels(gw, gh) as i32,
            &TextureFormat::rgba16f(),
            gw,
            gh,
        );
        ctx.texture_set_wrap_s(grab, TextureWrap::ClampToEdge);
        ctx.texture_set_wrap_t(grab, TextureWrap::ClampToEdge);
        let grab_fb = ctx.create_framebuffer(gw, gh)?;
        ctx.framebuffer_attach_color_texture_2d(grab_fb, 0, grab, 0);
        (self.grab, self.grab_fb, self.grab_size) = (grab, grab_fb, (gw, gh));
        if mip_levels(gw, gh) > 1 {
            let (w1, h1) = ((gw / 2).max(1), (gh / 2).max(1));
            let fb1 = ctx.create_framebuffer(w1, h1)?;
            ctx.framebuffer_attach_color_texture_2d(fb1, 0, grab, 1);
            self.grab_fb1 = fb1;
        }

        // Multisampled if the device takes it for this format; else straight
        // into the texture. No more samples than both formats take.
        if self.samples > 1 {
            let most = ctx
                .max_samples_for(RenderbufferFormat::Rgba16f)
                .min(ctx.max_samples_for(depth));
            self.samples = self.samples.min(most);
        }
        while self.samples > 1 && self.taa.is_none() {
            let fb = ctx.create_framebuffer(w, h)?;
            let color_rb = ctx.create_renderbuffer_multisample(
                RenderbufferFormat::Rgba16f,
                self.samples,
                w,
                h,
            )?;
            let depth_rb = ctx.create_renderbuffer_multisample(depth, self.samples, w, h)?;
            ctx.framebuffer_attach_color_renderbuffer(fb, 0, color_rb);
            ctx.framebuffer_attach_depth_renderbuffer(fb, depth_rb);
            if ctx.framebuffer_is_complete(fb) {
                (self.fb, self.color_rb, self.depth_rb) = (fb, color_rb, depth_rb);
                self.build_key_targets(ctx, w, h)?;
                return Ok(());
            }
            ctx.destroy_framebuffer(fb);
            ctx.destroy_renderbuffer(color_rb);
            ctx.destroy_renderbuffer(depth_rb);
            self.samples /= 2;
        }
        self.samples = 0;
        if let Some(taa) = &mut self.taa {
            let tex = |format: &TextureFormat, filter| -> Result<TextureId, Error> {
                let t = ctx.create_texture(
                    TextureTarget::Texture2D,
                    filter,
                    TextureMagFilter::Nearest,
                )?;
                ctx.texture_storage_2d(t, 1, format, w, h);
                Ok(t)
            };
            taa.depth = tex(&depth_texture_format(reverse_z), TextureMinFilter::Nearest)?;
            ctx.framebuffer_attach_depth_texture(resolve_fb, taa.depth, 0);
            for h_slot in &mut taa.history {
                let t = ctx.create_texture(
                    TextureTarget::Texture2D,
                    TextureMinFilter::Linear,
                    TextureMagFilter::Linear,
                )?;
                ctx.texture_storage_2d(t, 1, &TextureFormat::rgba16f(), w, h);
                ctx.texture_set_wrap_s(t, TextureWrap::ClampToEdge);
                ctx.texture_set_wrap_t(t, TextureWrap::ClampToEdge);
                let fb = ctx.create_framebuffer(w, h)?;
                ctx.framebuffer_attach_color_texture_2d(fb, 0, t, 0);
                *h_slot = (t, fb);
            }
            // Each resolve's motion and coverage next to its history
            // (renderable with the half-float color the HDR target needs).
            for i in 0..2 {
                let m = tex(&TextureFormat::rgba16f(), TextureMinFilter::Nearest)?;
                ctx.framebuffer_attach_color_texture_2d(taa.history[i].1, 1, m, 0);
                taa.motions[i] = m;
            }
            let v = tex(&TextureFormat::rg16f(), TextureMinFilter::Nearest)?;
            let fb = ctx.create_framebuffer(w, h)?;
            ctx.framebuffer_attach_color_texture_2d(fb, 0, v, 0);
            if ctx.framebuffer_is_complete(fb) {
                taa.velocity = (v, fb);
            } else {
                // Moving meshes then lean on the clip alone.
                ctx.destroy_framebuffer(fb);
                ctx.destroy_texture(v);
            }
            taa.reset = true;
        } else {
            let depth_rb = ctx.create_renderbuffer(depth, w, h)?;
            ctx.framebuffer_attach_depth_renderbuffer(resolve_fb, depth_rb);
            self.depth_rb = depth_rb;
        }
        self.fb = resolve_fb;
        if !ctx.framebuffer_is_complete(resolve_fb) {
            return Err("HDR output: the half-float target is incomplete on this device".into());
        }
        self.build_key_targets(ctx, w, h)?;
        Ok(())
    }

    // The opaque pass's framebuffers with the key light's share beside the
    // target's color and depth (contact shadows): the multisampled ones and
    // their resolve, or the textures themselves. Without the share's output,
    // or if the device can't take them, none (the pass draws into `fb`).
    fn build_key_targets(&mut self, ctx: &Context, w: i32, h: i32) -> Result<(), Error> {
        if self.key.is_null() {
            return Ok(());
        }
        let fb = ctx.create_framebuffer(w, h)?;
        if self.fb != self.resolve_fb {
            let rb =
                ctx.create_renderbuffer_multisample(RenderbufferFormat::R8, self.samples, w, h)?;
            self.key_rb = rb;
            ctx.framebuffer_attach_color_renderbuffer(fb, 0, self.color_rb);
            ctx.framebuffer_attach_color_renderbuffer(fb, 1, rb);
            ctx.framebuffer_attach_depth_renderbuffer(fb, self.depth_rb);
            let resolve = ctx.create_framebuffer(w, h)?;
            ctx.framebuffer_attach_color_texture_2d(resolve, 0, self.color, 0);
            ctx.framebuffer_attach_color_texture_2d(resolve, 1, self.key, 0);
            self.opaque_resolve_fb = resolve;
        } else {
            ctx.framebuffer_attach_color_texture_2d(fb, 0, self.color, 0);
            ctx.framebuffer_attach_color_texture_2d(fb, 1, self.key, 0);
            match &self.taa {
                Some(taa) if !taa.depth.is_null() => {
                    ctx.framebuffer_attach_depth_texture(fb, taa.depth, 0)
                }
                _ => ctx.framebuffer_attach_depth_renderbuffer(fb, self.depth_rb),
            }
        }
        self.opaque_fb = fb;
        let complete = ctx.framebuffer_is_complete(fb)
            && (self.opaque_resolve_fb.is_null()
                || ctx.framebuffer_is_complete(self.opaque_resolve_fb));
        if !complete {
            // Contact shadows then dim the whole pixel, as without the share.
            self.free_key_targets(ctx);
        }
        Ok(())
    }

    fn free_key_targets(&mut self, ctx: &Context) {
        for fb in [self.opaque_fb, self.opaque_resolve_fb] {
            if !fb.is_null() {
                ctx.destroy_framebuffer(fb);
            }
        }
        if !self.key_rb.is_null() {
            ctx.destroy_renderbuffer(self.key_rb);
        }
        if !self.key.is_null() {
            ctx.destroy_texture(self.key);
        }
        self.opaque_fb = FramebufferId::NULL;
        self.opaque_resolve_fb = FramebufferId::NULL;
        self.key_rb = RenderbufferId::NULL;
        self.key = TextureId::NULL;
    }

    fn free_target(&mut self, ctx: &Context) {
        if let Some(taa) = &mut self.taa {
            for (t, fb) in std::mem::replace(
                &mut taa.history,
                [(TextureId::NULL, FramebufferId::NULL); 2],
            ) {
                if !fb.is_null() {
                    ctx.destroy_framebuffer(fb);
                }
                if !t.is_null() {
                    ctx.destroy_texture(t);
                }
            }
            if !taa.depth.is_null() {
                ctx.destroy_texture(std::mem::replace(&mut taa.depth, TextureId::NULL));
            }
            for m in std::mem::replace(&mut taa.motions, [TextureId::NULL; 2]) {
                if !m.is_null() {
                    ctx.destroy_texture(m);
                }
            }
            let (v, fb) =
                std::mem::replace(&mut taa.velocity, (TextureId::NULL, FramebufferId::NULL));
            if !fb.is_null() {
                ctx.destroy_framebuffer(fb);
            }
            if !v.is_null() {
                ctx.destroy_texture(v);
            }
        }
        if let Some(peel) = self.peel.take() {
            peel.free(ctx);
        }
        if let Some((t, fb)) = self.screen_depth.take() {
            ctx.destroy_framebuffer(fb);
            ctx.destroy_texture(t);
        }
        if self.fb != self.resolve_fb && !self.fb.is_null() {
            ctx.destroy_framebuffer(self.fb);
        }
        if !self.resolve_fb.is_null() {
            ctx.destroy_framebuffer(self.resolve_fb);
        }
        if !self.grab_fb.is_null() {
            ctx.destroy_framebuffer(self.grab_fb);
        }
        if !self.grab_fb1.is_null() {
            ctx.destroy_framebuffer(self.grab_fb1);
        }
        self.grab_fb1 = FramebufferId::NULL;
        if !self.grab_depth.is_null() {
            ctx.destroy_renderbuffer(self.grab_depth);
        }
        self.grab_depth = RenderbufferId::NULL;
        self.grab_depth_complete = false;
        if !self.grab.is_null() {
            ctx.destroy_texture(self.grab);
        }
        self.grab_fb = FramebufferId::NULL;
        self.grab = TextureId::NULL;
        self.free_key_targets(ctx);
        for rb in [self.color_rb, self.depth_rb] {
            if !rb.is_null() {
                ctx.destroy_renderbuffer(rb);
            }
        }
        if !self.color.is_null() {
            ctx.destroy_texture(self.color);
        }
        self.fb = FramebufferId::NULL;
        self.resolve_fb = FramebufferId::NULL;
        self.color_rb = RenderbufferId::NULL;
        self.depth_rb = RenderbufferId::NULL;
        self.color = TextureId::NULL;
    }

    /// Frees it (after a context loss only the CPU-side group is left).
    pub fn destroy(mut self, ctx: &Context) {
        if let Some(a) = self.auto.take() {
            a.destroy(ctx, self.generation != ctx.generation());
        }
        ctx.destroy_param_group(self.group);
        ctx.destroy_param_layout(self.layout);
        ctx.destroy_param_group(self.grab_down.2);
        ctx.destroy_param_layout(self.grab_down.1);
        if self.generation == ctx.generation() {
            ctx.destroy_program(self.grab_down.0);
        }
        if let Some(taa) = &self.taa {
            ctx.destroy_param_group(taa.group);
            ctx.destroy_param_layout(taa.layout);
            ctx.destroy_param_group(taa.copy.2);
            ctx.destroy_param_layout(taa.copy.1);
            if self.generation == ctx.generation() {
                ctx.destroy_program(taa.program);
                ctx.destroy_program(taa.copy.0);
            }
        }
        if let Some(mut f) = self.fxaa.take() {
            ctx.destroy_param_group(f.group);
            ctx.destroy_param_layout(f.layout);
            if self.generation == ctx.generation() {
                f.free_target(ctx);
                ctx.destroy_program(f.program);
            }
        }
        if self.generation != ctx.generation() {
            return;
        }
        self.free_target(ctx);
        ctx.destroy_program(self.program);
    }
}

fn mip_levels(w: i32, h: i32) -> u32 {
    luxel_gl::texture::mip_levels(w, h) as u32
}

/// Depth with float precision where reverse-Z can use it (a texture to
/// sample, or a renderbuffer).
pub(crate) fn depth_texture_format(reverse_z: bool) -> TextureFormat {
    if reverse_z {
        TextureFormat::depth32f()
    } else {
        TextureFormat::depth24()
    }
}

pub(crate) fn depth_renderbuffer_format(reverse_z: bool) -> RenderbufferFormat {
    if reverse_z {
        RenderbufferFormat::Depth32f
    } else {
        RenderbufferFormat::Depth24
    }
}
