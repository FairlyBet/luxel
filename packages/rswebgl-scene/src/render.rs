//! Scene → draw list: `SceneRenderer` turns entities with a `MeshRenderer`
//! into draws in a `Pass`, with the camera's matrices in a per-frame uniform
//! block and each object's matrices in its own parameter group.

use std::collections::HashMap;

use glam::{Mat3, Mat4, Vec2, Vec3, Vec4};
use rswebgl::buffer::BufferUsage;
use rswebgl::console;
use rswebgl::context::Context;
use rswebgl::draw::{DrawCommand, DrawMode};
use rswebgl::error::Error;
use rswebgl::ids::{
    ParamGroupId, ParamLayoutId, ProgramId, StateId, TextureId, UniformBufferId, VertexArrayId,
};
use rswebgl::param::{GROUP_SLOTS, ParamField, ParamLayoutBuilder};
use rswebgl::pass::{KEY_LAST, Pass};
use rswebgl::render_state::RenderState;
use rswebgl::uniform_buffer::{UboLayout, UniformType};
use rswebgl_ecs::{Entity, Tick, World};

use crate::animation::MorphWeights;
use crate::camera::Camera;
use crate::culling::{Bounds, Frustum};
use crate::environment::{Environment, FULLSCREEN_VERT, brdf_lut_texture};
use crate::gpu_pick::{GpuPicker, PickView};
use crate::instances::{Batches, Dest, Instances, ProgramKind};
use crate::lighting::{CasterDraw, LIGHTS_BLOCK, LightingSystem};
use crate::model::{Morph, Skin};
use crate::motion::MotionVectors;
use crate::output::ToneMapping;
use crate::pick::Hit;
use crate::shader::create_scene_program;
use crate::transform::{Parent, WorldMatrix};

/// Name of the per-frame uniform block. Shaders declare it as
///
/// ```glsl
/// layout(std140) uniform Frame {
///     mat4 u_view;
///     mat4 u_proj;
///     mat4 u_view_proj;
///     vec4 u_camera_pos; // xyz; w = 1
/// };
/// ```
///
/// The block binds by name; declare all four members, in this order.
pub const FRAME_BLOCK: &str = "Frame";
/// Per-object uniforms, set in group slot 2: `uniform mat4 u_model;` and
/// `uniform mat3 u_normal_matrix;` (inverse-transpose of the model's upper
/// 3×3). A shader may declare either or neither. Shaders on the scene
/// prelude read them from the instance texture in the vertex stage instead
/// (`INSTANCE_BASE_UNIFORM`), so identical meshes batch into one instanced
/// draw; one that also reads them in its fragment stage draws each object
/// on its own.
pub const MODEL_UNIFORM: &str = "u_model";
pub const NORMAL_MATRIX_UNIFORM: &str = "u_normal_matrix";
/// Per-object, set in group slot 2: `uniform uint u_object_id;` — the
/// entity (its raw handle), for picking.
pub const OBJECT_ID_UNIFORM: &str = "u_object_id";
/// Per draw, set in group slot 2 for shaders that read their object from the
/// instance texture (the scene prelude, `object_instanced.glsl`): where this
/// draw's instances start.
pub const INSTANCE_BASE_UNIFORM: &str = "u_instance_base";
/// Per object, for a skinned mesh: the instance texture slot of its first
/// joint matrix (world(joint) × inverse bind).
pub const JOINT_BASE_UNIFORM: &str = "u_joint_base";
/// Per object, for a mesh with morph targets: the slot its weights start at
/// (four a texel), next to `u_morph_count` and the `u_morph_targets` texture.
pub const MORPH_BASE_UNIFORM: &str = "u_morph_base";
/// The frame's instance texture (slot 0): three RGBA32F texels per instance.
pub const INSTANCES_TEXTURE: &str = "u_instances";

/// Name of the lighting block, bound next to the frame block in slot 0
/// (the lights themselves are in `LIGHTS_BLOCK`):
///
/// ```glsl
/// layout(std140) uniform Lighting {
///     vec4 u_environment; // rgb intensity of the environment light, w the
///                         // material maps' LOD bias
///     vec4 u_lighting;    // x unused, y exposure, z environment rotation
///                         // (radians about +Y), w last specular mip
///     vec4 u_output;      // x shows (1) or leaves linear radiance (0), y tone
///                         // mapper, z 1 with a scene copy, w its last mip
///     vec4 u_shading;     // x specular AA variance, y its threshold
/// };
/// uniform samplerCube u_env_diffuse;  // irradiance / π
/// uniform samplerCube u_env_specular; // prefiltered: mip = roughness × w
/// uniform sampler2D u_env_brdf;       // split-sum (scale, bias) by (n·v, roughness)
/// uniform float u_background_blur;    // the background's mip, 0..1 of w
/// ```
pub const LIGHTING_BLOCK: &str = "Lighting";

// Intensity of the default (white, constant) environment: a dim ambient light,
// enough to see a scene with no lights.
const DEFAULT_ENV_INTENSITY: f32 = 0.2;

const SKYBOX_FRAG: &str = include_str!("../shaders/skybox.frag");

/// Group slots the scene fills: 0 frame, 1 material, 2 object.
pub const FRAME_SLOT: usize = 0;
pub const MATERIAL_SLOT: usize = 1;
pub const OBJECT_SLOT: usize = 2;

/// Draws the entity's mesh. Everything is a renderer handle: the scene neither
/// creates nor destroys them.
#[derive(Debug, Clone, Copy)]
pub struct MeshRenderer {
    pub program: ProgramId,
    pub state: StateId,
    pub vao: VertexArrayId,
    /// Slot-1 group (NULL = none). Draws are sorted by it after program and
    /// state, so objects sharing a material draw together.
    pub material: ParamGroupId,
    pub command: DrawCommand,
    /// Blended: drawn after everything opaque, farthest first (by the
    /// distance to its bounds' center, or its origin without `Bounds`).
    pub transparent: bool,
    /// Refracts what is behind it (transmission): drawn after the opaque
    /// scene, which it sees through (with HDR output).
    pub transmissive: bool,
}

/// Hides the entity and everything under it when false. No component =
/// visible. A hidden ancestor wins: `Visible(true)` doesn't show a child of a
/// hidden parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Visible(pub bool);

/// A mesh whose GPU data is being rebuilt (its model reloading after a
/// context loss): left out of frames, counted as pending, until removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reloading;

/// What the last `SceneRenderer::render` did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RenderStats {
    /// Meshes drawn (an instanced draw counts each of its instances).
    pub drawn: usize,
    /// Draw calls pushed into the pass: fewer than `drawn` when identical
    /// meshes were batched.
    pub draws: usize,
    /// Left out by frustum culling.
    pub culled: usize,
    /// Object groups whose matrices were written (moved or new objects).
    pub objects_written: usize,
    /// Transmissive meshes drawn (they want the scene behind them).
    pub transmissive: usize,
    /// Visible meshes left out because their shader is still compiling in
    /// the background (parallel compile); they appear once it links.
    pub pending: usize,
    /// Meshes drawn as instances of a batch (several identical meshes in
    /// one draw).
    pub instanced: usize,
    /// Meshes that moved since the last frame, given motion vectors (with
    /// motion vectors on: TAA).
    pub moving: usize,
}

// An entity's object group and the `WorldMatrix` change tick it holds. Kept
// per object rather than per run: a culled object isn't written, so it has
// to catch up when it comes back into view.
#[derive(Clone, Copy)]
struct Object {
    group: ParamGroupId,
    written: Tick,
}

