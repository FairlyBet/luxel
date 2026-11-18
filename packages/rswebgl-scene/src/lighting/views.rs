//! The shadow cameras: where each shadowing light looks from and what its
//! map covers. Math only (no GL).
//!
//! A spot light's map is fitted to the casters in front of it. A
//! directional light's is fitted to the casters' bounds (all of them, not
//! the view) — stable as the camera moves, sharp for a model — or, when the
//! view sees a large scene, split into cascades along the view: slices of
//! the view frustum, the nearest the smallest, each with its own map
//! (`cascades`). A cascade that would cover more than the casters' fit is
//! replaced by that fit, and is the last: a model in view keeps one sharp
//! map however many cascades are allowed.

use glam::{Mat4, Vec3, Vec4};
use rswebgl_ecs::World;

use super::light::{Light, LightKind};
use super::settings::{LightShadow, ShadowCaster};
use crate::camera::{Camera, Projection};
use crate::culling::Bounds;
use crate::model::Skin;
use crate::render::hidden;
use crate::transform::WorldMatrix;

/// Cascades a directional light splits into at most.
pub const MAX_CASCADES: usize = 4;
/// How far each cascade reaches back into the one before it, as a part of
/// that one's far bound; the shader blends across it (as Bevy's
/// `overlap_proportion`).
pub const CASCADE_OVERLAP: f32 = 0.2;

/// One shadow map's camera this frame.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ShadowView {
    /// Clip space of the shadow pass.
    pub view_proj: Mat4,
    /// World → the map's [0, 1]³ (texture coordinates, depth as stored).
    pub to_map: Mat4,
    /// The receiver's normal offset: world units, or per unit of depth
    /// for a perspective map (a spot's, a point light's faces).
    pub normal_bias: f32,
    pub state: LightShadow,
    /// An orthographic map's side in world units (0 for a spot's).
    pub extent: f32,
    /// Its side in texels (a power of two, at most a layer's): what the
    /// atlas lays out.
    pub size: u32,
    /// Where it is drawn: the layer, and the part of it (texture
    /// coordinates: x0, y0, x1, y1) — set when the maps are laid out.
    pub layer: usize,
    pub tile: [f32; 4],
}

impl ShadowView {
    /// Drawn into `tile` of `layer`: world → map now lands in the tile.
    pub fn place(&mut self, layer: usize, tile: [f32; 4]) {
        let [x0, y0, x1, y1] = tile;
        let into = Mat4::from_cols(
            Vec4::new(x1 - x0, 0.0, 0.0, 0.0),
            Vec4::new(0.0, y1 - y0, 0.0, 0.0),
            Vec4::Z,
            Vec4::new(x0, y0, 0.0, 1.0),
        );
        self.to_map = into * self.to_map;
        self.layer = layer;
        self.tile = tile;
    }
}

/// A point light's six faces (+X, −X, +Y, −Y, +Z, −Z), each a perspective
/// map a little wider than 90° — two texels to spare at its edges, so the
/// 3×3 filter of a point on a face's border stays on the face (Filament's
/// point shadows are six such maps too). `size`: a face's side in texels.
/// Its depth reaches the farthest caster (or the light's range). None when
/// no caster is around it.
pub(crate) fn point_views(
    light: &Light,
    world: &Mat4,
    bounds: (Vec3, Vec3),
    size: u32,
    reverse_z: bool,
    settings: LightShadow,
) -> Option<Vec<ShadowView>> {
    let (min, max) = bounds;
    if !(min.is_finite() && max.is_finite()) || min.cmpgt(max).any() {
        return None;
    }
    let eye = world.w_axis.truncate();
    let mut far = box_corners(bounds)
        .iter()
        .map(|c| c.distance(eye))
        .fold(0.0, f32::max);
    if far <= 0.0 {
        return None;
    }
    if light.range > 0.0 {
        far = far.min(light.range);
    }
    far *= 1.01;
    let near = far * 2e-3;
    let texels = size.max(8) as f32;
    // tan(fov / 2) with two texels to spare each side.
    let half_tan = texels / (texels - 4.0);
    let fov = 2.0 * half_tan.atan();
    let proj = if reverse_z {
        Mat4::perspective_rh(fov, 1.0, far, near)
    } else {
        Mat4::perspective_rh_gl(fov, 1.0, near, far)
    };
    // A texel's width one unit of depth away.
    let bias = settings.normal_bias * 2.0 * half_tan / texels;
    let faces = [
        (Vec3::X, Vec3::Y),
        (Vec3::NEG_X, Vec3::Y),
        (Vec3::Y, Vec3::Z),
        (Vec3::NEG_Y, Vec3::Z),
        (Vec3::Z, Vec3::Y),
        (Vec3::NEG_Z, Vec3::Y),
    ];
    Some(
        faces
            .iter()
            .map(|&(dir, up)| {
                let view = Mat4::look_at_rh(eye, eye + dir, up);
                let mut v = finish(proj * view, bias, settings, 0.0, reverse_z);
                v.size = size;
                v
            })
            .collect(),
    )
}

