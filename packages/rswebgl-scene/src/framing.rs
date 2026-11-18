//! Framing: a subtree's box in the world, and a camera placed to show it.

use glam::{Mat4, Vec3};
use rswebgl_ecs::{Entity, World};

use crate::camera::{Camera, Projection};
use crate::culling::Bounds;
use crate::hierarchy::descendants;
use crate::transform::{Transform, WorldMatrix};

/// The world-space box around the `Bounds` of `root` and everything under it
/// that is shown (`Visible`, KHR_node_visibility) — as of the last
/// `TransformSystem::update` — or None when none has bounds.
/// Rotated boxes count whole (the box around each, not its exact shape).
pub fn world_bounds(world: &World, root: Entity) -> Option<(Vec3, Vec3)> {
    let bounds = world.storage::<Bounds>()?;
    let worlds = world.storage::<WorldMatrix>()?;
    let any_hidden = crate::render::any_hidden(world);
    let mut out: Option<(Vec3, Vec3)> = None;
    for e in descendants(world, root) {
        let (Some(b), Some(m)) = (bounds.get(e), worlds.get(e)) else {
            continue;
        };
        if any_hidden && crate::render::hidden(world, e) {
            continue;
        }
        let (c, h) = b.transformed(&m.0);
        let (lo, hi) = (c - h, c + h);
        out = Some(out.map_or((lo, hi), |(a, b)| (a.min(lo), b.max(hi))));
    }
    out
}

/// Places `camera` so the sphere around the box `min`…`max` fills its view
/// (`margin` > 1 leaves room around it), looking the way it looks now from
/// the box's center. Its near and far planes are set with room to zoom:
/// near at 1% of the radius, far 20 times the distance to the sphere's back
/// (an infinite far plane stays).
/// Returns the center and the camera's distance from it, or None if
/// `camera` isn't a camera.
pub fn frame_box(
    world: &mut World,
    camera: Entity,
    min: Vec3,
    max: Vec3,
    margin: f32,
) -> Option<(Vec3, f32)> {
    let cam = *world.get::<Camera>(camera)?;
    let center = (min + max) * 0.5;
    let radius = ((max - min).length() * 0.5).max(1e-4) * margin.max(1e-3);
    // Where it looks now (its local −Z in the world).
    let look = world
        .get::<WorldMatrix>(camera)
        .map(|m| m.0)
        .or_else(|| world.get::<Transform>(camera).map(|t| t.matrix()))
        .unwrap_or(Mat4::IDENTITY);
    let forward = (-look.z_axis.truncate()).normalize_or(Vec3::NEG_Z);
    let aspect = if cam.aspect > 0.0 { cam.aspect } else { 1.0 };
    let (distance, projection) = match cam.projection {
        Projection::Perspective { fov_y, far, .. } => {
            // The narrower of the two fields of view.
            let fov_x = 2.0 * ((fov_y * 0.5).tan() * aspect).atan();
            let fov = fov_y.min(fov_x);
            let d = radius / (fov * 0.5).sin();
            let near = radius * 0.01;
            let far = if far.is_infinite() {
                far
            } else {
                (d + radius) * 20.0
            };
            (d, Projection::Perspective { fov_y, near, far })
        }
        Projection::Orthographic { .. } => {
            let half_height = radius / aspect.min(1.0);
            let d = radius * 2.0;
            let projection = Projection::Orthographic {
                half_height,
                near: radius * 0.01,
                far: (d + radius) * 20.0,
            };
            (d, projection)
        }
    };
    let eye = center - forward * distance;
    let up = if forward.y.abs() > 0.999 {
        Vec3::Z
    } else {
        Vec3::Y
    };
    // In the world; relative to its parent's when it has one.
    let placed = Mat4::look_at_rh(eye, center, up).inverse();
    let parent = world
        .get::<crate::transform::Parent>(camera)
        .and_then(|p| world.get::<WorldMatrix>(p.0))
        .map_or(Mat4::IDENTITY, |m| m.0);
    let t = Transform::from_matrix(parent.inverse() * placed);
    if let Some(mut c) = world.get_mut::<Camera>(camera) {
        c.set_if_neq(Camera { projection, ..cam });
    }
    match world.get_mut::<Transform>(camera) {
        Some(mut old) => old.set_if_neq(t),
        None => {
            world.insert(camera, t);
        }
    }
    Some((center, distance))
}