/// Builds a pass's draw list from the world.
///
/// Per frame, after `TransformSystem::update`:
///
/// ```ignore
/// transforms.update(&mut world);
/// pass.clear_draws();
/// scene.render(&world, &ctx, camera, &mut pass);
/// pass.sort();
/// ctx.render(&pass);
/// world.trim_removed(transforms.last_run().min(scene.removals_checked()));
/// ```
///
/// Owns one uniform buffer for the frame block and a parameter group per
/// drawn entity (its matrices, rewritten only when its `WorldMatrix`
/// changes). Groups of entities that lost their `MeshRenderer` or were
/// despawned are destroyed on the next `render`. Release everything with
/// `destroy`; after a context loss, build a new `SceneRenderer`.
pub struct SceneRenderer {
    frame_ubo: UniformBufferId,
    lighting_ubo: UniformBufferId,
    // What the lighting block holds, to write it only when it changes.
    lighting: Vec<Vec4>,
    env_intensity: Vec3,
    exposure: f32,
    // Specular antialiasing (variance, threshold).
    specular_aa: (f32, f32),
    // Depth-peeling mode of the passes being rendered (`set_peel`).
    peel: u32,
    // TAA: the projection's subpixel offset (NDC), the texture LOD bias, and
    // the last frame's view-projection without the offset.
    jitter: Vec2,
    mip_bias: f32,
    last_view_proj: Mat4,
    // TAA's motion vectors of moving meshes, while on.
    motion: Option<MotionVectors>,
    // The white constant environment, used while no other is set.
    default_env: Environment,
    env: Option<Environment>,
    env_rotation: f32,
    brdf_lut: TextureId,
    f_env_diffuse: ParamField,
    f_env_specular: ParamField,
    f_background_blur: ParamField,
    // Some(blur): draw the environment behind everything.
    background: Option<f32>,
    // Built on first use; None until then (Err once it failed).
    skybox: Option<Result<ProgramId, ()>>,
    skybox_state: StateId,
    frame_layout: ParamLayoutId,
    frame_group: ParamGroupId,
    object_layout: ParamLayoutId,
    f_model: ParamField,
    f_normal: ParamField,
    f_object_id: ParamField,
    objects: HashMap<Entity, Object>,
    // MeshRenderer's removal count at the last sweep check.
    // Removals logged after it are still to be looked at.
    removals_checked: Tick,
    f_instance_base: ParamField,
    f_joint_base: ParamField,
    f_morph_base: ParamField,
    f_morph_count: ParamField,
    f_morph_targets: ParamField,
    f_instances: ParamField,
    // This frame's instance data and batches; the batches' pooled object
    // groups; how each program gets its object.
    instances: Instances,
    batches: Batches,
    batch_groups: Vec<ParamGroupId>,
    program_kinds: HashMap<ProgramId, ProgramKind>,
    program_peels: HashMap<ProgramId, bool>,
    // Made on the first `pick`.
    picker: Option<GpuPicker>,
    // The context generation its GPU objects belong to.
    generation: u32,
    // Whether scene_output applies exposure, tone mapping and sRGB itself
    // (drawing straight to an 8-bit target) or leaves linear radiance for a
    // display pass.
    display_output: bool,
    tone_mapping: ToneMapping,
    // The opaque scene, copied for transmissive meshes (and its last mip),
    // when the frame has one.
    scene_color: Option<(TextureId, u32)>,
    // The least mip transmissive meshes read it at (`set_scene_color_min_lod`).
    scene_color_min_lod: f32,
    // This frame's view and (unjittered) projection: `drop_jitter` rewrites
    // the frame block from them.
    frame_view: Mat4,
    frame_proj: Mat4,
    stats: RenderStats,
    // The punctual lights and their shadow maps.
    lights: LightingSystem,
    // This frame's batched shadow casters (a draw and its matrices), and
    // their index by map, program, vertex array and material; kept across
    // frames for their allocations.
    shadow_batches: Vec<(CasterDraw, Vec<Mat4>)>,
    shadow_batch_index: HashMap<(usize, u32, u32, u32), Vec<usize>>,
    // This frame: the entities whose instance (and deformers) is written,
    // where it starts.
    frame_bases: HashMap<Entity, i32>,
}

impl SceneRenderer {
    pub fn new(ctx: &Context) -> Result<Self, Error> {
        // What's made so far is freed if a later step fails.
        let mut undo = rswebgl::undo::Undo::new(ctx);
        let frame_ubo = ctx.create_uniform_buffer(
            &UboLayout::new()
                .field("u_view", UniformType::Mat4)
                .field("u_proj", UniformType::Mat4)
                .field("u_view_proj", UniformType::Mat4)
                .field("u_camera_pos", UniformType::Vec4),
            BufferUsage::DynamicDraw,
        )?;
        undo.push(move |c| c.destroy_uniform_buffer(frame_ubo));
        let lighting_ubo = ctx.create_uniform_buffer(
            &UboLayout::new()
                .field("u_environment", UniformType::Vec4)
                .field("u_lighting", UniformType::Vec4)
                .field("u_output", UniformType::Vec4)
                .field("u_shading", UniformType::Vec4),
            BufferUsage::DynamicDraw,
        )?;
        undo.push(move |c| c.destroy_uniform_buffer(lighting_ubo));
        let frame_layout = ctx.create_param_layout(
            &ParamLayoutBuilder::new()
                .block(FRAME_BLOCK)
                .block(LIGHTING_BLOCK)
                .block(LIGHTS_BLOCK)
                .texture("u_env_diffuse")
                .texture("u_env_specular")
                .texture("u_env_brdf")
                .texture("u_scene_color")
                .texture(INSTANCES_TEXTURE)
                .texture("u_peel_depth")
                .texture(crate::lighting::SHADOW_MAP)
                .uniform("u_background_blur", UniformType::F32),
        )?;
        undo.push(move |c| c.destroy_param_layout(frame_layout));
        let frame_group = ctx.create_param_group(frame_layout)?;
        undo.push(move |c| c.destroy_param_group(frame_group));
        ctx.param_set_block_by_name(frame_group, FRAME_BLOCK, frame_ubo);
        ctx.param_set_block_by_name(frame_group, LIGHTING_BLOCK, lighting_ubo);
        let object_layout = ctx.create_param_layout(
            &ParamLayoutBuilder::new()
                .uniform(MODEL_UNIFORM, UniformType::Mat4)
                .uniform(NORMAL_MATRIX_UNIFORM, UniformType::Mat3)
                .uniform(OBJECT_ID_UNIFORM, UniformType::U32)
                .uniform(INSTANCE_BASE_UNIFORM, UniformType::I32)
                .uniform(JOINT_BASE_UNIFORM, UniformType::I32)
                .uniform(MORPH_BASE_UNIFORM, UniformType::I32)
                .uniform("u_morph_count", UniformType::I32)
                .texture("u_morph_targets"),
        )?;
        undo.push(move |c| c.destroy_param_layout(object_layout));
        let instances = Instances::new(ctx)?;
        let instance_texture = instances.texture();
        undo.push(move |c| c.destroy_texture(instance_texture));
        ctx.param_set_texture_by_name(frame_group, INSTANCES_TEXTURE, instances.texture());
        let brdf_lut = brdf_lut_texture(ctx)?;
        undo.push(move |c| c.destroy_texture(brdf_lut));
        let default_env = Environment::constant(ctx, Vec3::ONE)?;
        let (env_diffuse, env_specular) = (default_env.diffuse(), default_env.specular());
        undo.push(move |c| {
            c.destroy_texture(env_diffuse);
            c.destroy_texture(env_specular);
        });
        ctx.param_set_texture_by_name(frame_group, "u_env_brdf", brdf_lut);
        // Until a frame grabs the scene: any valid texture (not sampled).
        ctx.param_set_texture_by_name(frame_group, "u_scene_color", brdf_lut);
        ctx.param_set_texture_by_name(frame_group, "u_peel_depth", brdf_lut);
        // Made last: nothing after it can fail.
        let lights = LightingSystem::new(ctx, frame_layout, frame_group, lighting_ubo)?;
        let mut skybox_state = RenderState::new();
        skybox_state.depth_mask = false;
        let renderer = Self {
            frame_ubo,
            lighting_ubo,
            lighting: Vec::new(),
            env_intensity: Vec3::splat(DEFAULT_ENV_INTENSITY),
            exposure: 1.0,
            specular_aa: (0.0, 0.2),
            peel: 0,
            jitter: Vec2::ZERO,
            mip_bias: 0.0,
            last_view_proj: Mat4::IDENTITY,
            motion: None,
            default_env,
            env: None,
            env_rotation: 0.0,
            brdf_lut,
            f_env_diffuse: ctx.param_field(frame_layout, "u_env_diffuse")?,
            f_env_specular: ctx.param_field(frame_layout, "u_env_specular")?,
            f_background_blur: ctx.param_field(frame_layout, "u_background_blur")?,
            background: None,
            skybox: None,
            skybox_state: ctx.render_state(&skybox_state),
            frame_layout,
            frame_group,
            object_layout,
            f_model: ctx.param_field(object_layout, MODEL_UNIFORM)?,
            f_normal: ctx.param_field(object_layout, NORMAL_MATRIX_UNIFORM)?,
            f_object_id: ctx.param_field(object_layout, OBJECT_ID_UNIFORM)?,
            objects: HashMap::new(),
            removals_checked: 0,
            f_instance_base: ctx.param_field(object_layout, INSTANCE_BASE_UNIFORM)?,
            f_joint_base: ctx.param_field(object_layout, JOINT_BASE_UNIFORM)?,
            f_morph_base: ctx.param_field(object_layout, MORPH_BASE_UNIFORM)?,
            f_morph_count: ctx.param_field(object_layout, "u_morph_count")?,
            f_morph_targets: ctx.param_field(object_layout, "u_morph_targets")?,
            f_instances: ctx.param_field(frame_layout, INSTANCES_TEXTURE)?,
            instances,
            batches: Batches::default(),
            batch_groups: Vec::new(),
            program_kinds: HashMap::new(),
            program_peels: HashMap::new(),
            picker: None,
            generation: ctx.generation(),
            display_output: true,
            tone_mapping: ToneMapping::Neutral,
            scene_color: None,
            scene_color_min_lod: 0.0,
            frame_view: Mat4::IDENTITY,
            frame_proj: Mat4::IDENTITY,
            stats: RenderStats::default(),
            lights,
            shadow_batches: Vec::new(),
            shadow_batch_index: HashMap::new(),
            frame_bases: HashMap::new(),
        };
        renderer.bind_environment(ctx);
        undo.keep();
        Ok(renderer)
    }