/// The camera the cascades split: its world matrix and projection.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ViewCamera {
    pub world: Mat4,
    pub camera: Camera,
}

/// A directional light's maps this frame: the cascades nearest first, and
/// how far along the view (camera-space depth) each reaches. The last one
/// reaches everything beyond (`f32::MAX`).
#[derive(Debug, Clone, Default)]
pub(crate) struct CascadeViews {
    pub views: Vec<ShadowView>,
    pub far_bounds: Vec<f32>,
}

/// `light`'s shadow camera (its world matrix `world`), fitted around the
/// casters' world box `bounds`, for a `size`² map. None when nothing can
/// be in its shadow (no casters in front of a spot, a point light).
pub(crate) fn shadow_view(
    light: &Light,
    world: &Mat4,
    bounds: (Vec3, Vec3),
    size: u32,
    reverse_z: bool,
    settings: LightShadow,
) -> Option<ShadowView> {
    let (min, max) = bounds;
    if !(min.is_finite() && max.is_finite()) || min.cmpgt(max).any() {
        return None;
    }
    let corners = box_corners(bounds);
    let dir = light_direction(world);
    let up = if dir.y.abs() > 0.99 { Vec3::X } else { Vec3::Y };
    let texels = size.max(1) as f32;
    match light.kind {
        LightKind::Directional => {
            let center = (min + max) * 0.5;
            let radius = ((max - min).length() * 0.5).max(1e-4);
            let eye = center - dir * radius * 2.0;
            let view = Mat4::look_at_rh(eye, center, up);
            let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
            for c in corners {
                let p = view.transform_point3(c);
                lo = lo.min(p);
                hi = hi.max(p);
            }
            // A margin so the outermost texels still filter inside.
            let pad = (hi - lo).max_element() * 0.02 + 1e-4;
            let (l, r, b, t) = (lo.x - pad, hi.x + pad, lo.y - pad, hi.y + pad);
            let (near, far) = ((-hi.z - pad).max(1e-4), -lo.z + pad);
            let proj = ortho(l, r, b, t, near, far, reverse_z);
            let extent = (r - l).max(t - b);
            let bias = settings.normal_bias * extent / texels;
            Some(finish(proj * view, bias, settings, extent, reverse_z))
        }
        LightKind::Spot { outer_cone, .. } => {
            let eye = world.w_axis.truncate();
            let view = Mat4::look_at_rh(eye, eye + dir, up);
            let depths = corners.map(|c| -view.transform_point3(c).z);
            let mut far = depths.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            if far <= 0.0 {
                return None; // every caster behind the light
            }
            if light.range > 0.0 {
                far = far.min(light.range);
            }
            far *= 1.01;
            let nearest = depths.iter().copied().fold(f32::INFINITY, f32::min);
            let near = (nearest * 0.99).max(far * 1e-3);
            let fov = (2.0 * outer_cone + 0.02).min(std::f32::consts::PI - 0.05);
            let proj = if reverse_z {
                Mat4::perspective_rh(fov, 1.0, far, near)
            } else {
                Mat4::perspective_rh_gl(fov, 1.0, near, far)
            };
            // A texel's width one unit of depth away.
            let texel = 2.0 * (fov * 0.5).tan() / texels;
            let bias = settings.normal_bias * texel;
            Some(finish(proj * view, bias, settings, 0.0, reverse_z))
        }
        LightKind::Point => None,
    }
}

