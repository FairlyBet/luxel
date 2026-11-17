//! The `Lights` block: the punctual lights a frame shades with and their
//! shadows, as lights.glsl declares it.

use std::collections::HashMap;

use glam::{Mat4, Vec4};
use rswebgl::buffer::BufferUsage;
use rswebgl::console;
use rswebgl::context::Context;
use rswebgl::error::Error;
use rswebgl::ids::UniformBufferId;
use rswebgl::uniform_buffer::{UboLayout, UniformType};
use rswebgl_ecs::{Entity, World};

use super::ShadowSlot;
use super::light::Light;
use super::maps::MAX_SHADOW_VIEWS;
use super::views::ShadowView;
use crate::render::{Visible, hidden};
use crate::transform::WorldMatrix;

/// Lights the block holds; more are left out (reported once).
pub const MAX_LIGHTS: usize = 8;

// A light's shadow when it has none: no layer.
const NO_SHADOW: Vec4 = Vec4::new(-1.0, 0.0, 0.0, 0.0);

/// The block's buffer, and what it holds (written only when that changes).
pub(crate) struct LightsBlock {
    ubo: UniformBufferId,
    held: Vec<Vec4>,
}

impl LightsBlock {
    pub fn new(ctx: &Context) -> Result<Self, Error> {
        let light = UboLayout::new()
            .field("position", UniformType::Vec4)
            .field("direction", UniformType::Vec4)
            .field("color", UniformType::Vec4)
            .field("params", UniformType::Vec4);
        let ubo = ctx.create_uniform_buffer(
            &UboLayout::new()
                .struct_array("u_lights", &light, MAX_LIGHTS as u32)
                .field("u_light_count", UniformType::Vec4)
                .array("u_light_shadows", UniformType::Vec4, MAX_LIGHTS as u32)
                .array("u_shadow_splits", UniformType::Vec4, MAX_LIGHTS as u32)
                .array("u_shadow_views", UniformType::Vec4, MAX_SHADOW_VIEWS as u32)
                .array("u_shadow_tiles", UniformType::Vec4, MAX_SHADOW_VIEWS as u32)
                .array(
                    "u_shadow_matrices",
                    UniformType::Mat4,
                    MAX_SHADOW_VIEWS as u32,
                ),
            BufferUsage::DynamicDraw,
        )?;
        Ok(Self {
            ubo,
            held: Vec::new(),
        })
    }

    pub fn ubo(&self) -> UniformBufferId {
        self.ubo
    }