    /// The uniform buffer behind the frame block — e.g. to bind it into other
    /// passes' groups.
    pub fn frame_ubo(&self) -> UniformBufferId {
        self.frame_ubo
    }

    /// The slot-0 group every scene draw uses.
    pub fn frame_group(&self) -> ParamGroupId {
        self.frame_group
    }

    /// The object group of `e`, once it has been drawn.
    pub fn object_group(&self, e: Entity) -> Option<ParamGroupId> {
        self.objects.get(&e).map(|o| o.group)
    }

    pub fn stats(&self) -> RenderStats {
        self.stats
    }

    /// Scales the environment's light (linear RGB). With the default white
    /// environment this is a flat ambient light; default 0.2.
    pub fn set_environment_intensity(&mut self, rgb: Vec3) {
        self.env_intensity = rgb;
    }

    pub fn environment_intensity(&self) -> Vec3 {
        self.env_intensity
    }

    /// Lights the scene with `env` (and shows it as the background if that's
    /// on), taking it over; the previous one is destroyed. Resets the
    /// intensity to 1.
    pub fn set_environment(&mut self, ctx: &Context, env: Environment) {
        if let Some(old) = self.env.replace(env) {
            old.destroy(ctx);
        }
        self.env_intensity = Vec3::ONE;
        self.bind_environment(ctx);
    }

    /// Back to the default white environment at intensity 0.2.
    pub fn clear_environment(&mut self, ctx: &Context) {
        if let Some(old) = self.env.take() {
            old.destroy(ctx);
        }
        self.env_intensity = Vec3::splat(DEFAULT_ENV_INTENSITY);
        self.bind_environment(ctx);
    }

    pub fn has_environment(&self) -> bool {
        self.env.is_some()
    }

    /// Turns the environment about +Y (radians).
    pub fn set_environment_rotation(&mut self, radians: f32) {
        self.env_rotation = radians;
    }

    /// Draws the environment behind the scene, `blur` 0 (sharp) … 1 (the
    /// roughest prefiltered level); None turns it off (the pass's clear
    /// color shows).
    pub fn set_background(&mut self, blur: Option<f32>) {
        self.background = blur.map(|b| b.clamp(0.0, 1.0));
    }

    fn environment(&self) -> &Environment {
        self.env.as_ref().unwrap_or(&self.default_env)
    }

    fn bind_environment(&self, ctx: &Context) {
        let env = self.environment();
        ctx.param_set_texture(self.frame_group, self.f_env_diffuse, env.diffuse());
        ctx.param_set_texture(self.frame_group, self.f_env_specular, env.specular());
    }

    /// The background shader, built on first use (`set_background` callers can
    /// build it early, so it compiles while other things load). None if it
    /// failed to build (reported once).
    // The background shader if it has been built.
    pub(crate) fn built_background_program(&self) -> Option<ProgramId> {
        match self.skybox {
            Some(Ok(p)) => Some(p),
            _ => None,
        }
    }

    pub fn background_program(&mut self, ctx: &Context) -> Option<ProgramId> {
        let program = *self.skybox.get_or_insert_with(|| {
            create_scene_program(ctx, FULLSCREEN_VERT, SKYBOX_FRAG).map_err(|e| {
                console::error(&format!(
                    "[rswebgl-scene] background shader failed to build: {e}"
                ))
            })
        });
        program.ok()
    }

    // The background draw: first in the pass (key 0, pushed before any
    // mesh), not writing depth.
    fn push_background(&mut self, ctx: &Context, pass: &mut Pass) {
        let Some(blur) = self.background else {
            return;
        };
        let Some(program) = self.background_program(ctx) else {
            return;
        };
        // Still compiling: no background this frame rather than a stall.
        if ctx.program_link_pending(program) {
            return;
        }
        ctx.param_set_f32(self.frame_group, self.f_background_blur, blur);
        let mut groups = [ParamGroupId::NULL; GROUP_SLOTS];
        groups[FRAME_SLOT] = self.frame_group;
        pass.push_groups_with_key(
            0,
            program,
            self.skybox_state,
            VertexArrayId::NULL,
            groups,
            &DrawCommand::arrays(DrawMode::Triangles, 0, 3),
        );
    }

    /// The opaque scene as transmissive meshes see it through themselves:
    /// a texture with mips (`last_mip` its last level), or None to fall back
    /// to the environment. Set by the frame that copies it (see
    /// `HdrOutput::grab`), before drawing them.
    pub fn set_scene_color(&mut self, ctx: &Context, scene: Option<(TextureId, u32)>) {
        if self.scene_color == scene {
            return;
        }
        self.scene_color = scene;
        let tex = scene.map_or(self.brdf_lut, |(t, _)| t);
        ctx.param_set_texture_by_name(self.frame_group, "u_scene_color", tex);
    }

