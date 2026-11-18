//! The ground shadow: a soft dark patch on an invisible ground under the
//! casters — for scenes lit by their environment alone, where no light's
//! shadow map has anything to say. Ported from three.js' contact-shadows
//! example (webgl_shadow_contact.html, MIT): the casters' depth seen from
//! the ground up, as darkness (full on the ground, none at the reach's
//! top), blurred twice (Gaussian, a second time at 0.4 of the radius
//! against its artifacts), shown on a plane under them.
//!
//! The view from below is a shadow view (`views::ground_view`): its casters
//! come from the same culling and caching as the lights' maps, and are
//! redrawn only when they move.

use glam::{Mat4, Vec2};
use rswebgl::buffer::BufferUsage;
use rswebgl::context::Context;
use rswebgl::draw::{DrawCommand, DrawMode, Viewport};
use rswebgl::error::Error;
use rswebgl::framebuffer::ClearMask;
use rswebgl::ids::{
    FramebufferId, ParamGroupId, ParamLayoutId, ProgramId, StateId, TextureId, UniformBufferId,
    VertexArrayId,
};
use rswebgl::param::ParamLayoutBuilder;
use rswebgl::pass::Pass;
use rswebgl::render_state::{BlendFactor, DepthFunc, RenderState};
use rswebgl::shader_source::ShaderSource;
use rswebgl::texture::{
    TextureFormat, TextureMagFilter, TextureMinFilter, TextureTarget, TextureWrap,
};
use rswebgl::uniform_buffer::{UboLayout, UniformType};
use rswebgl::vao_builder::{AttrKind, VaoBuilder};

use super::views::ShadowView;
use crate::render::FRAME_BLOCK;
use crate::shader::create_scene_program;

const DARKNESS_FRAG: &str = include_str!("../../shaders/ground_darkness.frag");
const BLUR_FRAG: &str = include_str!("../../shaders/ground_blur.frag");
const PLANE_VERT: &str = include_str!("../../shaders/ground.vert");
const PLANE_FRAG: &str = include_str!("../../shaders/ground.frag");

/// How the ground shadow looks (three.js' example's defaults).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GroundShadow {
    /// How dark where a caster touches the ground (1: black before the
    /// blur; above 1 darkens higher parts too).
    pub darkness: f32,
    /// The blur's radius, in 256ths of the patch.
    pub blur: f32,
    /// The patch's opacity.
    pub opacity: f32,
    /// How far above the ground a caster still darkens it (world units; 0:
    /// half the casters' height).
    pub reach: f32,
    /// The darkness texture's side in texels.
    pub size: u32,
}

impl Default for GroundShadow {
    fn default() -> Self {
        Self {
            darkness: 1.0,
            blur: 3.5,
            opacity: 1.0,
            reach: 0.0,
            size: 512,
        }
    }
}

/// What the renderer draws for the ground shadow: the plane, as a blended
/// mesh after the opaque scene.
#[derive(Debug, Clone, Copy)]
pub(crate) struct GroundPlane {
    pub program: ProgramId,
    pub state: StateId,
    pub vao: VertexArrayId,
    pub material: ParamGroupId,
    pub model: Mat4,
    pub command: DrawCommand,
}

/// The ground shadow's GPU side.
pub(crate) struct Ground {
    pub settings: GroundShadow,
    // The casters' depth from below, and the darkness (two, ping-ponged by
    // the blur; the first ends up shown).
    size: u32,
    depth: TextureId,
    depth_fb: FramebufferId,
    dark: [TextureId; 2],
    dark_fb: [FramebufferId; 2],
    // The casters' pass, its frame block and group, its render state.
    pub pass: Pass,
    ubo: UniformBufferId,
    group: ParamGroupId,
    pub caster_state: StateId,
    // The full-screen steps.
    darkness: ProgramId,
    darkness_layout: ParamLayoutId,
    darkness_group: ParamGroupId,
    blur: ProgramId,
    blur_layout: ParamLayoutId,
    blur_groups: [ParamGroupId; 4],
    fullscreen_state: StateId,
    // The plane.
    plane_program: ProgramId,
    plane_layout: ParamLayoutId,
    plane_group: ParamGroupId,
    plane_state: StateId,
    plane_vao: VertexArrayId,
    // This frame's view and plane (None: nothing to shadow), the signature
    // of what the maps hold, and whether they're drawn again this frame.
    pub view: Option<(ShadowView, Mat4)>,
    drawn: Option<u64>,
    dirty: bool,
}