/// A directional light's cascades for the view of `camera`: the part of
/// the view the casters' box `bounds` spans (up to `max_distance` when
/// set) split by the practical scheme (Zhang et al. 2006, PSSM: λ between
/// logarithmic and uniform). Each slice fitted as Bevy's
/// `calculate_cascade` (MJP's stable cascades): a square as wide as the
/// slice's largest diagonal — the same however the camera turns — moved in
/// whole texels, so its texels don't swim. Its depth reaches back to the
/// casters toward the light (WebGL2 has no depth clamp).
pub(crate) fn cascade_views(
    light_world: &Mat4,
    camera: &ViewCamera,
    bounds: (Vec3, Vec3),
    size: u32,
    reverse_z: bool,
    settings: LightShadow,
) -> CascadeViews {
    let light = Light::directional(Vec3::ONE, 1.0);
    let Some(fit) = shadow_view(&light, light_world, bounds, size, reverse_z, settings) else {
        return CascadeViews::default();
    };
    let whole = CascadeViews {
        views: vec![fit],
        far_bounds: vec![f32::MAX],
    };
    let count = (settings.cascades as usize).clamp(1, MAX_CASCADES);
    let Some((near, far)) = view_depth_range(camera, bounds, settings.max_distance) else {
        return whole;
    };
    if count == 1 {
        return whole;
    }

    // The light's rotation alone (orthonormal, no translation): what
    // keeps the cascades steady (Bevy's world_from_light).
    let dir = light_direction(light_world);
    let up = if dir.y.abs() > 0.99 { Vec3::X } else { Vec3::Y };
    let z = -dir;
    let x = up.cross(z).normalize();
    let y = z.cross(x);
    let light_from_world =
        Mat4::from_cols(x.extend(0.0), y.extend(0.0), z.extend(0.0), Vec4::W).transpose();
    // The casters' depth range along the light (−z is away from it).
    let (mut cz_lo, mut cz_hi) = (f32::INFINITY, f32::NEG_INFINITY);
    for c in box_corners(bounds) {
        let p = light_from_world.transform_point3(c);
        cz_lo = cz_lo.min(p.z);
        cz_hi = cz_hi.max(p.z);
    }
    let light_from_camera = light_from_world * camera.world;
    let texels = size.max(1) as f32;
    let far_bounds = split_depths(near, far, count, settings.split_lambda);

    let mut out = CascadeViews::default();
    let mut slice_near = near;
    for (i, &slice_far) in far_bounds.iter().enumerate() {
        let corners = slice_corners(&camera.camera, slice_near, slice_far);
        // Bevy's diameter: the larger of the slice's body and far-plane
        // diagonals — rounded up in eighths of its power of two (Bevy
        // rounds to whole units, which suits metres, not a ring).
        let body = (corners[0] - corners[6]).length();
        let far_plane = (corners[4] - corners[6]).length();
        let diameter = round_up(body.max(far_plane));
        if fit.extent <= diameter {
            // The casters' fit is as sharp: it covers the rest.
            out.views.push(fit);
            out.far_bounds.push(f32::MAX);
            return out;
        }
        let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        for c in corners {
            let p = light_from_camera.transform_point3(c);
            lo = lo.min(p);
            hi = hi.max(p);
        }
        // The slice's center moved to whole texels (Bevy's near_plane_center).
        let texel = diameter / texels;
        let cx = (0.5 * (lo.x + hi.x) / texel).floor() * texel;
        let cy = (0.5 * (lo.y + hi.y) / texel).floor() * texel;
        // Toward the light as far as the casters go; away from it only as
        // far as both the slice and the casters (nothing beyond casts).
        let z_near = hi.z.max(cz_hi);
        let z_far = lo.z.max(cz_lo);
        let pad = (z_near - z_far).abs() * 0.01 + texel;
        let (n, f) = (-(z_near + pad), -(z_far - pad));
        let h = diameter * 0.5;
        let view = Mat4::from_translation(Vec3::new(-cx, -cy, 0.0)) * light_from_world;
        let proj = ortho(-h, h, -h, h, n, f, reverse_z);
        let bias = settings.normal_bias * texel;
        out.views
            .push(finish(proj * view, bias, settings, diameter, reverse_z));
        let last = i + 1 == count;
        out.far_bounds.push(if last { f32::MAX } else { slice_far });
        // The next slice starts inside this one: the shader blends there.
        slice_near = slice_far * (1.0 - CASCADE_OVERLAP);
    }
    out
}

