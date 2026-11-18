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
//! - `atlas` — where each map goes in the array's layers.
//! - `maps` — the depth-texture array, a pass per map.
//! - `casters` — which casters each map draws.
//! - `cache` — which maps changed and are drawn again.
//! - `ground` — the soft shadow on the ground under the casters.
//! - `pack` — the `Lights` block the lit shaders read (lights.glsl).

mod atlas;
mod cache;
mod casters;
mod ground;
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
pub(crate) use ground::GroundPlane;
pub use ground::GroundShadow;
pub use light::{Light, LightKind};
pub use maps::{MAX_SHADOW_LAYERS, MAX_SHADOW_VIEWS};
pub use pack::MAX_LIGHTS;
pub use settings::{LightShadow, ShadowCaster};
pub use views::MAX_CASCADES;
pub(crate) use views::caster_bounds as scene_bounds;
pub(crate) use views::key_light;

use ground::Ground;
use maps::ShadowMaps;
use pack::LightsBlock;
use settings::ShadowOptions;
use views::{
    CASCADE_OVERLAP, ShadowView, ViewCamera, cascade_views, caster_bounds, ground_bounds, ground_view,
    point_views, shadow_view,
};

use crate::camera::Camera;
use crate::render::{FRAME_SLOT, LIGHTING_BLOCK, MATERIAL_SLOT, OBJECT_SLOT, hidden};
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
///     vec4 u_light_shadows[8];   // x first view (< 0: none), y views, z 0 ortho, 1 spot, 2 point
///     vec4 u_shadow_splits[8];   // the cascades' far bounds (camera-space depth)
///     vec4 u_shadow_views[16];   // MAX_SHADOW_VIEWS: x normal bias, y layer
///     vec4 u_shadow_tiles[16];   // the view's part of its layer (x0, y0, x1, y1)
///     mat4 u_shadow_matrices[16]; // world → map (the tile's coordinates, depth)
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
    // Bumped by `invalidate`: part of every map's signature.
    epoch: u64,
    // Frozen: the maps drawn once, fitted to the whole scene, then kept
    // (`baked` once they are).
    frozen: bool,
    baked: bool,
    // This frame: each shadowing light's layers, the casters each layer
    // draws, and each layer's render state (its light's biases).
    layers: HashMap<Entity, ShadowSlot>,
    draws: Vec<CasterDraw>,
    states: Vec<StateId>,
    // Draw calls pushed into this frame's maps.
    draw_calls: usize,
    // The ground shadow, while on.
    ground: Ground,
    ground_on: bool,
    // Casters this frame left out while their programs link.
    waiting: usize,
}