impl Ground {
    /// Its programs, plane and groups; `frame_layout` groups with a frame
    /// block of their own and the `shared` blocks. The textures come with
    /// the first frame that needs them.
    pub fn new(
        ctx: &Context,
        frame_layout: ParamLayoutId,
        shared: &[(&str, UniformBufferId)],
    ) -> Result<Self, Error> {
        let mut undo = rswebgl::undo::Undo::new(ctx);
        let ubo = ctx.create_uniform_buffer(
            &UboLayout::new()
                .field("u_view", UniformType::Mat4)
                .field("u_proj", UniformType::Mat4)
                .field("u_view_proj", UniformType::Mat4)
                .field("u_camera_pos", UniformType::Vec4),
            BufferUsage::DynamicDraw,
        )?;
        undo.push(move |c| c.destroy_uniform_buffer(ubo));
        let group = ctx.create_param_group(frame_layout)?;
        undo.push(move |c| c.destroy_param_group(group));
        ctx.param_set_block_by_name(group, FRAME_BLOCK, ubo);
        for &(name, block) in shared {
            ctx.param_set_block_by_name(group, name, block);
        }

        let prefix = "#version 300 es\n";
        let vert = crate::fullscreen::vertex(prefix);
        let frag = ShaderSource::plain("ground_darkness.frag", &format!("{prefix}{DARKNESS_FRAG}"));
        let darkness = ctx.create_program_from(&vert, &frag)?;
        undo.push(move |c| c.destroy_program(darkness));
        let darkness_layout = ctx.create_param_layout(
            &ParamLayoutBuilder::new()
                .texture("u_depth")
                .uniform("u_ground_params", UniformType::Vec4),
        )?;
        undo.push(move |c| c.destroy_param_layout(darkness_layout));
        let darkness_group = ctx.create_param_group(darkness_layout)?;
        undo.push(move |c| c.destroy_param_group(darkness_group));

        let frag = ShaderSource::plain("ground_blur.frag", &format!("{prefix}{BLUR_FRAG}"));
        let blur = ctx.create_program_from(&vert, &frag)?;
        undo.push(move |c| c.destroy_program(blur));
        let blur_layout = ctx.create_param_layout(
            &ParamLayoutBuilder::new()
                .texture("u_src")
                .uniform("u_step", UniformType::Vec2),
        )?;
        undo.push(move |c| c.destroy_param_layout(blur_layout));
        let mut blur_groups = [ParamGroupId::NULL; 4];
        for g in &mut blur_groups {
            let group = ctx.create_param_group(blur_layout)?;
            undo.push(move |c| c.destroy_param_group(group));
            *g = group;
        }
        let mut fs = RenderState::new();
        fs.depth_test = false;
        fs.depth_mask = false;

        let plane_program = create_scene_program(ctx, PLANE_VERT, PLANE_FRAG)?;
        undo.push(move |c| c.destroy_program(plane_program));
        let plane_layout = ctx.create_param_layout(
            &ParamLayoutBuilder::new()
                .texture("u_ground")
                .uniform("u_ground_map", UniformType::Mat4)
                .uniform("u_ground_opacity", UniformType::F32),
        )?;
        undo.push(move |c| c.destroy_param_layout(plane_layout));
        let plane_group = ctx.create_param_group(plane_layout)?;
        undo.push(move |c| c.destroy_param_group(plane_group));
        // Blended over the scene, tested against it, writing no depth (as
        // glTF's blended materials).
        let mut ps = RenderState::new();
        ps.blend = true;
        ps.blend_src_rgb = BlendFactor::SrcAlpha;
        ps.blend_dst_rgb = BlendFactor::OneMinusSrcAlpha;
        ps.blend_src_alpha = BlendFactor::One;
        ps.blend_dst_alpha = BlendFactor::OneMinusSrcAlpha;
        ps.depth_test = true;
        ps.depth_mask = false;
        ps.cull_face = false;
        let mut vb = VaoBuilder::new(ctx);
        #[rustfmt::skip]
        vb.add_f32(0, AttrKind::F32x3, &[
            -0.5, 0.0, -0.5,  0.5, 0.0, -0.5,  -0.5, 0.0, 0.5,  0.5, 0.0, 0.5,
        ]);
        let plane_vao = vb.build()?;
        undo.push(move |c| c.destroy_vertex_array(plane_vao));

        // The casters: depth only, both faces, the nearest to the ground
        // kept.
        let mut cs = RenderState::new();
        cs.depth_test = true;
        cs.depth_func = DepthFunc::Less;
        cs.cull_face = false;
        let ground = Self {
            settings: GroundShadow::default(),
            size: 0,
            depth: TextureId::NULL,
            depth_fb: FramebufferId::NULL,
            dark: [TextureId::NULL; 2],
            dark_fb: [FramebufferId::NULL; 2],
            pass: Pass::to_default(),
            ubo,
            group,
            caster_state: ctx.render_state(&cs),
            darkness,
            darkness_layout,
            darkness_group,
            blur,
            blur_layout,
            blur_groups,
            fullscreen_state: ctx.render_state(&fs),
            plane_program,
            plane_layout,
            plane_group,
            plane_state: ctx.render_state(&ps),
            plane_vao,
            view: None,
            drawn: None,
            dirty: false,
        };
        undo.keep();
        Ok(ground)
    }