/// The ground shadow's camera: under the casters' box `bounds`, looking
/// up (+Y) over a square of their footprint padded for the blur, reaching
/// `reach` up (0: half their height, at least a twentieth of their
/// footprint) — as three.js' contact-shadows example's camera under its
/// plane. With the plane's world matrix: that square just under the box's
/// bottom (a unit square in XZ scaled to it).
pub(crate) fn ground_view(
    bounds: (Vec3, Vec3),
    reach: f32,
    size: u32,
    reverse_z: bool,
) -> Option<(ShadowView, Mat4)> {
    let (min, max) = bounds;
    if !(min.is_finite() && max.is_finite()) || min.cmpgt(max).any() {
        return None;
    }
    let height = max.y - min.y;
    let footprint = (max.x - min.x).max(max.z - min.z).max(1e-4);
    let reach = if reach > 0.0 {
        reach
    } else {
        (height * 0.5).max(footprint * 0.05)
    };
    // Room around the footprint for the blur to fade out in.
    let side = footprint * 1.5;
    let center = Vec3::new((min.x + max.x) * 0.5, min.y, (min.z + max.z) * 0.5);
    let view = Mat4::look_at_rh(center, center + Vec3::Y, Vec3::Z);
    let h = side * 0.5;
    let proj = ortho(-h, h, -h, h, 0.0, reach, reverse_z);
    let mut v = finish(proj * view, 0.0, LightShadow::default(), side, reverse_z);
    v.size = size;
    // The plane a little below the lowest point: a model's own floor there
    // hides it rather than fighting it for depth.
    let below = center - Vec3::Y * (footprint * 0.01);
    let plane = Mat4::from_translation(below) * Mat4::from_scale(Vec3::new(side, 1.0, side));
    Some((v, plane))
}

// The far bounds of `count` slices of [near, far]: the practical split,
// λ of the logarithmic one plus 1 − λ of the uniform.
fn split_depths(near: f32, far: f32, count: usize, lambda: f32) -> Vec<f32> {
    let lambda = lambda.clamp(0.0, 1.0);
    (1..=count)
        .map(|i| {
            let t = i as f32 / count as f32;
            let log = near * (far / near).powf(t);
            let uniform = near + (far - near) * t;
            lambda * log + (1.0 - lambda) * uniform
        })
        .collect()
}

// The depths along the view (positive, camera space) the casters' box
// spans, within the camera's near and far and `max_distance` (0: none).
fn view_depth_range(
    camera: &ViewCamera,
    bounds: (Vec3, Vec3),
    max_distance: f32,
) -> Option<(f32, f32)> {
    let view = camera.world.inverse();
    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
    for c in box_corners(bounds) {
        let d = -view.transform_point3(c).z;
        lo = lo.min(d);
        hi = hi.max(d);
    }
    let (cam_near, cam_far) = match camera.camera.projection {
        Projection::Perspective { near, far, .. } | Projection::Orthographic { near, far, .. } => {
            (near, far)
        }
    };
    let near = lo.max(cam_near);
    let mut far = hi.min(cam_far);
    if max_distance > 0.0 {
        far = far.min(max_distance);
    }
    (near.is_finite() && far > near * 1.001 && near > 0.0).then_some((near, far))
}