    /// Where `scene_output` sends shaded colors: `display` true (the default)
    /// applies exposure, `tone_mapping` and sRGB in each shader, for drawing
    /// straight to the canvas; false leaves linear radiance (HDR) for a
    /// display pass to finish (`HdrOutput`).
    pub fn set_output(&mut self, display: bool, tone_mapping: ToneMapping) {
        self.display_output = display;
        self.tone_mapping = tone_mapping;
    }

    /// Scales scene radiance before tone mapping. Default 1.
    pub fn set_exposure(&mut self, exposure: f32) {
        self.exposure = exposure;
    }

    pub fn exposure(&self) -> f32 {
        self.exposure
    }

    /// TAA's subpixel offset of the next frames' projection, in NDC (0: none),
    /// and the LOD bias of material maps.
    pub fn set_jitter(&mut self, jitter: Vec2, mip_bias: f32) {
        self.jitter = jitter;
        self.mip_bias = mip_bias;
    }

    /// Ends the jitter for the rest of this frame (TAA: what draws after the
    /// resolve): the frame block's projection and the maps' LOD bias back
    /// to the unjittered ones, for the passes rendered from now.
    pub fn drop_jitter(&mut self, ctx: &Context) {
        if self.jitter == Vec2::ZERO && self.mip_bias == 0.0 {
            return;
        }
        self.jitter = Vec2::ZERO;
        self.mip_bias = 0.0;
        let (view, proj) = (self.frame_view, self.frame_proj);
        ctx.ubo_set_mat(self.frame_ubo, "u_proj", &proj.to_cols_array());
        ctx.ubo_set_mat(
            self.frame_ubo,
            "u_view_proj",
            &(proj * view).to_cols_array(),
        );
        // The cached block too, so the next frame's bias rewrites it.
        if let Some(env) = self.lighting.first_mut() {
            env.w = 0.0;
            let e = *env;
            ctx.ubo_set_vec4(self.lighting_ubo, "u_environment", e.x, e.y, e.z, e.w);
        }
    }

    /// The least mip transmissive meshes read the scene copy at (0: as
    /// their roughness says): 1 where the copy is aliased (no MSAA, no TAA),
    /// so subpixel detail behind glass (a filament) shows as a soft line,
    /// not dashes.
    pub fn set_scene_color_min_lod(&mut self, lod: f32) {
        self.scene_color_min_lod = lod.max(0.0);
    }

    /// The projection's subpixel offset (NDC) set by `set_jitter`.
    pub fn jitter(&self) -> Vec2 {
        self.jitter
    }

    /// Motion vectors for moving meshes (TAA) on or off: while on, each
    /// frame notes the opaque meshes' state, and `push_motion` draws those
    /// that moved.
    pub fn set_motion_vectors(&mut self, ctx: &Context, on: bool) {
        if on == self.motion.is_some() {
            return;
        }
        if let Some(m) = self.motion.take() {
            m.destroy(ctx, self.is_lost(ctx));
        }
        if on {
            match MotionVectors::new(ctx) {
                Ok(m) => self.motion = Some(m),
                Err(err) => console::error_once(&format!("[rswebgl-scene] motion vectors: {err}")),
            }
        }
    }

    /// The motion of meshes that moved since the last frame (`render` with
    /// motion vectors on) into `pass`, tested against `scene_depth`: uv
    /// now − uv last frame per pixel. Returns the draws added: none if
    /// nothing moved.
    pub fn push_motion(&mut self, ctx: &Context, pass: &mut Pass, scene_depth: TextureId) -> usize {
        let (frame, jitter) = (self.frame_group, self.jitter);
        match &mut self.motion {
            Some(m) if m.any() => m.push_draws(ctx, pass, frame, scene_depth, jitter),
            _ => 0,
        }
    }

    /// The motion-vector shaders built so far (`set_motion_vectors`).
    pub(crate) fn motion_programs(&self) -> Vec<ProgramId> {
        self.motion
            .as_ref()
            .map_or_else(Vec::new, |m| m.programs().collect())
    }

    /// The view-projection of the last frame drawn, without the jitter.
    pub fn last_view_proj(&self) -> Mat4 {
        self.last_view_proj
    }

    /// Depth peeling for the passes rendered next: mode 0 off, 1 keeps
    /// only fragments beyond the layer in `depth` (peels the next layer), 2
    /// only those on it (shades it). Written at once (between renders of
    /// already built passes).
    pub fn set_peel(&mut self, ctx: &Context, mode: u32, depth: TextureId) {
        self.peel = mode;
        let reverse = if ctx.reverse_z() { 1.0 } else { 0.0 };
        let (v, t) = self.specular_aa;
        ctx.ubo_set_vec4(self.lighting_ubo, "u_shading", v, t, mode as f32, reverse);
        if let Some(cached) = self.lighting.last_mut() {
            *cached = Vec4::new(v, t, mode as f32, reverse);
        }
        // Off: back to a texture that lives as long as the renderer (a layer's
        // goes with its target, and a destroyed one would leave whatever
        // that unit last held bound).
        let depth = if depth.is_null() {
            self.brdf_lut
        } else {
            depth
        };
        ctx.param_set_texture_by_name(self.frame_group, "u_peel_depth", depth);
    }

    /// Geometric specular antialiasing (Kaplanyan, Tokuyoshi): where the
    /// normal turns within a pixel, roughness grows by up to `threshold` (in
    /// α²), `variance` scaling how fast. Higher calms the sparkle of sharp
    /// highlights on curved, smooth surfaces and softens highlights more;
    /// variance 0 turns it off. Default off (0, 0.2): it blurred more than
    /// it calmed (Filament's default is 0.15, 0.2).
    pub fn set_specular_antialiasing(&mut self, variance: f32, threshold: f32) {
        self.specular_aa = (variance.max(0.0), threshold.clamp(0.0, 1.0));
    }

    pub fn specular_antialiasing(&self) -> (f32, f32) {
        self.specular_aa
    }

    /// Appends the visible `MeshRenderer`s, as seen from `camera`, to `pass`.
    /// Doesn't clear or sort the pass. An entity without a `WorldMatrix` yet
    /// (run `TransformSystem::update` first) is skipped. Identical opaque
    /// meshes of a shader that reads its instance become one instanced draw;
    /// their matrices go to the instance texture, which the next call
    /// rewrites — render the pass first.
    pub fn render(
        &mut self,
        world: &World,
        ctx: &Context,
        camera: Entity,
        pass: &mut Pass,
    ) -> RenderStats {
        self.render_split(world, ctx, camera, pass, None, None, None)
    }