    /// The frame group of the casters' pass.
    pub fn group(&self) -> ParamGroupId {
        self.group
    }

    /// This frame: seen from `view` (None: nothing to shadow), its casters'
    /// signature `signature` (drawn again when it changed). Points the
    /// casters' pass at the depth texture, made or remade at the settings'
    /// size.
    pub fn prepare(&mut self, ctx: &Context, view: Option<(ShadowView, Mat4)>, signature: u64) {
        self.pass.clear_draws();
        self.view = view;
        self.dirty = false;
        let Some((v, _)) = view else {
            return;
        };
        let size = self.settings.size.clamp(64, 2048).next_power_of_two();
        if size != self.size || self.depth.is_null() {
            if let Err(e) = self.allocate(ctx, size) {
                rswebgl::console::error_once(&format!("[rswebgl-scene] ground shadow: {e}"));
                self.view = None;
                return;
            }
            self.drawn = None;
        }
        self.dirty = self.drawn != Some(signature);
        self.drawn = Some(signature);
        ctx.uniform_buffer_set_mat(self.ubo, "u_view", &Mat4::IDENTITY.to_cols_array());
        ctx.uniform_buffer_set_mat(self.ubo, "u_proj", &v.view_proj.to_cols_array());
        ctx.uniform_buffer_set_mat(self.ubo, "u_view_proj", &v.view_proj.to_cols_array());
        ctx.uniform_buffer_set_vec4(self.ubo, "u_camera_pos", 0.0, 0.0, 0.0, 1.0);
        let side = self.size as i32;
        self.pass.set_target(self.depth_fb);
        self.pass.set_viewport(&Viewport::new(0, 0, side, side));
        self.pass.set_clear(&ClearMask::depth());
    }

    pub fn plane_program(&self) -> ProgramId {
        self.plane_program
    }

    /// Whether the maps are drawn again this frame.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Keeps what it holds this frame (frozen shadows).
    pub fn hold(&mut self) {
        self.pass.clear_draws();
        self.dirty = false;
    }

    /// Draws them again after a change no signature sees.
    pub fn invalidate(&mut self) {
        self.drawn = None;
    }

    /// The casters' depth, then the darkness and its blurs, if they changed.
    pub fn render(&self, ctx: &Context) {
        if !self.dirty || self.view.is_none() {
            return;
        }
        ctx.render(&self.pass);
        let s = &self.settings;
        let reverse = if ctx.reverse_z() { 1.0 } else { 0.0 };
        ctx.param_set_texture_by_name(self.darkness_group, "u_depth", self.depth);
        ctx.param_set_floats_by_name(
            self.darkness_group,
            "u_ground_params",
            &[s.darkness, reverse, 0.0, 0.0],
        );
        self.fullscreen(ctx, self.darkness, self.darkness_group, self.dark_fb[0]);
        // dark 0 → 1 across, 1 → 0 down; again at 0.4 of the radius.
        for (i, (radius, across)) in [
            (s.blur, true),
            (s.blur, false),
            (s.blur * 0.4, true),
            (s.blur * 0.4, false),
        ]
        .into_iter()
        .enumerate()
        {
            let step = radius / 256.0;
            let step = if across {
                Vec2::new(step, 0.0)
            } else {
                Vec2::new(0.0, step)
            };
            let (src, dst) = if across { (0, 1) } else { (1, 0) };
            let g = self.blur_groups[i];
            ctx.param_set_texture_by_name(g, "u_src", self.dark[src]);
            ctx.param_set_floats_by_name(g, "u_step", &step.to_array());
            self.fullscreen(ctx, self.blur, g, self.dark_fb[dst]);
        }
    }

