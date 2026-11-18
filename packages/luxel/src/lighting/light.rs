use glam::{Mat4, Vec3, Vec4};

use super::views::caster_bounds;
use crate::render::hidden;
use crate::transform::WorldMatrix;

/// The kinds of punctual light (KHR_lights_punctual). Cone angles are in
/// radians from the light's axis, `0 <= inner < outer <= π/2`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LightKind {
    Directional,
    Point,
    Spot { inner_cone: f32, outer_cone: f32 },
}

/// A punctual light. It shines down its entity's local −Z (directional,
/// spot) from the entity's world position (point, spot).
///
/// Units as in glTF: `intensity` is lux (lm/m²) for a directional light and
/// candela (lm/sr) otherwise. `range` cuts a point or spot light off smoothly
/// at that distance; 0 means no cutoff (inverse-square only).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Light {
    pub kind: LightKind,
    pub color: Vec3,
    pub intensity: f32,
    pub range: f32,
}

impl Light {
    pub fn directional(color: Vec3, intensity: f32) -> Self {
        Self {
            kind: LightKind::Directional,
            color,
            intensity,
            range: 0.0,
        }
    }

    pub fn point(color: Vec3, intensity: f32, range: f32) -> Self {
        Self {
            kind: LightKind::Point,
            color,
            intensity,
            range,
        }
    }

    pub fn spot(color: Vec3, intensity: f32, range: f32, inner_cone: f32, outer_cone: f32) -> Self {
        Self {
            kind: LightKind::Spot {
                inner_cone,
                outer_cone,
            },
            color,
            intensity,
            range,
        }
    }

    /// The light as the shader's `Light` struct (4 vec4s): position,
    /// direction, color + intensity, and (range, spot scale, spot offset,
    /// type).
    pub(crate) fn packed(&self, world: &Mat4) -> [Vec4; 4] {
        let position = world.w_axis;
        let direction = (-world.z_axis.truncate()).normalize_or(Vec3::NEG_Z);
        let (scale, offset, ty) = match self.kind {
            LightKind::Directional => (0.0, 0.0, 0.0),
            LightKind::Point => (0.0, 0.0, 1.0),
            LightKind::Spot {
                inner_cone,
                outer_cone,
            } => {
                // KHR_lights_punctual's smooth cone falloff.
                let (ci, co) = (inner_cone.cos(), outer_cone.cos());
                let scale = 1.0 / (ci - co).max(0.001);
                (scale, -co * scale, 2.0)
            }
        };
        [
            position,
            direction.extend(0.0),
            self.color.extend(self.intensity),
            Vec4::new(self.range.max(0.0), scale, offset, ty),
        ]
    }
}

/// The key light: the visible light giving the most light at the middle of
/// the shadow casters (the scene's centre) — a directional light its
/// intensity (lux), a point or spot light its intensity over the squared
/// distance (candela → lux), nothing beyond its range or outside its cone.
/// Contact shadows are of it, and the shaders report its share of each
/// pixel. Its entity, and its world position (w 1) or the direction toward
/// it (w 0).
pub(crate) fn key_light(world: &luxel_ecs::World) -> Option<(luxel_ecs::Entity, Vec4)> {
    let lights = world.storage::<Light>()?;
    let worlds = world.storage::<WorldMatrix>()?;
    let any_hidden = crate::render::any_hidden(world);
    let centre = caster_bounds(world).map_or(Vec3::ZERO, |(lo, hi)| (lo + hi) * 0.5);
    let mut best: Option<(f32, luxel_ecs::Entity, Vec4)> = None;
    for (e, light) in lights.iter() {
        if any_hidden && hidden(world, e) {
            continue;
        }
        let Some(m) = worlds.get(e) else {
            continue;
        };
        let power = light.intensity * light.color.max_element();
        // It shines down its −Z.
        let forward = (-m.0.z_axis.truncate()).normalize_or(Vec3::NEG_Z);
        let (lux, at) = match light.kind {
            LightKind::Directional => (power, (-forward).extend(0.0)),
            LightKind::Point | LightKind::Spot { .. } => {
                let position = m.0.w_axis.truncate();
                let to = centre - position;
                let d2 = to.length_squared().max(1e-6);
                let mut lux = power / d2;
                if light.range > 0.0 && d2 > light.range * light.range {
                    lux = 0.0;
                }
                if let LightKind::Spot { outer_cone, .. } = light.kind
                    && forward.dot(to / d2.sqrt()) < outer_cone.cos()
                {
                    lux = 0.0;
                }
                (lux, position.extend(1.0))
            }
        };
        if lux > 0.0 && best.is_none_or(|(b, _, _)| lux > b) {
            best = Some((lux, e, at));
        }
    }
    best.map(|(_, e, at)| (e, at))
}

#[cfg(test)]
mod key_light_tests {
    use super::*;
    use crate::culling::Bounds;
    use crate::lighting::settings::ShadowCaster;
    use crate::transform::WorldMatrix;
    use luxel_ecs::World;

    #[test]
    fn the_key_light_gives_the_most_light_at_the_middle() {
        let mut world = World::new();
        let caster = world.spawn();
        world.insert(
            caster,
            ShadowCaster {
                program: luxel_gl::ids::ProgramId::NULL,
            },
        );
        world.insert(caster, WorldMatrix(Mat4::IDENTITY));
        world.insert(caster, Bounds::new(Vec3::splat(-0.1), Vec3::splat(0.1)));
        let sun = world.spawn();
        world.insert(sun, Light::directional(Vec3::ONE, 512.0));
        world.insert(sun, WorldMatrix(Mat4::IDENTITY));
        let bulb = world.spawn();
        world.insert(bulb, Light::point(Vec3::ONE, 1500.0, 0.0));
        let near = Mat4::from_translation(Vec3::new(0.12, 0.12, 0.0));
        world.insert(bulb, WorldMatrix(near));
        let (key, at) = key_light(&world).unwrap();
        assert_eq!(key, bulb);
        assert_eq!(at, Vec4::new(0.12, 0.12, 0.0, 1.0));
        // 10 m away it gives 15 lux: the sun's (toward it: +Z).
        world.insert(
            bulb,
            WorldMatrix(Mat4::from_translation(Vec3::new(10.0, 0.0, 0.0))),
        );
        assert_eq!(
            key_light(&world),
            Some((sun, Vec4::new(0.0, 0.0, 1.0, 0.0)))
        );
        // Out of its range: nothing from it.
        world.insert(bulb, Light::point(Vec3::ONE, 1e9, 1.0));
        assert_eq!(key_light(&world).map(|(e, _)| e), Some(sun));
    }
}