    /// `render`, with transmissive and transparent meshes into `late`: drawn
    /// after `opaque` has been rendered (and copied for them to see
    /// through, `set_scene_color`). With `see_through`, the transparent
    /// (not transmissive) ones also into it: drawn into that copy too, so
    /// glass shows them (as Khronos' renderer does).
    #[allow(clippy::too_many_arguments)]
    pub fn render_split(
        &mut self,
        world: &World,
        ctx: &Context,
        camera: Entity,
        opaque: &mut Pass,
        mut late: Option<&mut Pass>,
        mut glass: Option<&mut Pass>,
        mut see_through: Option<&mut Pass>,
    ) -> RenderStats {
        let _span = tracing::info_span!("scene.collect", meshes = tracing::field::Empty).entered();
        self.stats = RenderStats::default();
        self.release_removed(world, ctx);
        self.instances.clear();
        self.batches.clear();
        if let Some(m) = &mut self.motion {
            m.begin(self.last_view_proj);
        }

        let Some((view, view_proj)) = self.write_frame(world, ctx, camera) else {
            return self.stats;
        };
        self.frame_bases.clear();
        let view_camera = world
            .get::<Camera>(camera)
            .zip(world.get::<WorldMatrix>(camera))
            .map(|(c, m)| (*c, m.0));
        self.lights
            .prepare(world, ctx, self.frame_group, view_camera);
        // Shadow casters still linking count as pending meshes: the frame
        // isn't finished until they cast.
        self.stats.pending += self.lights.waiting();
        self.write_lighting(ctx);
        self.push_background(ctx, opaque);
        let frustum = Frustum::from_view_proj_depth(&view_proj, ctx.reverse_z());

        let (Some(meshes), Some(worlds)) = (
            world.storage::<MeshRenderer>(),
            world.storage::<WorldMatrix>(),
        ) else {
            return self.stats;
        };
        let bounds = world.storage::<Bounds>();
        let reloading = world.storage::<Reloading>().filter(|r| !r.is_empty());
        // Ancestors are walked per mesh only while something is hidden.
        let any_hidden = world
            .storage::<Visible>()
            .is_some_and(|v| v.data().iter().any(|v| !v.0));
        for (e, mesh) in meshes.iter() {
            if any_hidden && hidden(world, e) {
                continue;
            }
            let Some(row) = worlds.row(e) else {
                continue;
            };
            let m = worlds.data()[row as usize].0;
            let mut center = m.w_axis.truncate();
            if let Some(b) = bounds.and_then(|b| b.get(e)) {
                let (c, h) = b.transformed(&m);
                if !frustum.intersects(c, h) {
                    self.stats.culled += 1;
                    continue;
                }
                center = c;
            }

            // A shader still linking would stall the frame: skip the mesh until
            // it is done. Likewise one whose model is reloading.
            if reloading.is_some_and(|r| r.contains(e)) || ctx.program_link_pending(mesh.program) {
                self.stats.pending += 1;
                continue;
            }

            if mesh.transmissive {
                self.stats.transmissive += 1;
            }
            // Peeled only if its shader takes part (calls peel_discard, as the
            // PBR shader does): another would draw whole in every layer pass.
            let dest = if glass.is_some()
                && mesh.transmissive
                && !mesh.transparent
                && self.program_peels(ctx, mesh.program)
            {
                Dest::Glass
            } else if late.is_some() && (mesh.transparent || mesh.transmissive) {
                Dest::Late
            } else {
                Dest::Opaque
            };
            if let Some(motion) = &mut self.motion {
                let opaque = dest == Dest::Opaque && !mesh.transparent;
                motion.track(world, e, &m, mesh, opaque, &mut self.instances);
            }
            let kind = self.program_kind(ctx, mesh.program);
            // Opaque meshes of a shader that reads its instance draw
            // together: one instanced draw per program, state, vertex array,
            // material and command.
            // (Not a command that draws its own instances, or can't.)
            let single = mesh.command.instance_count == 1 && mesh.command.is_instanceable();
            if kind == ProgramKind::Batched && !mesh.transparent && single {
                self.batches.add(dest, mesh, &m);
                continue;
            }

            let Some(group) = self.ensure_object_group(ctx, e, &m, worlds.changed_tick(row)) else {
                continue;
            };
            if kind != ProgramKind::Uniforms {
                // An instance of its own — as many copies as the command
                // draws instances (each sees its object, as with uniforms).
                let base = self.instances.push(&m);
                if mesh.command.is_instanceable() {
                    for _ in 1..mesh.command.instance_count.clamp(1, 4096) {
                        self.instances.push(&m);
                    }
                }
                ctx.param_set_i32(group, self.f_instance_base, base);
                self.write_deformers(world, ctx, e, group);
                self.frame_bases.insert(e, base);
            }

            let mut groups = [ParamGroupId::NULL; GROUP_SLOTS];
            groups[FRAME_SLOT] = self.frame_group;
            groups[MATERIAL_SLOT] = mesh.material;
            groups[OBJECT_SLOT] = group;
            let pass: &mut Pass = match (dest, &mut late, &mut glass) {
                (Dest::Glass, _, Some(glass)) => glass,
                (Dest::Late, Some(late), _) => late,
                _ => &mut *opaque,
            };
            // As many copies as have instance slots (4096 at most).
            let mut cmd = mesh.command;
            if kind != ProgramKind::Uniforms && cmd.is_instanceable() {
                cmd.instance_count = cmd.instance_count.clamp(1, 4096);
            }
            let mesh = &MeshRenderer {
                command: cmd,
                ..*mesh
            };
            if mesh.transparent {
                // After every default key, farthest first: the key grows as
                // the distance shrinks (a non-negative float's bits order
                // like the float).
                let distance = (-view.transform_point3(center).z).max(0.0);
                let key = KEY_LAST | u64::from(u32::MAX - distance.to_bits());
                if dest == Dest::Late
                    && !mesh.transmissive
                    && let Some(copy) = see_through.as_deref_mut()
                {
                    copy.push_groups_with_key(
                        key,
                        mesh.program,
                        mesh.state,
                        mesh.vao,
                        groups,
                        &mesh.command,
                    );
                }
                pass.push_groups_with_key(
                    key,
                    mesh.program,
                    mesh.state,
                    mesh.vao,
                    groups,
                    &mesh.command,
                );
            } else {
                pass.push_groups(mesh.program, mesh.state, mesh.vao, groups, &mesh.command);
            }
            self.stats.drawn += 1;
            self.stats.draws += 1;
        }

        // The batches: their instances after the single ones, a pooled group
        // each for where they start.
        let batches = std::mem::take(&mut self.batches);
        let mut batch_groups_used = 0;
        for (i, batch) in batches.iter().enumerate() {
            batch_groups_used = i + 1;
            let base = self.instances.len();
            for m in &batch.matrices {
                self.instances.push(m);
            }
            let Some(group) = self.batch_group(ctx, i) else {
                break;
            };
            ctx.param_set_i32(group, self.f_instance_base, base);
            let mut groups = [ParamGroupId::NULL; GROUP_SLOTS];
            groups[FRAME_SLOT] = self.frame_group;
            groups[MATERIAL_SLOT] = batch.mesh.material;
            groups[OBJECT_SLOT] = group;
            let mut command = batch.mesh.command;
            command.instance_count = batch.matrices.len() as i32;
            let pass: &mut Pass = match (batch.dest, &mut late, &mut glass) {
                (Dest::Glass, _, Some(glass)) => glass,
                (Dest::Late, Some(late), _) => late,
                _ => &mut *opaque,
            };
            let mesh = &batch.mesh;
            pass.push_groups(mesh.program, mesh.state, mesh.vao, groups, &command);
            self.stats.drawn += batch.matrices.len();
            self.stats.draws += 1;
            self.stats.instanced += batch.matrices.len();
        }
        self.batches = batches;
        self.batches.clear();
        self.stats.moving = self.motion.as_ref().map_or(0, MotionVectors::count);
        let batch_groups_used =
            batch_groups_used + self.collect_shadows(world, ctx, batch_groups_used);
        // The ground shadow's plane: blended after the opaque scene.
        if let Some(plane) = self.lights.ground_plane(ctx)
            && let Some(group) = self.batch_group(ctx, batch_groups_used)
        {
            let base = self.instances.push(&plane.model);
            ctx.param_set_i32(group, self.f_instance_base, base);
            let mut groups = [ParamGroupId::NULL; GROUP_SLOTS];
            groups[FRAME_SLOT] = self.frame_group;
            groups[MATERIAL_SLOT] = plane.material;
            groups[OBJECT_SLOT] = group;
            let distance = (-view.transform_point3(plane.model.w_axis.truncate()).z).max(0.0);
            let key = KEY_LAST | u64::from(u32::MAX - distance.to_bits());
            let pass: &mut Pass = match &mut late {
                Some(late) => late,
                None => &mut *opaque,
            };
            pass.push_groups_with_key(
                key,
                plane.program,
                plane.state,
                plane.vao,
                groups,
                &plane.command,
            );
        }
        _span.record("meshes", self.stats.drawn);
        let _upload = tracing::info_span!("scene.uploadInstances").entered();
        if let Err(err) = self
            .instances
            .upload(ctx, self.frame_group, self.f_instances)
        {
            console::error_once(&format!("[rswebgl-scene] instance data: {err}"));
        }
        let texture = self.instances.texture();
        self.lights.bind_instances(ctx, INSTANCES_TEXTURE, texture);
        self.stats
    }

