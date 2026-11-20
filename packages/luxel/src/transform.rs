use glam::{Mat3, Mat4, Quat, Vec3};
use luxel_ecs::Entity;

/// Local position, rotation and scale — relative to the [`Parent`] if the
/// entity has one, to the world otherwise.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Transform {
    pub const IDENTITY: Self = Self {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    pub fn from_translation(translation: Vec3) -> Self {
        Self {
            translation,
            ..Self::IDENTITY
        }
    }

    pub fn from_rotation(rotation: Quat) -> Self {
        Self {
            rotation,
            ..Self::IDENTITY
        }
    }

    pub fn from_scale(scale: Vec3) -> Self {
        Self {
            scale,
            ..Self::IDENTITY
        }
    }

    /// From a matrix with no shear (glTF node matrices are such). An axis
    /// scaled to zero (a hidden node) gets scale 0 and a rotation completed
    /// from the other axes, never NaN.
    pub fn from_matrix(m: Mat4) -> Self {
        let translation = m.w_axis.truncate();
        let cols = [
            m.x_axis.truncate(),
            m.y_axis.truncate(),
            m.z_axis.truncate(),
        ];
        let mut scale = Vec3::from_array(cols.map(|c| c.length()));
        if m.determinant() < 0.0 {
            scale.x = -scale.x;
        }
        let axis = |i: usize| (scale[i] != 0.0).then(|| cols[i] / scale[i]);
        let rotation = match (axis(0), axis(1), axis(2)) {
            (Some(x), Some(y), Some(z)) => Quat::from_mat3(&Mat3::from_cols(x, y, z)),
            (Some(x), Some(y), None) => basis(x, y),
            (Some(x), None, Some(z)) => basis(x, z.cross(x)),
            (None, Some(y), Some(z)) => basis(y.cross(z), y),
            (Some(x), None, None) => basis(x, x.any_orthonormal_vector()),
            (None, Some(y), None) => basis(y.any_orthonormal_vector().cross(y), y),
            (None, None, Some(z)) => {
                let x = z.any_orthonormal_vector();
                basis(x, z.cross(x))
            }
            (None, None, None) => Quat::IDENTITY,
        };
        Self {
            translation,
            rotation,
            scale,
        }
    }

    /// Scale, then rotate, then translate.
    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }
}

// The rotation taking the X and Y axes to `x` and `y` (unit, at right angles).
fn basis(x: Vec3, y: Vec3) -> Quat {
    Quat::from_mat3(&Mat3::from_cols(x, y, x.cross(y)))
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// The entity whose transform this one's is relative to. Set it with
/// [`crate::set_parent`], which refuses cycles; a parent that is dead or has no
/// [`Transform`] makes the entity a root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parent(pub Entity);

/// The entity's world matrix: parent's world matrix × local transform.
/// [`crate::TransformSystem`] adds and writes it; changes are stamped with the
/// system's tick, so a renderer can pick up only the moved objects.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldMatrix(pub Mat4);

impl Default for WorldMatrix {
    fn default() -> Self {
        Self(Mat4::IDENTITY)
    }
}
