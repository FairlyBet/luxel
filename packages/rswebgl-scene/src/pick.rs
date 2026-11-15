//! Rays and picking by bounds, on the CPU. The scene picks on the GPU
//! (`SceneRenderer::pick`: exact, asynchronous); this is the cheap,
//! synchronous approximation — the meshes' boxes — and the ray maths.

use glam::{Mat4, Vec2, Vec3};
use rswebgl_ecs::{Entity, World};

use crate::camera::Camera;
use crate::culling::Bounds;
use crate::render::{MeshRenderer, Visible, hidden};
use crate::transform::WorldMatrix;

/// A ray: `origin + t·dir`, `t ≥ 0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ray {
    pub origin: Vec3,
    pub dir: Vec3,
}

impl Ray {
    /// The ray through `ndc` (−1…1, +Y up) from the camera whose
    /// view-projection is `view_proj`: from the near plane toward the far.
    pub fn through(view_proj: &Mat4, ndc: Vec2) -> Self {
        let inv = view_proj.inverse();
        let near = inv.project_point3(ndc.extend(-1.0));
        let far = inv.project_point3(ndc.extend(1.0));
        Self {
            origin: near,
            dir: (far - near).normalize_or_zero(),
        }
    }

    pub fn at(&self, t: f32) -> Vec3 {
        self.origin + self.dir * t
    }

    /// The same ray in the space `m` maps from (e.g. world → a mesh's local
    /// space with `m` = the inverse world matrix). `dir` isn't renormalized,
    /// so `t` keeps meaning the same point in both spaces.
    pub fn transformed(&self, m: &Mat4) -> Self {
        Self {
            origin: m.transform_point3(self.origin),
            dir: m.transform_vector3(self.dir),
        }
    }
}

/// `camera`'s view and projection (depth −1…1), from its `Camera` and the
/// last `TransformSystem::update`'s world matrix.
pub fn view_and_projection(world: &World, camera: Entity) -> Option<(Mat4, Mat4)> {
    let cam = world.get::<Camera>(camera)?;
    let view = world.get::<WorldMatrix>(camera)?.0.inverse();
    Some((view, cam.projection_matrix()))
}

/// The ray from `camera` through `ndc` (−1…1, +Y up), from its near plane.
pub fn camera_ray(world: &World, camera: Entity, ndc: Vec2) -> Option<Ray> {
    let (view, proj) = view_and_projection(world, camera)?;
    Some(Ray::through(&(proj * view), ndc))
}

/// Where world point `p` lands as `camera` sees it: x, y in normalized
/// device coordinates (−1…1, +Y up) and z its depth in front of the camera
/// (along the view axis; ≤ 0 behind it, where x and y mean nothing).
pub fn world_to_ndc(world: &World, camera: Entity, p: Vec3) -> Option<Vec3> {
    let (view, proj) = view_and_projection(world, camera)?;
    let v = view.transform_point3(p);
    let clip = proj * v.extend(1.0);
    let w = if clip.w.abs() > f32::EPSILON {
        clip.w
    } else {
        f32::EPSILON
    };
    Some(Vec3::new(clip.x / w, clip.y / w, -v.z))
}

/// Entry distance of `ray` into the box (0 if it starts inside), or None.
pub fn ray_aabb(ray: &Ray, min: Vec3, max: Vec3) -> Option<f32> {
    let inv = ray.dir.recip();
    let t1 = (min - ray.origin) * inv;
    let t2 = (max - ray.origin) * inv;
    let near = t1.min(t2).max_element();
    let far = t1.max(t2).min_element();
    (far >= near.max(0.0)).then_some(near.max(0.0))
}

/// What a pick hit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    pub entity: Entity,
    pub distance: f32,
    pub point: Vec3,
}

/// The visible mesh whose `Bounds` a ray from `camera` through `ndc` enters
/// first, or None; `distance` from the camera (as the GPU pick's). Meshes
/// without bounds can't be hit. World matrices are the last
/// `TransformSystem::update`'s.
pub fn pick_bounds(world: &World, camera: Entity, ndc: Vec2) -> Option<Hit> {
    let ray = camera_ray(world, camera, ndc)?;
    let eye = world.get::<WorldMatrix>(camera)?.0.w_axis.truncate();
    let meshes = world.storage::<MeshRenderer>()?;
    let worlds = world.storage::<WorldMatrix>()?;
    let bounds = world.storage::<Bounds>()?;
    let any_hidden = world
        .storage::<Visible>()
        .is_some_and(|v| v.data().iter().any(|v| !v.0));
    let mut best: Option<(f32, Entity)> = None;
    for (e, _) in meshes.iter() {
        let (Some(m), Some(b)) = (worlds.get(e), bounds.get(e)) else {
            continue;
        };
        if any_hidden && hidden(world, e) {
            continue;
        }
        let local = ray.transformed(&m.0.inverse());
        if let Some(t) = ray_aabb(&local, b.min, b.max)
            && best.is_none_or(|(bt, _)| t < bt)
        {
            best = Some((t, e));
        }
    }
    best.map(|(t, entity)| {
        let point = ray.at(t);
        Hit {
            entity,
            distance: point.distance(eye),
            point,
        }
    })
}
