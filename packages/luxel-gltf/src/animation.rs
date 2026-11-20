//! glTF animations as plain data: per channel, the target node and property,
//! the keyframe times and values, and how to interpolate between them
//! (glTF 2.0 §3.11 and Appendix C). Sampling and applying them is the
//! scene's (`luxel` `animation.rs`).

use gltf::accessor::Dimensions;
use gltf::animation::Property;

/// What a channel animates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnimationPath {
    /// The node's translation: 3 floats a value.
    Translation,
    /// Its rotation, a unit quaternion (x, y, z, w): 4 floats a value.
    Rotation,
    /// Its scale: 3 floats a value.
    Scale,
    /// The morph target weights of its mesh: one float per target a value.
    Weights,
}

/// Between keyframes (glTF 2.0 Appendix C).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interpolation {
    /// Each keyframe's value holds until the next.
    Step,
    /// Linear; spherical (slerp) for rotations.
    Linear,
    /// Cubic Hermite spline: each keyframe stores an in-tangent, the value
    /// and an out-tangent, in that order.
    CubicSpline,
}

/// One animated property of one node.
#[derive(Clone, Debug)]
pub struct AnimationChannel {
    /// glTF node index.
    pub node: u32,
    pub path: AnimationPath,
    pub interpolation: Interpolation,
    /// Keyframe times in seconds, increasing.
    pub times: Vec<f32>,
    /// The values, flattened: per keyframe `components` floats (three
    /// groups of them for a cubic spline: in-tangent, value, out-tangent).
    pub values: Vec<f32>,
    /// Floats per value: 3, 4, or the number of morph targets.
    pub components: usize,
}

/// A named set of channels played together.
#[derive(Clone, Debug)]
pub struct Animation {
    pub name: Option<String>,
    pub channels: Vec<AnimationChannel>,
    /// The last keyframe time over all channels, in seconds.
    pub duration: f32,
}

// Every animation of the document. A channel the file can't give values for
// (bad accessors) is left out; so is one with no keyframes.
pub(crate) fn read_animations(doc: &gltf::Document, buffers: &[Vec<u8>]) -> Vec<Animation> {
    doc.animations()
        .map(|anim| {
            let mut channels = Vec::new();
            for (c, channel) in anim.channels().enumerate() {
                let sampler = channel.sampler();
                let skip = |why: &str| {
                    crate::fetch::warn(format!(
                        "animation {} channel {c}: {why}; skipped",
                        anim.index()
                    ))
                };
                // Read with the crate's own accessor reader: gltf's iterators
                // panic on a stride smaller than the element and on output
                // types the spec forbids.
                let input = sampler.input();
                if input.dimensions() != Dimensions::Scalar {
                    skip("keyframe times aren't scalars");
                    continue;
                }
                let Some(times) = crate::accessor::read_floats(&input, buffers) else {
                    skip("keyframe times can't be read");
                    continue;
                };
                if !times.iter().all(|t| t.is_finite()) || times.windows(2).any(|w| w[0] > w[1]) {
                    skip("keyframe times aren't finite and increasing");
                    continue;
                }
                let interpolation = match sampler.interpolation() {
                    gltf::animation::Interpolation::Step => Interpolation::Step,
                    gltf::animation::Interpolation::Linear => Interpolation::Linear,
                    gltf::animation::Interpolation::CubicSpline => Interpolation::CubicSpline,
                };
                // Values per keyframe: a cubic spline's in-tangent, value and
                // out-tangent.
                let per_key = if interpolation == Interpolation::CubicSpline {
                    3
                } else {
                    1
                };
                let (path, dims) = match channel.target().property() {
                    Property::Translation => (AnimationPath::Translation, Dimensions::Vec3),
                    Property::Rotation => (AnimationPath::Rotation, Dimensions::Vec4),
                    Property::Scale => (AnimationPath::Scale, Dimensions::Vec3),
                    Property::MorphTargetWeights => (AnimationPath::Weights, Dimensions::Scalar),
                };
                let output = sampler.output();
                if output.dimensions() != dims {
                    skip("its values have the wrong type");
                    continue;
                }
                let Some(values) = crate::accessor::read_floats(&output, buffers) else {
                    skip("its values can't be read");
                    continue;
                };
                let components = match path {
                    AnimationPath::Translation | AnimationPath::Scale => 3,
                    AnimationPath::Rotation => 4,
                    AnimationPath::Weights if times.is_empty() => 0,
                    AnimationPath::Weights => values.len() / (times.len() * per_key),
                };
                if times.is_empty()
                    || components == 0
                    || values.len() != times.len() * per_key * components
                {
                    skip("no keyframes, or its values don't match its times");
                    continue;
                }
                channels.push(AnimationChannel {
                    node: channel.target().node().index() as u32,
                    path,
                    interpolation,
                    times,
                    values,
                    components,
                });
            }
            let duration = channels
                .iter()
                .filter_map(|c| c.times.last().copied())
                .fold(0.0f32, f32::max);
            Animation {
                name: anim.name().map(str::to_owned),
                channels,
                duration,
            }
        })
        .collect()
}
