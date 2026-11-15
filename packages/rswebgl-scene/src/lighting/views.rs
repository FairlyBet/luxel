//! The shadow cameras: where each shadowing light looks from and what its
//! map covers. Math only (no GL): fitted to the casters' bounds (all of
//! them, not the view) — stable as the camera moves, sharp for a model,
//! coarse for a large scene (cascades are for later).

use glam::{Mat4, Vec3, Vec4};
use rswebgl_ecs::World;

use super::light::{Light, LightKind};
use super::settings::{LightShadow, ShadowCaster};
use crate::culling::Bounds;
use crate::model::Skin;
use crate::render::Visible;
use crate::render::hidden;
use crate::transform::WorldMatrix;

/// One shadowing light's camera this frame.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ShadowView {
    /// Clip space of the shadow pass.
    pub view_proj: Mat4,
    /// World → the map's [0, 1]³ (texture coordinates, depth as stored).
    pub to_map: Mat4,
    /// The receiver's normal offset: world units, or per unit of depth
    /// along the light (`perspective`).
    pub normal_bias: f32,
    pub perspective: bool,
    pub state: LightShadow,
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
    let corners: [Vec3; 8] = std::array::from_fn(|i| {
        Vec3::new(
            if i & 1 == 0 { min.x } else { max.x },
            if i & 2 == 0 { min.y } else { max.y },
            if i & 4 == 0 { min.z } else { max.z },
        )
    });
    let dir = (-world.z_axis.truncate()).normalize_or(Vec3::NEG_Z);
    let up = if dir.y.abs() > 0.99 { Vec3::X } else { Vec3::Y };
    let texels = size.max(1) as f32;
    let (view, proj, normal_bias, perspective) = match light.kind {
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
            let proj = if reverse_z {
                Mat4::orthographic_rh(l, r, b, t, far, near)
            } else {
                Mat4::orthographic_rh_gl(l, r, b, t, near, far)
            };
            let texel = (r - l).max(t - b) / texels;
            (view, proj, settings.normal_bias * texel, false)
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
            (view, proj, settings.normal_bias * texel, true)
        }
        LightKind::Point => return None,
    };
    let view_proj = proj * view;
    // Clip → texture coordinates and stored depth: x, y from [−1, 1];
    // z from [−1, 1] too, or as it is under reverse-Z (zero-to-one).
    let z = if reverse_z { (1.0, 0.0) } else { (0.5, 0.5) };
    let window = Mat4::from_cols(
        Vec4::new(0.5, 0.0, 0.0, 0.0),
        Vec4::new(0.0, 0.5, 0.0, 0.0),
        Vec4::new(0.0, 0.0, z.0, 0.0),
        Vec4::new(0.5, 0.5, z.1, 1.0),
    );
    Some(ShadowView {
        view_proj,
        to_map: window * view_proj,
        normal_bias,
        perspective,
        state: settings,
    })
}

// The world box around every visible shadow caster: its bounds, or for a
// skinned mesh (its rest bounds don't hold) its joints' box, padded by a
// third of its size for the skin around them.
pub(crate) fn caster_bounds(world: &World) -> Option<(Vec3, Vec3)> {
    let casters = world.storage::<ShadowCaster>()?;
    let worlds = world.storage::<WorldMatrix>()?;
    let bounds = world.storage::<Bounds>();
    let skins = world.storage::<Skin>();
    let any_hidden = world
        .storage::<Visible>()
        .is_some_and(|v| v.data().iter().any(|v| !v.0));
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