    /// Renders this frame's shadow maps (after `render` / `render_split`,
    /// before the passes that shade with them).
    pub fn render_shadows(&self, ctx: &Context) {
        self.lights.render_shadows(ctx);
    }

    /// The side of each shadow map, in texels (default 2048; rounded to a
    /// power of two in 256..=8192, within the device's texture size).
    pub fn set_shadow_map_size(&mut self, size: u32) {
        self.lights.set_shadow_map_size(size);
    }

    pub fn shadow_map_size(&self) -> u32 {
        self.lights.shadow_map_size()
    }

    /// 16-bit shadow maps instead of 24-bit: half the memory, coarser
    /// depth steps (fine for a model or a room). Default off.
    pub fn set_shadow_depth16(&mut self, on: bool) {
        self.lights.set_shadow_depth16(on);
    }

    pub fn shadow_depth16(&self) -> bool {
        self.lights.shadow_depth16()
    }

    /// Tints what each shadow cascade shades, to see where they split.
    pub fn set_debug_cascades(&mut self, on: bool) {
        self.lights.set_debug_cascades(on);
    }

    /// Shadow maps drawn this frame.
    pub fn shadow_views(&self) -> usize {
        self.lights.shadow_views()
    }

    /// Draw calls into this frame's shadow maps.
    pub fn shadow_draw_calls(&self) -> usize {
        self.lights.draw_calls()
    }

    /// The programs of the shadow system's own (the ground shadow's plane),
    /// for `whenReady`.
    pub(crate) fn shadow_programs(&self) -> Vec<ProgramId> {
        self.lights.programs()
    }

    /// The projection this frame's opaque pass drew with (TAA's jitter
    /// in it while it lasts).
    pub fn drawn_projection(&self) -> Mat4 {
        let jitter = Mat4::from_cols(
            Vec4::X,
            Vec4::Y,
            Vec4::Z,
            Vec4::new(self.jitter.x, self.jitter.y, 0.0, 1.0),
        );
        jitter * self.frame_proj
    }

    /// The ground shadow on (with these settings) or off (None).
    pub fn set_ground_shadow(&mut self, settings: Option<crate::lighting::GroundShadow>) {
        self.lights.set_ground_shadow(settings);
    }

    pub fn ground_shadow(&self) -> Option<crate::lighting::GroundShadow> {
        self.lights.ground_shadow()
    }

    /// Whether the ground shadow was drawn again this frame.
    pub fn ground_shadow_drawn(&self) -> bool {
        self.lights.ground_drawn()
    }

    /// Shadow maps drawn again this frame (their light or casters changed).
    pub fn shadow_views_drawn(&self) -> usize {
        self.lights.shadow_views_drawn()
    }

    /// Draws every shadow map again (frozen ones too): for changes the
    /// maps can't see, such as a material's alpha cutoff edited in place.
    pub fn invalidate_shadows(&mut self) {
        self.lights.invalidate();
    }

    /// Bakes frozen shadows again next frame (others follow their lights
    /// and casters anyway): after adding, removing or showing what casts.
    pub fn rebake_shadows(&mut self) {
        self.lights.rebake();
    }

    /// Frozen shadows: drawn once, fitted to every caster (not the view),
    /// then kept whatever moves, until unfrozen or invalidated.
    pub fn set_shadows_frozen(&mut self, on: bool) {
        self.lights.set_frozen(on);
    }

    pub fn shadows_frozen(&self) -> bool {
        self.lights.frozen()
    }

    /// The shadow maps' memory, in bytes.
    pub fn shadow_bytes(&self) -> usize {
        self.lights.shadow_bytes()
    }

    // The shadow casters' objects. Identical meshes of a shader that reads
    // its instance draw as one per map (as in the camera's pass): their
    // matrices appended to the instance texture, a pooled batch group each
    // (after the camera's `batch_groups_used`) for where they start. Any
    // other caster draws alone, with the instance the camera's pass wrote,
    // or one written now.
    // Returns the batch groups it used.
    fn collect_shadows(&mut self, world: &World, ctx: &Context, batch_groups_used: usize) -> usize {
        let draws = self.lights.take_caster_draws();
        let Some(worlds) = world.storage::<WorldMatrix>() else {
            self.lights.push_casters(draws, &[]);
            return 0;
        };
        let mut out = Vec::with_capacity(draws.len());
        let mut objects = Vec::with_capacity(draws.len());
        let mut batches = std::mem::take(&mut self.shadow_batches);
        let mut index = std::mem::take(&mut self.shadow_batch_index);
        batches.iter_mut().for_each(|(_, m)| m.clear());
        index.clear();
        let mut used = 0;
        for d in &draws {
            let (e, row) = (d.entity, d.row);
            let m = worlds.data()[row as usize].0;
            let command = &d.mesh.command;
            let single = command.instance_count == 1 && command.is_instanceable();
            if single && self.program_kind(ctx, d.program) == ProgramKind::Batched {
                let key = (
                    d.view,
                    d.program.to_raw(),
                    d.mesh.vao.to_raw(),
                    d.mesh.material.to_raw(),
                );
                let slots = index.entry(key).or_default();
                match slots
                    .iter()
                    .find(|&&i| batches[i].0.mesh.command == *command)
                {
                    Some(&i) => batches[i].1.push(m),
                    None => {
                        if used == batches.len() {
                            batches.push((*d, Vec::new()));
                        }
                        batches[used].0 = *d;
                        batches[used].1.push(m);
                        slots.push(used);
                        used += 1;
                    }
                }
                continue;
            }
            let Some(group) = self.ensure_object_group(ctx, e, &m, worlds.changed_tick(row)) else {
                // Not drawn: its map is told (drawn again next frame).
                out.push(*d);
                objects.push(ParamGroupId::NULL);
                continue;
            };
            if !self.frame_bases.contains_key(&e) {
                let base = self.instances.push(&m);
                if command.is_instanceable() {
                    for _ in 1..command.instance_count.clamp(1, 4096) {
                        self.instances.push(&m);
                    }
                }
                ctx.param_set_i32(group, self.f_instance_base, base);
                self.write_deformers(world, ctx, e, group);
                self.frame_bases.insert(e, base);
            }
            out.push(*d);
            objects.push(group);
        }
        for (i, (d, matrices)) in batches[..used].iter().enumerate() {
            let base = self.instances.len();
            for m in matrices {
                self.instances.push(m);
            }
            let Some(group) = self.batch_group(ctx, batch_groups_used + i) else {
                out.push(*d);
                objects.push(ParamGroupId::NULL);
                continue;
            };
            ctx.param_set_i32(group, self.f_instance_base, base);
            let mut d = *d;
            d.mesh.command.instance_count = matrices.len() as i32;
            out.push(d);
            objects.push(group);
        }
        self.shadow_batches = batches;
        self.shadow_batch_index = index;
        self.lights.push_casters(out, &objects);
        used
    }

