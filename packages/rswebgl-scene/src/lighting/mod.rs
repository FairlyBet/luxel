//! Punctual lights and their shadows, one system: the renderer asks it to
//! `prepare` a frame (which lights shine, which cast shadows and from
//! where, what each shadow map draws), gives the casters their objects,
//! and renders the maps before the lit passes. Nothing else knows about
//! shadows.
//!
//! - `light` — the `Light` component (KHR_lights_punctual).
//! - `settings` — what the app sets: `LightShadow` per light,
//!   `ShadowCaster` per mesh, `ShadowOptions` for the maps.
//! - `views` — each shadowing light's camera (math only).
//! - `maps` — the depth-texture array, a layer and pass per light.
//! - `casters` — which casters each map draws.
//! - `pack` — the `Lights` block the lit shaders read (lights.glsl).

mod casters;
mod light;
mod maps;
mod pack;
mod settings;
mod views;

use std::collections::HashMap;

use rswebgl::console;
use rswebgl::context::Context;
use rswebgl::error::Error;
use rswebgl::ids::{ParamGroupId, ParamLayoutId, StateId, TextureId, UniformBufferId};
use rswebgl::param::GROUP_SLOTS;
use rswebgl_ecs::{Entity, World};

pub(crate) use casters::CasterDraw;
pub use light::{Light, LightKind};
pub use maps::MAX_SHADOW_LAYERS;
pub use pack::MAX_LIGHTS;
pub use settings::{LightShadow, ShadowCaster};
pub use views::MAX_CASCADES;

use maps::ShadowMaps;
use pack::LightsBlock;
use settings::ShadowOptions;
use views::{CASCADE_OVERLAP, ShadowView, ViewCamera, cascade_views, caster_bounds, shadow_view};

use crate::camera::Camera;
use crate::render::{FRAME_SLOT, LIGHTING_BLOCK, MATERIAL_SLOT, OBJECT_SLOT, Visible, hidden};
use crate::transform::WorldMatrix;

/// Name of the lights block, bound next to the frame and lighting blocks
/// in slot 0 (lights.glsl declares it):
///
/// ```glsl
/// struct Light {
///     vec4 position;  // xyz world position
///     vec4 direction; // xyz world direction the light points
///     vec4 color;     // rgb color, w intensity
///     vec4 params;    // x range, y spot scale, z spot offset, w type (0 dir, 1 point, 2 spot)
/// };
/// layout(std140) uniform Lights {
///     Light u_lights[8];                // MAX_LIGHTS
///     vec4 u_light_count;        // x lights, y 1: cascades tinted, z cascade overlap
///     vec4 u_light_shadows[8];   // x first layer (< 0: none), y layers, z 1: perspective
///     vec4 u_shadow_splits[8];   // the cascades' far bounds (camera-space depth)
///     vec4 u_shadow_layers[8];   // MAX_SHADOW_LAYERS: x normal bias
///     mat4 u_shadow_matrices[8]; // world → map [0, 1]³
/// };
/// uniform highp sampler2DArrayShadow u_shadow_map;
/// ```
pub const LIGHTS_BLOCK: &str = "Lights";
/// The shadow maps' depth array, in the frame group.
pub const SHADOW_MAP: &str = "u_shadow_map";

/// The lights and shadows of a scene renderer.
pub(crate) struct LightingSystem {
    block: LightsBlock,
    maps: ShadowMaps,
    options: ShadowOptions,
    // Cascades tinted by index (debugging their splits).
    debug_cascades: bool,
    // This frame: each shadowing light's layers, the casters each layer
    // draws, and each layer's render state (its light's biases).
    layers: HashMap<Entity, ShadowSlot>,
    draws: Vec<CasterDraw>,
    states: Vec<StateId>,
}

impl LightingSystem {
    /// Its block and maps bound into `frame_group` (of `frame_layout`);
    /// the shadow passes' groups share `lighting_ubo` with it.
    pub fn new(
        ctx: &Context,
        frame_layout: ParamLayoutId,
        frame_group: ParamGroupId,
        lighting_ubo: UniformBufferId,
    ) -> Result<Self, Error> {
        let block = LightsBlock::new(ctx)?;
        let shared = [(LIGHTING_BLOCK, lighting_ubo), (LIGHTS_BLOCK, block.ubo())];
        let maps = match ShadowMaps::new(ctx, frame_layout, &shared) {
            Ok(maps) => maps,
            Err(e) => {
                block.destroy(ctx, false);
                return Err(e);
            }
        };
        ctx.param_set_block_by_name(frame_group, LIGHTS_BLOCK, block.ubo());
        ctx.param_set_texture_by_name(frame_group, SHADOW_MAP, maps.texture());
        Ok(Self {
            block,
            maps,
            options: ShadowOptions::default(),
            debug_cascades: false,
            layers: HashMap::new(),
            draws: Vec::new(),
            states: Vec::new(),
        })
    }

