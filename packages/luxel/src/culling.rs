use glam::{Mat4, Vec3, Vec4, Vec4Swizzles};

/// The entity's local axis-aligned bounding box, in the space of its
/// `Transform` (before the world matrix). An entity without one is never
/// culled.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub min: Vec3,
    pub max: Vec3,
}

impl Bounds {
    pub fn new(min: Vec3, max: Vec3) -> Self {
        Self { min, max }
    }

    /// The smallest box around `points` (x, y, z triples), or None for none.
    pub fn from_points(points: &[f32]) -> Option<Self> {
        let mut it = points.as_chunks::<3>().0.iter().map(|&p| Vec3::from(p));
        let first = it.next()?;
        Some(it.fold(Self::new(first, first), |b, p| {
            Self::new(b.min.min(p), b.max.max(p))
        }))
    }

    /// Center and half-size in world space after `m`: the box around the
    /// transformed box (conservative under rotation).
    pub fn transformed(&self, m: &Mat4) -> (Vec3, Vec3) {
        let center = (self.min + self.max) * 0.5;
        // A min/max given the wrong way round is the same box.
        let half = ((self.max - self.min) * 0.5).abs();
        let c = m.transform_point3(center);
        let h = m.x_axis.xyz().abs() * half.x
            + m.y_axis.xyz().abs() * half.y
            + m.z_axis.xyz().abs() * half.z;
        (c, h)
    }
}

/// The six planes of a view-projection's clip volume, normals pointing in.
#[derive(Debug, Clone, Copy)]
pub struct Frustum {
    planes: [Vec4; 6],
}

impl Frustum {
    /// From a GL-style view-projection (clip z in `[-w, w]`).
    pub fn from_view_proj(m: &Mat4) -> Self {
        Self::from_view_proj_depth(m, false)
    }

    /// From a view-projection whose clip z is in `[0, w]` when
    /// `zero_to_one` (reverse-Z, `EXT_clip_control`), `[-w, w]` otherwise.
    pub fn from_view_proj_depth(m: &Mat4, zero_to_one: bool) -> Self {
        let r = [m.row(0), m.row(1), m.row(2), m.row(3)];
        let low_z = if zero_to_one { r[2] } else { r[3] + r[2] };
        let planes = [
            r[3] + r[0],
            r[3] - r[0],
            r[3] + r[1],
            r[3] - r[1],
            low_z,
            r[3] - r[2],
        ]
        .map(|p| {
            let len = p.xyz().length();
            if len > 0.0 { p / len } else { p }
        });
        Self { planes }
    }

    /// Whether a box (world center and half-size) is at least partly inside.
    /// Conservative: a box near a corner can pass while outside.
    /// A box with an infinite or NaN side is never culled.
    pub fn intersects(&self, center: Vec3, half: Vec3) -> bool {
        if !(center.is_finite() && half.is_finite()) {
            return true;
        }
        self.planes.iter().all(|p| {
            let n = p.xyz();
            n.dot(center) + p.w >= -n.abs().dot(half)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frustum() -> Frustum {
        let proj = Mat4::perspective_rh_gl(1.0, 1.0, 0.1, 100.0);
        let view = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y);
        Frustum::from_view_proj(&(proj * view))
    }

    #[test]
    fn inverted_bounds_are_the_same_box() {
        let b = Bounds::new(Vec3::splat(1.0), Vec3::splat(-1.0));
        let (c, h) = b.transformed(&Mat4::IDENTITY);
        assert_eq!(h, Vec3::ONE);
        assert!(frustum().intersects(c, h));
    }

    #[test]
    fn infinite_bounds_are_never_culled() {
        let b = Bounds::new(Vec3::splat(f32::NEG_INFINITY), Vec3::splat(f32::INFINITY));
        let (c, h) = b.transformed(&Mat4::IDENTITY);
        assert!(frustum().intersects(c, h));
        let huge = Bounds::new(Vec3::splat(-f32::MAX), Vec3::splat(f32::MAX));
        let (c, h) = huge.transformed(&Mat4::from_scale(Vec3::splat(10.0)));
        assert!(frustum().intersects(c, h));
    }

    #[test]
    fn a_box_behind_the_camera_is_culled() {
        let b = Bounds::new(Vec3::new(-1.0, -1.0, 9.0), Vec3::new(1.0, 1.0, 11.0));
        let (c, h) = b.transformed(&Mat4::IDENTITY);
        assert!(!frustum().intersects(c, h));
    }
}