    // How `program` gets its object: uniforms (a shader of its own), or the
    // instance texture — batched unless its fragment stage also reads the
    // object uniforms. Asked once the program has linked (the frame skips
    // it before), so the uniform list doesn't wait.
    fn program_kind(&mut self, ctx: &Context, program: ProgramId) -> ProgramKind {
        // Programs come and go (variants, context loss): forget them now
        // and then rather than grow for ever; kinds are cheap to ask again.
        if self.program_kinds.len() > 1024 {
            self.program_kinds.clear();
        }
        if let Some(&kind) = self.program_kinds.get(&program) {
            return kind;
        }
        let names = ctx.program_active_uniform_names(program);
        let has = |n: &str| names.iter().any(|x| x == n);
        // A deforming shader (skin, morph targets) reads this object's own
        // joints or weights: one draw per object.
        let kind = if has(JOINT_BASE_UNIFORM) || has(MORPH_BASE_UNIFORM) {
            ProgramKind::Single
        } else if !has(INSTANCE_BASE_UNIFORM) {
            ProgramKind::Uniforms
        } else if has(MODEL_UNIFORM) || has(NORMAL_MATRIX_UNIFORM) || has(OBJECT_ID_UNIFORM) {
            ProgramKind::Single
        } else {
            ProgramKind::Batched
        };
        self.program_kinds.insert(program, kind);
        kind
    }

    // A skinned or morphing mesh's joint matrices and morph weights into the
    // instance texture, and where they start into its object group.
    fn write_deformers(&mut self, world: &World, ctx: &Context, e: Entity, group: ParamGroupId) {
        if let Some(skin) = world.get::<Skin>(e) {
            let base = self.instances.len();
            for (j, &joint) in skin.joints.iter().enumerate() {
                let w = world
                    .get::<WorldMatrix>(joint)
                    .map_or(Mat4::IDENTITY, |w| w.0);
                let ibm = skin.inverse_bind.get(j).copied().unwrap_or(Mat4::IDENTITY);
                self.instances.push(&(w * ibm));
            }
            ctx.param_set_i32(group, self.f_joint_base, base);
        }
        if let Some(morph) = world.get::<Morph>(e) {
            // The weights are the node's (the primitive's parent).
            let node = world.get::<Parent>(e).map(|p| p.0);
            let weights = node.and_then(|n| world.get::<MorphWeights>(n));
            let base = self.instances.push_floats(
                weights.map_or(&[][..], |w| &w.0[..]),
                morph.targets as usize,
            );
            ctx.param_set_i32(group, self.f_morph_base, base);
            ctx.param_set_i32(group, self.f_morph_count, morph.targets as i32);
            ctx.param_set_texture(group, self.f_morph_targets, morph.texture);
        }
    }

    // Whether `program` reads the peeled depth (its fragment stage calls
    // peel_discard). Asked once linked, like `program_kind`.
    fn program_peels(&mut self, ctx: &Context, program: ProgramId) -> bool {
        if self.program_peels.len() > 1024 {
            self.program_peels.clear();
        }
        *self.program_peels.entry(program).or_insert_with(|| {
            ctx.program_active_uniform_names(program)
                .iter()
                .any(|n| n == "u_peel_depth")
        })
    }

    // The `i`th pooled object group for a batch (made on first need).
    fn batch_group(&mut self, ctx: &Context, i: usize) -> Option<ParamGroupId> {
        while self.batch_groups.len() <= i {
            match ctx.create_param_group(self.object_layout) {
                Ok(g) => self.batch_groups.push(g),
                Err(err) => {
                    console::error_once(&format!(
                        "[rswebgl-scene] can't create a batch group: {err}"
                    ));
                    return None;
                }
            }
        }
        Some(self.batch_groups[i])
    }

    /// Rebuilds the GPU side after a context loss (`Context::generation`
    /// moved on), keeping the settings: environment intensity and rotation,
    /// exposure, specular antialiasing, background. The environment itself is back to the default
    /// white one: set it again (`set_environment`, which resets the
    /// intensity to 1). Object groups are rewritten as meshes are drawn.
    pub fn rebuild(&mut self, ctx: &Context) -> Result<(), Error> {
        let mut fresh = Self::new(ctx)?;
        fresh.env_intensity = self.env_intensity;
        fresh.exposure = self.exposure;
        fresh.specular_aa = self.specular_aa;
        fresh.env_rotation = self.env_rotation;
        fresh.background = self.background;
        fresh.display_output = self.display_output;
        fresh.tone_mapping = self.tone_mapping;
        fresh.lights.keep_settings(&self.lights);
        if matches!(self.skybox, Some(Ok(_))) {
            fresh.background_program(ctx);
        }
        std::mem::replace(self, fresh).destroy(ctx);
        Ok(())
    }

    /// Whether its GPU objects belong to an earlier context generation
    /// (`rebuild` it).
    pub fn is_lost(&self, ctx: &Context) -> bool {
        self.generation != ctx.generation()
    }

    /// Destroys every resource the scene renderer created. Draws already in a
    /// pass then refer to stale groups. After a context loss only the
    /// CPU-side parameter groups are left to free.
    pub fn destroy(self, ctx: &Context) {
        for o in self.objects.into_values() {
            ctx.destroy_param_group(o.group);
        }
        for g in self.batch_groups {
            ctx.destroy_param_group(g);
        }
        if let Some(m) = self.motion {
            m.destroy(ctx, self.generation != ctx.generation());
        }
        self.lights
            .destroy(ctx, self.generation != ctx.generation());
        ctx.destroy_param_group(self.frame_group);
        ctx.destroy_param_layout(self.frame_layout);
        ctx.destroy_param_layout(self.object_layout);
        if self.generation != ctx.generation() {
            if let Some(p) = self.picker {
                p.destroy_lost(ctx);
            }
            return;
        }
        ctx.destroy_uniform_buffer(self.frame_ubo);
        ctx.destroy_uniform_buffer(self.lighting_ubo);
        ctx.destroy_texture(self.brdf_lut);
        self.instances.destroy(ctx);
        if let Some(p) = self.picker {
            p.destroy(ctx);
        }
        self.default_env.destroy(ctx);
        if let Some(env) = self.env {
            env.destroy(ctx);
        }
        if let Some(Ok(p)) = self.skybox {
            ctx.destroy_program(p);
        }
    }

    // Environment intensity, exposure, rotation, output and shading into
    // the lighting block, if anything in it changed since the last frame.
    fn write_lighting(&mut self, ctx: &Context) {
        let last_mip = (self.environment().specular_mips() - 1) as f32;
        let reverse = if ctx.reverse_z() { 1.0 } else { 0.0 };
        let data = vec![
            // w: the material maps' LOD bias (TAA sharpens them back).
            self.env_intensity.extend(self.mip_bias),
            Vec4::new(0.0, self.exposure, self.env_rotation, last_mip),
            Vec4::new(
                if self.display_output { 1.0 } else { 0.0 },
                self.tone_mapping as u32 as f32,
                // z: 1 + the least mip when there is a scene copy (0: none).
                if self.scene_color.is_some() {
                    1.0 + self.scene_color_min_lod
                } else {
                    0.0
                },
                self.scene_color.map_or(0.0, |(_, mip)| mip as f32),
            ),
            Vec4::new(
                self.specular_aa.0,
                self.specular_aa.1,
                self.peel as f32,
                reverse,
            ),
        ];
        if data == self.lighting {
            return;
        }
        let ubo = self.lighting_ubo;
        let [env, params, output, shading] = [data[0], data[1], data[2], data[3]];
        ctx.ubo_set_vec4(ubo, "u_environment", env.x, env.y, env.z, env.w);
        ctx.ubo_set_vec4(ubo, "u_lighting", params.x, params.y, params.z, params.w);
        ctx.ubo_set_vec4(ubo, "u_output", output.x, output.y, output.z, output.w);
        ctx.ubo_set_vec4(ubo, "u_shading", shading.x, shading.y, shading.z, shading.w);
        self.lighting = data;
    }