// The 8 corners (camera space) of the view between depths `near` and
// `far`: the near plane's four, then the far plane's, each as Bevy orders
// them (bottom right, top right, top left, bottom left).
fn slice_corners(camera: &Camera, near: f32, far: f32) -> [Vec3; 8] {
    let aspect = if camera.aspect > 0.0 {
        camera.aspect
    } else {
        1.0
    };
    let half = |d: f32| match camera.projection {
        Projection::Perspective { fov_y, .. } => d * (fov_y * 0.5).tan(),
        Projection::Orthographic { half_height, .. } => half_height,
    };
    let plane = |d: f32| {
        let h = half(d);
        let w = h * aspect;
        [
            Vec3::new(w, -h, -d),
            Vec3::new(w, h, -d),
            Vec3::new(-w, h, -d),
            Vec3::new(-w, -h, -d),
        ]
    };
    let (a, b) = (plane(near), plane(far));
    [a[0], a[1], a[2], a[3], b[0], b[1], b[2], b[3]]
}

// `d` rounded up to a multiple of an eighth of its power of two: changes
// only in steps, so a cascade's texels stay put while it holds.
fn round_up(d: f32) -> f32 {
    if d.is_nan() || d <= 0.0 || d.is_infinite() {
        return d.max(1e-4);
    }
    let step = 2f32.powi(d.log2().floor() as i32 - 3);
    (d / step).ceil() * step
}

fn light_direction(world: &Mat4) -> Vec3 {
    (-world.z_axis.truncate()).normalize_or(Vec3::NEG_Z)
}

fn box_corners((min, max): (Vec3, Vec3)) -> [Vec3; 8] {
    std::array::from_fn(|i| {
        Vec3::new(
            if i & 1 == 0 { min.x } else { max.x },
            if i & 2 == 0 { min.y } else { max.y },
            if i & 4 == 0 { min.z } else { max.z },
        )
    })
}

// An orthographic projection, GL's or reverse-Z's (zero-to-one, near → 1).
fn ortho(l: f32, r: f32, b: f32, t: f32, near: f32, far: f32, reverse_z: bool) -> Mat4 {
    if reverse_z {
        Mat4::orthographic_rh(l, r, b, t, far, near)
    } else {
        Mat4::orthographic_rh_gl(l, r, b, t, near, far)
    }
}

// The view with its world → map matrix: clip → texture coordinates and
// stored depth (x, y from [−1, 1]; z from [−1, 1] too, or as it is under
// reverse-Z: zero-to-one).
fn finish(
    view_proj: Mat4,
    normal_bias: f32,
    state: LightShadow,
    extent: f32,
    reverse_z: bool,
) -> ShadowView {
    let z = if reverse_z { (1.0, 0.0) } else { (0.5, 0.5) };
    let window = Mat4::from_cols(
        Vec4::new(0.5, 0.0, 0.0, 0.0),
        Vec4::new(0.0, 0.5, 0.0, 0.0),
        Vec4::new(0.0, 0.0, z.0, 0.0),
        Vec4::new(0.5, 0.5, z.1, 1.0),
    );
    ShadowView {
        view_proj,
        to_map: window * view_proj,
        normal_bias,
        state,
        extent,
        size: 0,
        layer: 0,
        tile: [0.0, 0.0, 1.0, 1.0],
    }
}

