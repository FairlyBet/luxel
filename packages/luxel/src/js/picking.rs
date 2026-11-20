//! Asking the scene about the screen: what is under a point (on the GPU or
//! by bounds), where a world point shows, the ray through a screen point,
//! and framing a model in a camera.

use glam::Vec2;
use luxel_ecs::Entity;
use luxel_gl::error::Error;
use wasm_bindgen::prelude::*;

use super::{Scene, hit_object, vec3};

#[wasm_bindgen]
impl Scene {
    /// What's under a screen point, as `camera` sees it: a promise of
    /// `{ entity, point, distance }` (`PickHit`), or null when nothing is.
    /// Found on the GPU (exact for any geometry; the answer comes a frame or
    /// so later), with the world as of now. `x`, `y` are normalized device
    /// coordinates, −1…1 with +Y up — from a pointer event:
    /// `x = e.offsetX / canvas.clientWidth * 2 - 1`,
    /// `y = 1 - e.offsetY / canvas.clientHeight * 2`. The entity is the mesh
    /// entity (`parent` leads to its glTF node), `point` the world position
    /// hit, `distance` its distance from the camera. Meshes are drawn with
    /// their location-0 attribute as the position.
    #[wasm_bindgen(unchecked_return_type = "Promise<PickHit | null>")]
    pub fn pick(&mut self, camera: Entity, x: f32, y: f32) -> js_sys::Promise {
        let reject = |e: Error| js_sys::Promise::reject(&e.into());
        if !self.sync_context() {
            return reject(Error::context_lost("pick: the WebGL context is lost"));
        }
        // The aspect the next frame draws at (as `frameBounds`).
        self.sync_camera_aspect(camera);
        self.transforms.update(&mut self.world);
        let fb = self.ctx.default_framebuffer();
        let found = self.renderer.pick(
            &self.world,
            &self.ctx,
            camera,
            Vec2::new(x, y),
            fb.width().max(0) as u32,
            fb.height().max(0) as u32,
        );
        let found = match found {
            Ok(f) => f,
            Err(e) => return reject(e),
        };
        wasm_bindgen_futures::future_to_promise(async move {
            let Some(hit) = found.await? else {
                return Ok(JsValue::NULL);
            };
            Ok(hit_object(&hit))
        })
    }

    /// Points `camera` at `root` and everything under it, from the side it
    /// looks from now: far enough that the sphere around their bounds fills
    /// the view (`margin`, default 1.1, leaves room around it); near and far
    /// planes with room to zoom. Uses the meshes' bounds in the world as
    /// they are now (a model's, or yours from `setBounds`); a model with
    /// none (skinned) uses its bounds as loaded. Returns `[cx, cy, cz,
    /// distance]` — the center looked at and how far the camera is (an
    /// orbit's target and radius) — or empty when there is nothing to
    /// frame or `camera` isn't a camera.
    #[wasm_bindgen(js_name = frameBounds)]
    pub fn frame_bounds(&mut self, camera: Entity, root: Entity, margin: Option<f32>) -> Vec<f32> {
        // The aspect the next frame draws at (a new camera has 1 until then).
        self.sync_camera_aspect(camera);
        self.transforms.update(&mut self.world);
        let bounds = crate::framing::world_bounds(&self.world, root).or_else(|| {
            // A model without mesh bounds: its sphere as loaded, moved with
            // the root.
            let model = self.model(root)?;
            let m = self.world.get::<crate::transform::WorldMatrix>(root)?.0;
            let c = m.transform_point3(glam::Vec3::from_slice(&model.center()));
            let scale = m
                .x_axis
                .truncate()
                .length()
                .max(m.y_axis.truncate().length())
                .max(m.z_axis.truncate().length());
            let r = glam::Vec3::splat(model.radius() * scale / 3f32.sqrt());
            Some((c - r, c + r))
        });
        let Some((min, max)) = bounds else {
            return Vec::new();
        };
        let margin = margin.filter(|m| m.is_finite() && *m > 0.0).unwrap_or(1.1);
        crate::framing::frame_box(&mut self.world, camera, min, max, margin)
            .map_or_else(Vec::new, |(c, d)| vec![c.x, c.y, c.z, d])
    }

    /// Where world point `[x, y, z]` shows as `camera` sees it:
    /// `[x, y, depth]` — x, y in normalized device coordinates (−1…1, +Y up;
    /// to CSS pixels: `(x + 1) / 2 * canvas.clientWidth`,
    /// `(1 - y) / 2 * canvas.clientHeight`), depth its distance in front of
    /// the camera along the view axis (≤ 0: behind it — hide a hotspot
    /// then). Empty if `camera` isn't a camera. With the transforms as of now.
    #[wasm_bindgen(js_name = worldToScreen)]
    pub fn world_to_screen(
        &mut self,
        camera: Entity,
        #[wasm_bindgen(unchecked_param_type = "ArrayLike<number>")] point: &[f32],
    ) -> Vec<f32> {
        let Some(p) = vec3("worldToScreen", "point", point) else {
            return Vec::new();
        };
        self.sync_camera_aspect(camera);
        self.transforms.update(&mut self.world);
        crate::pick::world_to_ndc(&self.world, camera, p)
            .map_or_else(Vec::new, |v| v.to_array().to_vec())
    }

    /// The ray from `camera` through a screen point (`x`, `y` normalized
    /// device coordinates, as `pick`): `[ox, oy, oz, dx, dy, dz]` — its origin
    /// on the near plane and unit direction, in world space. Empty if
    /// `camera` isn't a camera.
    #[wasm_bindgen(js_name = screenToRay)]
    pub fn screen_to_ray(&mut self, camera: Entity, x: f32, y: f32) -> Vec<f32> {
        self.sync_camera_aspect(camera);
        self.transforms.update(&mut self.world);
        crate::pick::camera_ray(&self.world, camera, Vec2::new(x, y)).map_or_else(Vec::new, |r| {
            [r.origin.to_array(), r.dir.to_array()].concat()
        })
    }

    /// `pick` at once, by the meshes' bounding boxes instead of their
    /// triangles: the nearest visible mesh whose box the ray enters (`point`
    /// on the box). Coarse — a box is bigger than what's in it — but
    /// synchronous: for hover effects and drag targets; `pick` for what's
    /// exactly under the pointer.
    #[wasm_bindgen(js_name = raycastBounds, unchecked_return_type = "PickHit | null")]
    pub fn raycast_bounds(&mut self, camera: Entity, x: f32, y: f32) -> JsValue {
        self.sync_camera_aspect(camera);
        self.transforms.update(&mut self.world);
        let Some(hit) = crate::pick::pick_bounds(&self.world, camera, Vec2::new(x, y)) else {
            return JsValue::NULL;
        };
        hit_object(&hit)
    }
}