    /// The side of each shadow map, in texels (default 2048; rounded to a
    /// power of two in 256..=8192, within the device's texture size).
    pub fn set_shadow_map_size(&mut self, size: u32) {
        let max = rswebgl::limits::max_texture_size().max(256) as u32;
        self.options.map_size = ShadowOptions::clamp_size(size, max);
    }

    pub fn shadow_map_size(&self) -> u32 {
        self.options.map_size
    }

    /// 16-bit shadow maps (half the memory) instead of 24-bit; default off.
    pub fn set_shadow_depth16(&mut self, on: bool) {
        self.options.depth16 = on;
    }

    pub fn shadow_depth16(&self) -> bool {
        self.options.depth16
    }

    /// Tints what each cascade shades (red, green, blue, yellow nearest
    /// first): to see where they split.
    pub fn set_debug_cascades(&mut self, on: bool) {
        self.debug_cascades = on;
    }

    /// Shadow maps drawn this frame (all lights' layers).
    pub fn shadow_views(&self) -> usize {
        self.maps.views.len()
    }

    /// The shadow maps' memory now, in bytes (a 1×1 placeholder while no
    /// light casts shadows).
    pub fn shadow_bytes(&self) -> usize {
        let side = self.maps.size() as usize;
        let texel = if self.maps.depth16() { 2 } else { 4 };
        side * side * self.maps.layers() * texel
    }

    /// A new frame seen by `camera` (its projection and world matrix; the
    /// cascades follow it): which lights cast shadows and from where (the
    /// directional / spot lights with a `LightShadow`, while
    /// `MAX_SHADOW_LAYERS` last), the lights block, and the casters each
    /// map draws (`push_casters`).
    pub fn prepare(
        &mut self,
        world: &World,
        ctx: &Context,
        frame_group: ParamGroupId,
        camera: Option<(Camera, glam::Mat4)>,
    ) {
        let camera = camera.map(|(camera, world)| ViewCamera { world, camera });
        self.plan(world, ctx, frame_group, camera.as_ref());
        let debug = self.debug_cascades;
        self.block.write(
            world,
            ctx,
            &self.layers,
            &self.maps.views,
            debug,
            CASCADE_OVERLAP,
        );
        casters::cull(world, ctx, &self.maps.views, &mut self.draws);
        self.states.clear();
        for view in &self.maps.views {
            self.states.push(ShadowMaps::state(ctx, &view.state));
        }
    }

    /// This frame's caster draws, for the renderer to give each its object
    /// group; hand them back with `push_casters`.
    pub fn take_caster_draws(&mut self) -> Vec<CasterDraw> {
        std::mem::take(&mut self.draws)
    }

    /// Pushes `draws` into their maps' passes, each with the object group
    /// at its index in `objects` (NULL: skipped).
    pub fn push_casters(&mut self, draws: Vec<CasterDraw>, objects: &[ParamGroupId]) {
        for (d, &object) in draws.iter().zip(objects) {
            if object.is_null() {
                continue;
            }
            let mut groups = [ParamGroupId::NULL; GROUP_SLOTS];
            groups[FRAME_SLOT] = self.maps.group(d.view);
            groups[MATERIAL_SLOT] = d.mesh.material;
            groups[OBJECT_SLOT] = object;
            self.maps.passes[d.view].push_groups(
                d.program,
                self.states[d.view],
                d.mesh.vao,
                groups,
                &d.mesh.command,
            );
        }
        self.draws = draws;
    }

    /// The frame's instance texture, for the casters' passes.
    pub fn bind_instances(&self, ctx: &Context, name: &str, texture: TextureId) {
        for i in 0..self.maps.views.len() {
            ctx.param_set_texture_by_name(self.maps.group(i), name, texture);
        }
    }

