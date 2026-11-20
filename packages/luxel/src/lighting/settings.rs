//! What the app sets: per light (`LightShadow`), per mesh (`ShadowCaster`)
//! and for the whole scene (`ShadowOptions`).

use luxel_gl::ids::ProgramId;

/// Makes the light on the entity (directional, spot or point — a point
/// light's six faces take six of the `MAX_SHADOW_VIEWS`) cast shadows.
/// Biases as Filament's `ShadowOptions` (their defaults): the casters'
/// depth pushed back by `slope_bias` × its slope + `constant_bias` depth
/// units (polygon offset), the receiver moved along its normal by
/// `normal_bias` shadow-map texels. Raise them for shadow acne (stripes on
/// lit surfaces), lower them for peter-panning (shadows coming loose).
///
/// A directional light's map splits into up to `cascades` along the
/// view where the view sees more than a model (1–4; see `views`):
/// `split_lambda` between uniform slices (0) and logarithmic ones (1,
/// more of the map near the eye); `max_distance` (> 0) ends shadows that
/// far along the view.
///
/// `map_size` (> 0): the side of this light's maps in texels (a point
/// light's faces; a power of two, at most the scene's `ShadowOptions`
/// size), laid out with the others' in the atlas; 0, the default: the
/// scene's size (half of it for a point light's faces).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightShadow {
    pub normal_bias: f32,
    pub slope_bias: f32,
    pub constant_bias: f32,
    pub cascades: u32,
    pub split_lambda: f32,
    pub max_distance: f32,
    pub map_size: u32,
}

impl Default for LightShadow {
    fn default() -> Self {
        Self {
            normal_bias: 1.0,
            slope_bias: 2.0,
            constant_bias: 0.5,
            cascades: 3,
            split_lambda: 0.5,
            max_distance: 0.0,
            map_size: 0,
        }
    }
}

/// Draws the entity's mesh into shadow maps with `program`: a depth-only
/// shader of the same vertex stage (the PBR's `DEPTH_ONLY` variant for
/// models; the scene sets it). No component: the mesh casts no shadow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShadowCaster {
    pub program: ProgramId,
}

/// The shadow maps' storage, the same for every light.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ShadowOptions {
    /// Side of each map in texels: a power of two in 256..=8192.
    pub map_size: u32,
    /// 16-bit depth instead of 24: half the memory (a 2048 map 8 MB
    /// instead of 16), coarser steps — enough for a model or a room, acne
    /// sooner over long distances.
    pub depth16: bool,
}

impl Default for ShadowOptions {
    fn default() -> Self {
        Self {
            map_size: 2048,
            depth16: false,
        }
    }
}

impl LightShadow {
    /// The side of this light's maps in a `layer`-sided atlas: its own
    /// size, else `default`; a power of two in 64..=layer.
    pub fn side(&self, default: u32, layer: u32) -> u32 {
        let want = if self.map_size > 0 {
            self.map_size
        } else {
            default
        };
        want.clamp(64, layer.max(64))
            .next_power_of_two()
            .min(layer.max(64))
    }
}

impl ShadowOptions {
    /// `size` rounded to a power of two in 256..=8192, within `max_texture`.
    pub fn clamp_size(size: u32, max_texture: u32) -> u32 {
        size.clamp(256, 8192)
            .next_power_of_two()
            .min(max_texture.max(256))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_round_to_powers_of_two_within_the_device() {
        assert_eq!(ShadowOptions::clamp_size(1000, 16384), 1024);
        assert_eq!(ShadowOptions::clamp_size(10, 16384), 256);
        assert_eq!(ShadowOptions::clamp_size(100_000, 16384), 8192);
        assert_eq!(ShadowOptions::clamp_size(8192, 4096), 4096);
    }
}