/// A caster draw's view index for the ground shadow's pass.
const GROUND_VIEW: usize = usize::MAX;

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
        let ground = match Ground::new(ctx, frame_layout, &shared) {
            Ok(g) => g,
            Err(e) => {
                maps.destroy(ctx, false);
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
            epoch: 0,
            frozen: false,
            baked: false,
            layers: HashMap::new(),
            draws: Vec::new(),
            states: Vec::new(),
            draw_calls: 0,
            ground,
            ground_on: false,
            waiting: 0,
        })
    }

    /// The side of each shadow map, in texels (default 2048; rounded to a
    /// power of two in 256..=8192, within the device's texture size).
    pub fn set_shadow_map_size(&mut self, size: u32) {
        let max = rswebgl::limits::max_texture_size().max(256) as u32;
        self.options.map_size = ShadowOptions::clamp_size(size, max);
        self.baked = false;
    }

    pub fn shadow_map_size(&self) -> u32 {
        self.options.map_size
    }

    /// 16-bit shadow maps (half the memory) instead of 24-bit; default off.
    pub fn set_shadow_depth16(&mut self, on: bool) {
        self.options.depth16 = on;
        self.baked = false;
    }

    pub fn shadow_depth16(&self) -> bool {
        self.options.depth16
    }

    /// Tints what each cascade shades (red, green, blue, yellow nearest
    /// first): to see where they split.
    pub fn debug_cascades(&self) -> bool {
        self.debug_cascades
    }

    pub fn set_debug_cascades(&mut self, on: bool) {
        self.debug_cascades = on;
    }

    /// Shadow maps in use this frame (all lights' layers).
    pub fn shadow_views(&self) -> usize {
        self.maps.views.len()
    }

    /// Draw calls into this frame's shadow maps (identical meshes drawn as
    /// one).
    pub fn draw_calls(&self) -> usize {
        self.draw_calls
    }

    /// Shadow maps drawn this frame: those whose light or casters changed.
    pub fn shadow_views_drawn(&self) -> usize {
        self.maps.redrawn()
    }

    /// Draws every shadow map again next frame: for changes the maps can't
    /// see (a material's alpha cutoff or texture edited in place). Frozen
    /// maps are baked again.
    pub fn invalidate(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.baked = false;
        self.ground.invalidate();
    }

    /// Takes `old`'s settings (map size and depth, cascade tint, frozen,
    /// the ground shadow): a rebuild after a context loss keeps them.
    pub fn keep_settings(&mut self, old: &LightingSystem) {
        self.options = old.options;
        self.debug_cascades = old.debug_cascades;
        self.frozen = old.frozen;
        self.ground.settings = old.ground.settings;
        self.ground_on = old.ground_on;
    }

    /// The ground shadow on (with these settings) or off.
    pub fn set_ground_shadow(&mut self, settings: Option<GroundShadow>) {
        self.ground_on = settings.is_some();
        if let Some(s) = settings
            && s != self.ground.settings
        {
            self.ground.settings = s;
            self.ground.invalidate();
        }
        self.baked = false;
    }

    pub fn ground_shadow(&self) -> Option<GroundShadow> {
        self.ground_on.then_some(self.ground.settings)
    }

    /// Casters this frame left out of their maps while their programs link
    /// (or their model reloads).
    pub fn waiting(&self) -> usize {
        self.waiting
    }

    /// Its own programs (the ground shadow's plane).
    pub fn programs(&self) -> Vec<rswebgl::ids::ProgramId> {
        vec![self.ground.plane_program()]
    }

    /// The ground shadow's plane to draw this frame, if any.
    pub fn ground_plane(&self, ctx: &Context) -> Option<GroundPlane> {
        if !self.ground_on {
            return None;
        }
        self.ground.plane(ctx)
    }

    /// Whether the ground shadow is drawn again this frame.
    pub fn ground_drawn(&self) -> bool {
        self.ground_on && self.ground.is_dirty()
    }

    /// Bakes frozen maps again next frame (unfrozen ones follow their
    /// lights and casters by their signatures).
    pub fn rebake(&mut self) {
        self.baked = false;
    }

    /// Frozen shadows: drawn once — directional lights fitted to every
    /// caster, not the view, so they hold wherever the camera goes — and
    /// kept until unfrozen or `invalidate`d, whatever moves. For a static
    /// scene: no shadow pass after the first frame.
    pub fn set_frozen(&mut self, on: bool) {
        if on != self.frozen {
            self.frozen = on;
            self.baked = false;
        }
    }

    pub fn frozen(&self) -> bool {
        self.frozen
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
        let debug = self.debug_cascades;
        if self.frozen && self.baked {
            // The maps, their cameras and the lights' layers as baked; the
            // lights themselves may still change color or intensity.
            self.maps.hold();
            self.ground.hold();
            self.draws.clear();
            self.waiting = 0;
            self.block.write(
                world,
                ctx,
                &self.layers,
                &self.maps.views,
                debug,
                CASCADE_OVERLAP,
            );
            return;
        }
        // Frozen: fitted to the scene, not the view.
        let camera = camera
            .filter(|_| !self.frozen)
            .map(|(camera, world)| ViewCamera { world, camera });
        self.plan(world, ctx, frame_group, camera.as_ref());
        self.block.write(
            world,
            ctx,
            &self.layers,
            &self.maps.views,
            debug,
            CASCADE_OVERLAP,
        );
        let waiting = casters::cull(world, ctx, &self.maps.views, &mut self.draws);
        let signatures = cache::signatures(world, &self.maps.views, &self.draws, self.epoch);
        self.maps.mark(&signatures);
        // Only the maps drawn again need their casters.
        let maps = &self.maps;
        self.draws.retain(|d| maps.is_dirty(d.view));
        let ground_waiting = self.prepare_ground(world, ctx);
        self.waiting = waiting + ground_waiting;
        // Baked once every caster could draw (none still linking or
        // reloading) into some map.
        if self.frozen {
            let any = !self.maps.views.is_empty() || self.ground.view.is_some();
            self.baked = waiting == 0 && ground_waiting == 0 && any;
        }
        self.states.clear();
        for view in &self.maps.views {
            self.states.push(ShadowMaps::state(ctx, &view.state));
        }
    }

    // The ground shadow's view under the casters and, if it changed, its
    // casters (added to this frame's draws). Returns the casters it waits
    // for (still linking or reloading).
    fn prepare_ground(&mut self, world: &World, ctx: &Context) -> usize {
        let view = if self.ground_on {
            let s = self.ground.settings;
            ground_bounds(world).and_then(|b| ground_view(b, s.reach, s.size, ctx.reverse_z()))
        } else {
            None
        };
        let Some((v, plane)) = view else {
            self.ground.prepare(ctx, None, 0);
            return 0;
        };
        let mut draws = Vec::new();
        let waiting = casters::cull(world, ctx, std::slice::from_ref(&v), &mut draws);
        // Its signature: the casters', and what the darkness depends on.
        let s = self.ground.settings;
        let tweak = (u64::from(s.darkness.to_bits()) << 32) | u64::from(s.blur.to_bits());
        let sig = cache::signatures(world, std::slice::from_ref(&v), &draws, self.epoch)[0] ^ tweak;
        self.ground.prepare(ctx, Some((v, plane)), sig);
        if self.ground.is_dirty() {
            self.draws.extend(draws.into_iter().map(|mut d| {
                d.view = GROUND_VIEW;
                d
            }));
        }
        waiting
    }

    /// This frame's caster draws, for the renderer to give each its object
    /// group; hand them back with `push_casters`.
    pub fn take_caster_draws(&mut self) -> Vec<CasterDraw> {
        std::mem::take(&mut self.draws)
    }

    /// Pushes `draws` into their maps' passes, each with the object group
    /// at its index in `objects` (NULL: skipped).
    pub fn push_casters(&mut self, draws: Vec<CasterDraw>, objects: &[ParamGroupId]) {
        self.draw_calls = 0;
        for (d, &object) in draws.iter().zip(objects) {
            if object.is_null() {
                // Its map would hold less than its signature says: drawn
                // again next frame (and a frozen bake isn't done).
                if d.view == GROUND_VIEW {
                    self.ground.invalidate();
                } else {
                    self.maps.forget(d.view);
                }
                self.baked = false;
                continue;
            }
            let mut groups = [ParamGroupId::NULL; GROUP_SLOTS];
            groups[MATERIAL_SLOT] = d.mesh.material;
            groups[OBJECT_SLOT] = object;
            if d.view == GROUND_VIEW {
                groups[FRAME_SLOT] = self.ground.group();
                let state = self.ground.caster_state;
                self.ground
                    .pass
                    .push_groups(d.program, state, d.mesh.vao, groups, &d.mesh.command);
                self.draw_calls += 1;
                continue;
            }
            groups[FRAME_SLOT] = self.maps.group(d.view);
            self.maps.passes[d.view].push_groups(
                d.program,
                self.states[d.view],
                d.mesh.vao,
                groups,
                &d.mesh.command,
            );
            self.draw_calls += 1;
        }
        self.draws = draws;
    }

    /// The frame's instance texture, for the casters' passes.
    pub fn bind_instances(&self, ctx: &Context, name: &str, texture: TextureId) {
        for i in 0..self.maps.views.len() {
            ctx.param_set_texture_by_name(self.maps.group(i), name, texture);
        }
        if self.ground.is_dirty() {
            ctx.param_set_texture_by_name(self.ground.group(), name, texture);
        }
    }

    /// Renders this frame's shadow maps (before the passes that shade with
    /// them).
    pub fn render_shadows(&self, ctx: &Context) {
        self.maps.render(ctx);
        if self.ground_on {
            self.ground.render(ctx);
        }
    }

    /// Frees it all; after a context loss (`lost`) only the CPU side.
    pub fn destroy(self, ctx: &Context, lost: bool) {
        self.ground.destroy(ctx, lost);
        self.maps.destroy(ctx, lost);
        self.block.destroy(ctx, lost);
    }

    // The shadowing lights' cameras — a spot's fitted to the casters in
    // front of it, a directional's cascades, a point's six faces — laid
    // out in the layers; the maps grown to hold them (or shrunk to what the
    // lights could ask for).
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
        let any_hidden = crate::render::any_hidden(world);
        // Each shadowing light's views, placed; the most layers the lights
        // could ask for (a directional light's every cascade).
        let mut planned: Vec<(Entity, ShadowSlot, Vec<ShadowView>)> = Vec::new();
        let mut views_used = 0;
        // The layers' worth of maps the lights could ask for (a directional
        // light's every cascade), in layer areas.
        let mut could = 0.0;
        let area = |side: u32| (side as f32 / map_size as f32).powi(2);
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
            let (mut views, splits, kind, side) = match (light.kind, camera) {
                (LightKind::Point, _) => {
                    let side = s.side(map_size / 2, map_size);
                    could += 6.0 * area(side);
                    let faces = point_views(light, &m.0, bounds, side, reverse, s);
                    (faces.unwrap_or_default(), vec![f32::MAX], 2.0, side)
                }
                (LightKind::Directional, Some(camera)) => {
                    let side = s.side(map_size, map_size);
                    could += (s.cascades as usize).clamp(1, MAX_CASCADES) as f32 * area(side);
                    let c = cascade_views(&m.0, camera, bounds, side, reverse, s);
                    (c.views, c.far_bounds, 0.0, side)
                }
                (kind, _) => {
                    let side = s.side(map_size, map_size);
                    could += area(side);
                    let view = shadow_view(light, &m.0, bounds, side, reverse, s);
                    let kind = if matches!(kind, LightKind::Spot { .. }) {
                        1.0
                    } else {
                        0.0
                    };
                    (view.into_iter().collect(), vec![f32::MAX], kind, side)
                }
            };
            if views.is_empty() {
                continue;
            }
            if views_used + views.len() > MAX_SHADOW_VIEWS {
                warn_too_many();
                break;
            }
            for v in &mut views {
                v.size = side;
            }
            let mut far = [f32::MAX; 4];
            for (f, s) in far.iter_mut().zip(&splits) {
                *f = *s;
            }
            let slot = ShadowSlot {
                first: views_used,
                count: views.len(),
                splits: far,
                kind,
            };
            views_used += views.len();
            planned.push((e, slot, views));
        }
        // Laid out in the atlas; lights from the last dropped while they
        // need more layers than there are.
        let used = loop {
            let sides: Vec<u32> = planned
                .iter()
                .flat_map(|(_, _, v)| v.iter().map(|v| v.size))
                .collect();
            let (places, layers) = atlas::lay_out(&sides, map_size);
            if layers <= MAX_SHADOW_LAYERS {
                let mut places = places.into_iter();
                for (_, _, views) in &mut planned {
                    for v in views {
                        if let Some((layer, tile)) = places.next() {
                            v.place(layer, tile);
                        }
                    }
                }
                break layers;
            }
            warn_too_many();
            if let Some((_, slot, _)) = planned.pop() {
                views_used -= slot.count;
            }
        };
        if views_used == 0 {
            return;
        }
        // Grown to this frame's layers; shrunk only below what the lights
        // could ask for, so a cascade coming and going doesn't reallocate.
        let could = (could.ceil() as usize).clamp(used, MAX_SHADOW_LAYERS);
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

// Lights past what the maps hold shine unshadowed: said once.
fn warn_too_many() {
    console::warn_once(&format!(
        "[rswebgl-scene] shadows need more than {MAX_SHADOW_LAYERS} map layers \
         ({MAX_SHADOW_VIEWS} maps); the rest of the lights shine unshadowed"
    ));
}

/// Where a shadowing light's maps are this frame: `count` views from
/// `first` (a directional light's cascades, nearest first, each reaching
/// to its far bound in `splits`; a point light's six faces). `kind`: 0
/// orthographic, 1 a spot's perspective, 2 a point's faces.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ShadowSlot {
    pub first: usize,
    pub count: usize,
    pub splits: [f32; 4],
    pub kind: f32,
}