    /// The visible lights (the first `MAX_LIGHTS`), each with its shadow
    /// maps' layers from `slots` (into `views`), the cascades' far bounds,
    /// each layer's normal bias and world → map matrix. `debug` tints the
    /// cascades; `overlap` is how far they reach into each other.
    pub fn write(
        &mut self,
        world: &World,
        ctx: &Context,
        slots: &HashMap<Entity, ShadowSlot>,
        views: &[ShadowView],
        debug: bool,
        overlap: f32,
    ) {
        let mut lights: Vec<Vec4> = Vec::with_capacity(MAX_LIGHTS * 4);
        let mut shadows: Vec<Vec4> = Vec::with_capacity(MAX_LIGHTS);
        let mut splits: Vec<Vec4> = Vec::with_capacity(MAX_LIGHTS);
        // The key light: the brightest directional one (as the scene picks
        // it for contact shadows), by its index here; −1 without one.
        let mut key: Option<(f32, usize)> = None;
        if let (Some(storage), Some(worlds)) =
            (world.storage::<Light>(), world.storage::<WorldMatrix>())
        {
            let any_hidden = world
                .storage::<Visible>()
                .is_some_and(|v| v.data().iter().any(|v| !v.0));
            for (e, light) in storage.iter() {
                let Some(m) = worlds.get(e) else {
                    continue;
                };
                if any_hidden && hidden(world, e) {
                    continue;
                }
                if shadows.len() == MAX_LIGHTS {
                    console::warn_once(&format!(
                        "[rswebgl-scene] more than {MAX_LIGHTS} lights; the rest are ignored"
                    ));
                    break;
                }
                if matches!(light.kind, crate::lighting::LightKind::Directional) {
                    let power = light.intensity * light.color.max_element();
                    if key.is_none_or(|(p, _)| power > p) {
                        key = Some((power, shadows.len()));
                    }
                }
                lights.extend(light.packed(&m.0));
                // x its first view (−1: none), y how many, z its kind (0
                // orthographic, 1 spot, 2 point).
                match slots.get(&e) {
                    Some(slot) => {
                        let (first, count) = (slot.first as f32, slot.count as f32);
                        shadows.push(Vec4::new(first, count, slot.kind, 0.0));
                        splits.push(Vec4::from_array(slot.splits));
                    }
                    None => {
                        shadows.push(NO_SHADOW);
                        splits.push(Vec4::splat(f32::MAX));
                    }
                }
            }
        }
        let count = shadows.len();
        shadows.resize(MAX_LIGHTS, NO_SHADOW);
        splits.resize(MAX_LIGHTS, Vec4::splat(f32::MAX));
        let mut data = lights;
        let debug = if debug { 1.0 } else { 0.0 };
        let key = key.map_or(-1.0, |(_, i)| i as f32);
        data.push(Vec4::new(count as f32, debug, overlap, key));
        data.extend(shadows);
        data.extend(splits);
        for i in 0..MAX_SHADOW_VIEWS {
            let (bias, layer) = views
                .get(i)
                .map_or((0.0, 0.0), |v| (v.normal_bias, v.layer as f32));
            data.push(Vec4::new(bias, layer, 0.0, 0.0));
        }
        for i in 0..MAX_SHADOW_VIEWS {
            let tile = views.get(i).map_or([0.0, 0.0, 1.0, 1.0], |v| v.tile);
            data.push(Vec4::from_array(tile));
        }
        for i in 0..MAX_SHADOW_VIEWS {
            let m = views.get(i).map_or(Mat4::IDENTITY, |v| v.to_map);
            data.extend([m.x_axis, m.y_axis, m.z_axis, m.w_axis]);
        }
        if data == self.held {
            return;
        }
        let ubo = self.ubo;
        const MEMBERS: [&str; 4] = ["position", "direction", "color", "params"];
        for (i, light) in data[..count * 4].chunks(4).enumerate() {
            for (member, v) in MEMBERS.iter().zip(light) {
                let path = format!("u_lights[{i}].{member}");
                ctx.ubo_set_vec4(ubo, &path, v.x, v.y, v.z, v.w);
            }
        }
        let mut at = count * 4;
        let c = data[at];
        ctx.ubo_set_vec4(ubo, "u_light_count", c.x, c.y, c.z, c.w);
        at += 1;
        for (name, n) in [
            ("u_light_shadows", MAX_LIGHTS),
            ("u_shadow_splits", MAX_LIGHTS),
            ("u_shadow_views", MAX_SHADOW_VIEWS),
            ("u_shadow_tiles", MAX_SHADOW_VIEWS),
        ] {
            for i in 0..n {
                let v = data[at + i];
                ctx.ubo_set_vec4(ubo, &format!("{name}[{i}]"), v.x, v.y, v.z, v.w);
            }
            at += n;
        }
        for i in 0..MAX_SHADOW_VIEWS {
            let m = at + i * 4;
            let cols: Vec<f32> = data[m..m + 4].iter().flat_map(|c| c.to_array()).collect();
            ctx.ubo_set_mat(ubo, &format!("u_shadow_matrices[{i}]"), &cols);
        }
        self.held = data;
    }

    /// Frees the buffer (nothing to free after a context loss).
    pub fn destroy(self, ctx: &Context, lost: bool) {
        if !lost {
            ctx.destroy_uniform_buffer(self.ubo);
        }
    }
}