    /// The plane to draw this frame, if any.
    pub fn plane(&self, ctx: &Context) -> Option<GroundPlane> {
        let (v, model) = self.view?;
        if ctx.program_link_pending(self.plane_program) {
            return None;
        }
        let g = self.plane_group;
        ctx.param_set_texture_by_name(g, "u_ground", self.dark[0]);
        ctx.param_set_floats_by_name(g, "u_ground_map", &v.to_map.to_cols_array());
        ctx.param_set_floats_by_name(g, "u_ground_opacity", &[self.settings.opacity]);
        Some(GroundPlane {
            program: self.plane_program,
            state: self.plane_state,
            vao: self.plane_vao,
            material: g,
            model,
            command: DrawCommand::arrays(DrawMode::TriangleStrip, 0, 4),
        })
    }

    fn fullscreen(
        &self,
        ctx: &Context,
        program: ProgramId,
        group: ParamGroupId,
        fb: FramebufferId,
    ) {
        let side = self.size as i32;
        ctx.render(&crate::fullscreen::pass(
            fb,
            (side, side),
            program,
            self.fullscreen_state,
            group,
        ));
    }

    // The depth and darkness textures and their framebuffers, `size`².
    fn allocate(&mut self, ctx: &Context, size: u32) -> Result<(), Error> {
        self.free_textures(ctx);
        let side = size as i32;
        let depth = ctx.create_texture(
            TextureTarget::Texture2D,
            TextureMinFilter::Nearest,
            TextureMagFilter::Nearest,
        )?;
        ctx.texture_storage_2d(depth, 1, &TextureFormat::depth24(), side, side);
        self.depth = depth;
        self.depth_fb = ctx.create_framebuffer(side, side)?;
        ctx.framebuffer_attach_depth_texture(self.depth_fb, depth, 0);
        for i in 0..2 {
            let t = ctx.create_texture(
                TextureTarget::Texture2D,
                TextureMinFilter::Linear,
                TextureMagFilter::Linear,
            )?;
            ctx.texture_storage_2d(t, 1, &TextureFormat::r8(), side, side);
            ctx.texture_set_wrap_s(t, TextureWrap::ClampToEdge);
            ctx.texture_set_wrap_t(t, TextureWrap::ClampToEdge);
            self.dark[i] = t;
            let fb = ctx.create_framebuffer(side, side)?;
            ctx.framebuffer_attach_color_texture_2d(fb, 0, t, 0);
            self.dark_fb[i] = fb;
        }
        self.size = size;
        Ok(())
    }

    fn free_textures(&mut self, ctx: &Context) {
        for fb in [self.depth_fb, self.dark_fb[0], self.dark_fb[1]] {
            if !fb.is_null() {
                ctx.destroy_framebuffer(fb);
            }
        }
        for t in [self.depth, self.dark[0], self.dark[1]] {
            if !t.is_null() {
                ctx.destroy_texture(t);
            }
        }
        self.depth = TextureId::NULL;
        self.depth_fb = FramebufferId::NULL;
        self.dark = [TextureId::NULL; 2];
        self.dark_fb = [FramebufferId::NULL; 2];
        self.size = 0;
    }

    /// Frees it all; after a context loss (`lost`) only the CPU side.
    pub fn destroy(mut self, ctx: &Context, lost: bool) {
        ctx.destroy_param_group(self.group);
        ctx.destroy_param_group(self.darkness_group);
        for g in self.blur_groups {
            ctx.destroy_param_group(g);
        }
        ctx.destroy_param_group(self.plane_group);
        ctx.destroy_param_layout(self.darkness_layout);
        ctx.destroy_param_layout(self.blur_layout);
        ctx.destroy_param_layout(self.plane_layout);
        if lost {
            return;
        }
        self.free_textures(ctx);
        ctx.destroy_uniform_buffer(self.ubo);
        ctx.destroy_program(self.darkness);
        ctx.destroy_program(self.blur);
        ctx.destroy_program(self.plane_program);
        ctx.destroy_vertex_array(self.plane_vao);
    }
}