// The world box around every visible shadow caster: its bounds, or for a
// skinned mesh (its rest bounds don't hold) its joints' box, padded by a
// third of its size for the skin around them.
pub(crate) fn caster_bounds(world: &World) -> Option<(Vec3, Vec3)> {
    let casters = world.storage::<ShadowCaster>()?;
    let worlds = world.storage::<WorldMatrix>()?;
    let bounds = world.storage::<Bounds>();
    let skins = world.storage::<Skin>();
    let any_hidden = crate::render::any_hidden(world);
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    for (e, _) in casters.iter() {
        if any_hidden && hidden(world, e) {
            continue;
        }
        let Some(m) = worlds.get(e) else {
            continue;
        };
        if let Some(b) = bounds.and_then(|b| b.get(e)) {
            let (c, h) = b.transformed(&m.0);
            lo = lo.min(c - h);
            hi = hi.max(c + h);
        } else if let Some(skin) = skins.and_then(|s| s.get(e)) {
            let mut a = Vec3::splat(f32::INFINITY);
            let mut b = Vec3::splat(f32::NEG_INFINITY);
            for &j in skin.joints.iter() {
                if let Some(w) = worlds.get(j) {
                    let p = w.0.w_axis.truncate();
                    a = a.min(p);
                    b = b.max(p);
                }
            }
            if a.x <= b.x {
                let pad = Vec3::splat((b - a).max_element() / 3.0);
                lo = lo.min(a - pad);
                hi = hi.max(b + pad);
            }
        } else {
            let p = m.0.w_axis.truncate();
            lo = lo.min(p);
            hi = hi.max(p);
        }
    }
    (lo.x <= hi.x).then_some((lo, hi))
}