    /// Renders this frame's shadow maps (before the passes that shade with
    /// them).
    pub fn render_shadows(&self, ctx: &Context) {
        self.maps.render(ctx);
    }

    /// Frees it all; after a context loss (`lost`) only the CPU side.
    pub fn destroy(self, ctx: &Context, lost: bool) {
        self.maps.destroy(ctx, lost);
        self.block.destroy(ctx, lost);
    }

    // The shadowing lights' cameras: a spot's fitted to the casters in
    // front of it, a directional's cascades; the maps grown to hold them
    // (or shrunk to what the lights could ask for).
    fn plan(
        &mut self,
        world: &World,
        ctx: &Context,
        frame_group: ParamGroupId,
        camera: Option<&ViewCamera>,
    ) {
        self.maps.begin(ctx);
        self.layers.clear();
        let (Some(settings), Some(lights), Some(worlds)) = (
            world.storage::<LightShadow>(),
            world.storage::<Light>(),
            world.storage::<WorldMatrix>(),
        ) else {
            return;
        };
        if settings.is_empty() {
            return;
        }
        let Some(bounds) = caster_bounds(world) else {
            return;
        };
        let ShadowOptions { map_size, depth16 } = self.options;
        let reverse = ctx.reverse_z();
        let any_hidden = world
            .storage::<Visible>()
            .is_some_and(|v| v.data().iter().any(|v| !v.0));
        // Each shadowing light's views, and the most layers they could ask
        // for (a directional light's every cascade).
        let mut planned: Vec<(Entity, ShadowSlot, Vec<ShadowView>)> = Vec::new();
        let (mut used, mut could) = (0, 0);
        for (e, light) in lights.iter() {
            let Some(&s) = settings.get(e) else {
                continue;
            };
            let Some(m) = worlds.get(e) else {
                continue;
            };
            if any_hidden && hidden(world, e) {
                continue;
            }
            let (views, splits) = match (light.kind, camera) {
                (LightKind::Point, _) => continue,
                (LightKind::Directional, Some(camera)) => {
                    let c = cascade_views(&m.0, camera, bounds, map_size, reverse, s);
                    could += (s.cascades as usize).clamp(1, MAX_CASCADES);
                    (c.views, c.far_bounds)
                }
                _ => {
                    could += 1;
                    let view = shadow_view(light, &m.0, bounds, map_size, reverse, s);
                    (view.into_iter().collect(), vec![f32::MAX])
                }
            };
            if views.is_empty() {
                continue;
            }
            if used + views.len() > MAX_SHADOW_LAYERS {
                console::warn_once(&format!(
                    "[rswebgl-scene] shadows need more than {MAX_SHADOW_LAYERS} map layers; \
                     the rest of the lights shine unshadowed"
                ));
                break;
            }
            let mut far = [f32::MAX; 4];
            for (f, s) in far.iter_mut().zip(&splits) {
                *f = *s;
            }
            let slot = ShadowSlot {
                first: used,
                count: views.len(),
                splits: far,
                perspective: views[0].perspective,
            };
            used += views.len();
            planned.push((e, slot, views));
        }
        if used == 0 {
            return;
        }
        // Grown to this frame's layers; shrunk only below what the lights
        // could ask for, so a cascade coming and going doesn't reallocate.
        let could = could.clamp(used, MAX_SHADOW_LAYERS);
        let layers = self.maps.layers().clamp(used, could);
        let stale = self.maps.size() != map_size
            || self.maps.layers() != layers
            || self.maps.depth16() != depth16;
        if stale {
            if let Err(e) = self.maps.resize(ctx, map_size, layers, depth16) {
                console::error_once(&format!("[rswebgl-scene] shadow maps: {e}"));
                return;
            }
            ctx.param_set_texture_by_name(frame_group, SHADOW_MAP, self.maps.texture());
        }
        for (e, slot, views) in planned {
            for view in views {
                let i = self.maps.views.len();
                self.maps.prepare(ctx, i, &view);
                self.maps.views.push(view);
            }
            self.layers.insert(e, slot);
        }
    }
}

/// Where a shadowing light's maps are this frame: `count` layers from
/// `first` (a directional light's cascades, nearest first, each reaching
/// to its far bound in `splits`).
#[derive(Debug, Clone, Copy)]
pub(crate) struct ShadowSlot {
    pub first: usize,
    pub count: usize,
    pub splits: [f32; 4],
    pub perspective: bool,
}