    // Camera matrices into the frame block; the view and view-projection for
    // sorting and culling.
    fn write_frame(
        &mut self,
        world: &World,
        ctx: &Context,
        camera: Entity,
    ) -> Option<(Mat4, Mat4)> {
        let (Some(cam), Some(cam_world)) = (
            world.get::<Camera>(camera),
            world.get::<WorldMatrix>(camera),
        ) else {
            console::error_once(
                "[rswebgl-scene] the camera entity needs a Camera and a WorldMatrix \
                 (a Transform, after TransformSystem::update); nothing drawn",
            );
            return None;
        };
        let view = cam_world.0.inverse();
        let proj = cam.projection_for(ctx.reverse_z());
        let view_proj = proj * view;
        self.last_view_proj = view_proj;
        // TAA's subpixel jitter: clip xy moved by jitter·w (any projection).
        let jitter = Mat4::from_cols(
            Vec4::X,
            Vec4::Y,
            Vec4::Z,
            Vec4::new(self.jitter.x, self.jitter.y, 0.0, 1.0),
        );
        let drawn_proj = jitter * proj;
        (self.frame_view, self.frame_proj) = (view, proj);
        let eye = cam_world.0.w_axis;
        ctx.ubo_set_mat(self.frame_ubo, "u_view", &view.to_cols_array());
        ctx.ubo_set_mat(self.frame_ubo, "u_proj", &drawn_proj.to_cols_array());
        ctx.ubo_set_mat(
            self.frame_ubo,
            "u_view_proj",
            &(drawn_proj * view).to_cols_array(),
        );
        ctx.ubo_set_vec4(self.frame_ubo, "u_camera_pos", eye.x, eye.y, eye.z, 1.0);
        Some((view, view_proj))
    }

    // `e`'s object group, created on first use and rewritten when its world
    // matrix changed (at `tick`) since it was last written.
    pub(crate) fn ensure_object_group(
        &mut self,
        ctx: &Context,
        e: Entity,
        m: &Mat4,
        tick: Tick,
    ) -> Option<ParamGroupId> {
        match self.objects.get(&e).copied() {
            Some(o) => {
                if o.written != tick {
                    self.write_object(ctx, o.group, m);
                    self.objects.insert(e, Object { written: tick, ..o });
                }
                Some(o.group)
            }
            None => match ctx.create_param_group(self.object_layout) {
                Ok(group) => {
                    self.write_object(ctx, group, m);
                    // For picking: which entity a pixel is.
                    ctx.param_set_uints(group, self.f_object_id, &[e.to_raw()]);
                    self.objects.insert(
                        e,
                        Object {
                            group,
                            written: tick,
                        },
                    );
                    Some(group)
                }
                Err(err) => {
                    console::error_once(&format!(
                        "[rswebgl-scene] can't create an object group: {err}"
                    ));
                    None
                }
            },
        }
    }

    fn write_object(&mut self, ctx: &Context, group: ParamGroupId, m: &Mat4) {
        let upper = Mat3::from_mat4(*m);
        // A zero scale has no inverse; fall back rather than upload NaNs.
        let normal = if upper.determinant() != 0.0
            && upper
                .inverse()
                .to_cols_array()
                .iter()
                .all(|x| x.is_finite())
        {
            upper.inverse().transpose()
        } else {
            upper
        };
        ctx.param_set_floats(group, self.f_model, &m.to_cols_array());
        ctx.param_set_floats(group, self.f_normal, &normal.to_cols_array());
        self.stats.objects_written += 1;
    }

    /// The nearest visible mesh under `ndc` (−1…1, +Y up) as `camera` sees
    /// it on a `width`×`height` target, found on the GPU (see `gpu_pick`):
    /// the future resolves a frame or so later. Uses the world matrices as
    /// of the last `TransformSystem::update`. An error if the camera isn't
    /// one or the picking objects can't be made.
    pub fn pick(
        &mut self,
        world: &World,
        ctx: &Context,
        camera: Entity,
        ndc: Vec2,
        width: u32,
        height: u32,
    ) -> Result<impl Future<Output = Result<Option<Hit>, Error>> + use<>, Error> {
        let (Some(cam), Some(cam_world)) = (
            world.get::<Camera>(camera),
            world.get::<WorldMatrix>(camera),
        ) else {
            return Err("pick: the camera entity needs a Camera and a WorldMatrix".into());
        };
        let reverse_z = ctx.reverse_z();
        let view_proj = cam.projection_for(reverse_z) * cam_world.0.inverse();
        let pick_view_proj =
            GpuPicker::narrowing(ndc, width.max(1) as f32, height.max(1) as f32) * view_proj;
        let frustum = Frustum::from_view_proj_depth(&pick_view_proj, reverse_z);
        let eye = cam_world.0.w_axis.truncate();

        let mut meshes = Vec::new();
        if let (Some(storage), Some(worlds)) = (
            world.storage::<MeshRenderer>(),
            world.storage::<WorldMatrix>(),
        ) {
            let bounds = world.storage::<Bounds>();
            let any_hidden = world
                .storage::<Visible>()
                .is_some_and(|v| v.data().iter().any(|v| !v.0));
            for (e, mesh) in storage.iter() {
                if mesh.vao.is_null() || (any_hidden && hidden(world, e)) {
                    continue;
                }
                let Some(row) = worlds.row(e) else {
                    continue;
                };
                let m = worlds.data()[row as usize].0;
                if let Some(b) = bounds.and_then(|b| b.get(e)) {
                    let (c, h) = b.transformed(&m);
                    if !frustum.intersects(c, h) {
                        continue;
                    }
                }
                if let Some(group) = self.ensure_object_group(ctx, e, &m, worlds.changed_tick(row))
                {
                    meshes.push((*mesh, group));
                }
            }
        }
        if self.picker.is_none() {
            self.picker = Some(GpuPicker::new(ctx)?);
        }
        let picker = self.picker.as_ref().expect("just made");
        let view = PickView {
            pick_view_proj,
            view_proj,
            ndc,
            eye,
            zero_to_one: reverse_z,
        };
        Ok(picker.pick(ctx, &meshes, &view))
    }

    // Groups of entities that no longer have a MeshRenderer, seen through
    // the removal log since the last look; a sweep also runs if there are
    // more groups than meshes (a log trimmed too early).
    /// The tick through which it has looked at the removal logs:
    /// `World::trim_removed` no further than this.
    pub fn removals_checked(&self) -> Tick {
        self.removals_checked
    }

    fn release_removed(&mut self, world: &World, ctx: &Context) {
        let meshes = world.storage::<MeshRenderer>();
        let since = self.removals_checked;
        // What's logged from here on is stamped at `tick()` or later.
        self.removals_checked = world.tick().saturating_sub(1);
        let removed = meshes.is_some_and(|m| m.any_removed_since(since));
        let live = meshes.map_or(0, |m| m.len());
        if !removed && self.objects.len() <= live {
            return;
        }
        self.objects.retain(|&e, o| {
            let keep = world.has::<MeshRenderer>(e);
            if !keep {
                ctx.destroy_param_group(o.group);
            }
            keep
        });
    }
}

// Whether `e` or an ancestor has `Visible(false)`. Bounded against a
// hand-made `Parent` cycle.
pub(crate) fn hidden(world: &World, mut e: Entity) -> bool {
    for _ in 0..=world.len() {
        if world.get::<Visible>(e).is_some_and(|v| !v.0) {
            return true;
        }
        match world.get::<Parent>(e) {
            Some(p) => e = p.0,
            None => return false,
        }
    }
    false
}