/// The key light: the visible light giving the most light at the middle of
/// the shadow casters (the scene's centre) — a directional light its
/// intensity (lux), a point or spot light its intensity over the squared
/// distance (candela → lux), nothing beyond its range or outside its cone.
/// Contact shadows are of it, and the shaders report its share of each
/// pixel. Its entity, and its world position (w 1) or the direction toward
/// it (w 0).
pub(crate) fn key_light(world: &World) -> Option<(rswebgl_ecs::Entity, Vec4)> {
    let lights = world.storage::<Light>()?;
    let worlds = world.storage::<WorldMatrix>()?;
    let any_hidden = crate::render::any_hidden(world);
    let centre = caster_bounds(world).map_or(Vec3::ZERO, |(lo, hi)| (lo + hi) * 0.5);
    let mut best: Option<(f32, rswebgl_ecs::Entity, Vec4)> = None;
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

/// Where the ground shadow lies: the casters' box grown by every visible
/// mesh with bounds — glass and other meshes that cast no shadow stand on
/// the ground too (a lamp's bulb below its shade), so the ground is under
/// them all; only the casters darken it.
pub(crate) fn ground_bounds(world: &World) -> Option<(Vec3, Vec3)> {
    let mut out = caster_bounds(world);
    let (Some(meshes), Some(bounds), Some(worlds)) = (
        world.storage::<crate::render::MeshRenderer>(),
        world.storage::<Bounds>(),
        world.storage::<WorldMatrix>(),
    ) else {
        return out;
    };
    let any_hidden = crate::render::any_hidden(world);
    for (e, _) in meshes.iter() {
        let (Some(b), Some(m)) = (bounds.get(e), worlds.get(e)) else {
            continue;
        };
        if any_hidden && hidden(world, e) {
            continue;
        }
        let (c, h) = b.transformed(&m.0);
        out = Some(out.map_or((c - h, c + h), |(lo, hi)| (lo.min(c - h), hi.max(c + h))));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_light_gives_the_most_light_at_the_middle() {

        let mut world = World::new();
        let caster = world.spawn();
        world.insert(caster, ShadowCaster { program: rswebgl::ids::ProgramId::NULL });
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
        world.insert(bulb, WorldMatrix(Mat4::from_translation(Vec3::new(10.0, 0.0, 0.0))));
        assert_eq!(key_light(&world), Some((sun, Vec4::new(0.0, 0.0, 1.0, 0.0))));
        // Out of its range: nothing from it.
        world.insert(bulb, Light::point(Vec3::ONE, 1e9, 1.0));
        assert_eq!(key_light(&world).map(|(e, _)| e), Some(sun));

    }

    // A sun from above and a little to the side.
    fn sun() -> Mat4 {
        Mat4::look_at_rh(Vec3::new(1.0, 4.0, 0.5), Vec3::ZERO, Vec3::Y).inverse()
    }

    // A camera at `eye` looking at `at` (60° fov, 16:9).
    fn camera(eye: Vec3, at: Vec3) -> ViewCamera {
        ViewCamera {
            world: Mat4::look_at_rh(eye, at, Vec3::Y).inverse(),
            camera: Camera::perspective(1.05, 16.0 / 9.0, 0.1, 1000.0),
        }
    }

    fn settings(cascades: u32) -> LightShadow {
        LightShadow {
            cascades,
            ..LightShadow::default()
        }
    }

    #[test]
    fn splits_grow_to_the_far_end() {
        let s = split_depths(1.0, 100.0, 3, 0.5);
        assert_eq!(s.len(), 3);
        assert!(s[0] < s[1] && s[1] < s[2]);
        assert!((s[2] - 100.0).abs() < 1e-3);
        // Between the uniform split (34) and the logarithmic one (4.6).
        assert!(s[0] > 4.6 && s[0] < 34.0, "{s:?}");
    }

    #[test]
    fn a_model_in_view_keeps_one_map_fitted_to_it() {
        let model = (Vec3::splat(-0.5), Vec3::splat(0.5));
        let cam = camera(Vec3::new(0.0, 0.5, 3.0), Vec3::ZERO);
        let c = cascade_views(&sun(), &cam, model, 2048, false, settings(3));
        assert_eq!(c.views.len(), 1, "{:?}", c.far_bounds);
        assert_eq!(c.far_bounds, vec![f32::MAX]);
    }

    #[test]
    fn a_large_scene_splits_into_cascades_nearest_smallest() {
        let ground = (
            Vec3::new(-200.0, -1.0, -200.0),
            Vec3::new(200.0, 5.0, 200.0),
        );
        let cam = camera(Vec3::new(0.0, 2.0, 0.0), Vec3::new(0.0, 1.0, -10.0));
        let c = cascade_views(&sun(), &cam, ground, 2048, false, settings(3));
        assert_eq!(c.views.len(), 3);
        let e: Vec<f32> = c.views.iter().map(|v| v.extent).collect();
        assert!(e[0] < e[1] && e[1] <= e[2], "{e:?}");
        assert!(c.far_bounds[0] < c.far_bounds[1]);
        assert_eq!(c.far_bounds[2], f32::MAX);
        // A point just ahead of the eye on the ground is inside the first.
        let p = c.views[0].to_map.project_point3(Vec3::new(0.0, 0.0, -1.0));
        assert!(p.cmpge(Vec3::ZERO).all() && p.cmple(Vec3::ONE).all(), "{p}");
        // One cascade only: the casters' fit.
        let one = cascade_views(&sun(), &cam, ground, 2048, false, settings(1));
        assert_eq!(one.views.len(), 1);
    }

    // Moving the camera a little moves a cascade's texels by whole texels
    // (or not at all): its shadows don't swim.
    #[test]
    fn cascades_move_in_whole_texels() {
        let ground = (
            Vec3::new(-200.0, -1.0, -200.0),
            Vec3::new(200.0, 5.0, 200.0),
        );
        let a = camera(Vec3::new(0.0, 2.0, 0.0), Vec3::new(0.0, 1.0, -10.0));
        let step = Vec3::new(0.013, 0.0, 0.007);
        let b = camera(
            Vec3::new(0.0, 2.0, 0.0) + step,
            Vec3::new(0.0, 1.0, -10.0) + step,
        );
        let ca = cascade_views(&sun(), &a, ground, 2048, false, settings(3));
        let cb = cascade_views(&sun(), &b, ground, 2048, false, settings(3));
        assert_eq!(ca.views[0].extent, cb.views[0].extent);
        let p = Vec3::new(3.0, 0.0, -7.0);
        let (pa, pb) = (
            ca.views[0].to_map.project_point3(p),
            cb.views[0].to_map.project_point3(p),
        );
        let d = (pb - pa).truncate() * 2048.0;
        assert!(
            (d - d.round()).abs().max_element() < 1e-2,
            "moved {d} texels"
        );
    }

    // Every direction from a point light lands on the face its largest
    // axis picks (as the shader does), inside the face's quarter with the
    // filter's two texels to spare — no seams between faces.
    #[test]
    fn point_faces_cover_every_direction_with_room_to_filter() {
        let light = Light::point(Vec3::ONE, 1.0, 0.0);
        let eye = Vec3::new(0.3, 1.0, -0.2);
        let world = Mat4::from_translation(eye);
        let bounds = (Vec3::splat(-5.0), Vec3::splat(5.0));
        let map = 1024;
        let mut views = point_views(
            &light,
            &world,
            bounds,
            map / 2,
            false,
            LightShadow::default(),
        )
        .unwrap();
        let tiles = [
            [0.0, 0.0, 0.5, 0.5],
            [0.5, 0.0, 1.0, 0.5],
            [0.0, 0.5, 0.5, 1.0],
            [0.5, 0.5, 1.0, 1.0],
            [0.0, 0.0, 0.5, 0.5],
            [0.5, 0.0, 1.0, 0.5],
        ];
        for (v, t) in views.iter_mut().zip(tiles) {
            v.place(0, t);
        }
        let spare = 2.0 / map as f32;
        let mut worst = f32::INFINITY;
        for i in 0..2000 {
            // Directions spread over the sphere (a golden spiral).
            let z = 1.0 - 2.0 * (i as f32 + 0.5) / 2000.0;
            let r = (1.0 - z * z).sqrt();
            let phi = i as f32 * 2.399_963;
            let d = Vec3::new(r * phi.cos(), r * phi.sin(), z);
            let a = d.abs();
            let face = if a.x >= a.y && a.x >= a.z {
                if d.x > 0.0 { 0 } else { 1 }
            } else if a.y >= a.z {
                if d.y > 0.0 { 2 } else { 3 }
            } else if d.z > 0.0 {
                4
            } else {
                5
            };
            let v = &views[face];
            let p = v.to_map.project_point3(eye + d * 2.0);
            let [x0, y0, x1, y1] = v.tile;
            let room = (p.x - x0).min(x1 - p.x).min(p.y - y0).min(y1 - p.y);
            worst = worst.min(room);
            assert!(p.z > 0.0 && p.z < 1.0, "{d}: depth {}", p.z);
        }
        assert!(
            worst >= spare * 0.99,
            "closest to a tile's edge: {} texels",
            worst * map as f32
        );
    }

    // The ground view looks up from the box's bottom: a point on the
    // ground under the box's center maps to the map's center at depth 0
    // (the ground), one at the reach's top to depth 1.
    #[test]
    fn the_ground_view_looks_up_from_under_the_casters() {
        let b = (Vec3::new(-1.0, 2.0, -0.5), Vec3::new(1.0, 4.0, 0.5));
        let (v, plane) = ground_view(b, 0.0, 512, false).unwrap();
        let p = v.to_map.project_point3(Vec3::new(0.0, 2.0, 0.0));
        assert!(
            (p - Vec3::new(0.5, 0.5, 0.0)).abs().max_element() < 1e-4,
            "{p}"
        );
        let top = v.to_map.project_point3(Vec3::new(0.0, 3.0, 0.0));
        assert!((top.z - 1.0).abs() < 1e-4, "reach: half the height: {top}");
        // The plane: 1.5 × the footprint, a hundredth of it under the bottom.
        let corner = plane.transform_point3(Vec3::new(0.5, 0.0, 0.5));
        assert!(
            (corner - Vec3::new(1.5, 1.98, 1.5)).abs().max_element() < 1e-5,
            "{corner}"
        );
        // Its corners land at the map's corners.
        let c = v
            .to_map
            .project_point3(plane.transform_point3(Vec3::new(-0.5, 0.0, -0.5)));
        assert!(
            c.x.min(1.0 - c.x).abs() < 1e-4 && c.y.min(1.0 - c.y).abs() < 1e-4,
            "{c}"
        );
    }
}
