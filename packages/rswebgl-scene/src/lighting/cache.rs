//! Which shadow maps need drawing again: a map holds what its camera saw
//! of its casters, so while neither changed it is still right. Each frame
//! gives every map a signature of what it would draw — its camera, its
//! light's biases, and each caster's mesh, material and when it last moved
//! or deformed (its `WorldMatrix`, its skin's joints, its morph weights) —
//! and a map whose signature matches the one it was drawn with is kept.
//! What no signature sees (a material's values or textures edited in
//! place) bumps the epoch every signature includes (`invalidate`).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::mem::discriminant;

use rswebgl_ecs::World;

use super::casters::CasterDraw;
use super::views::ShadowView;
use crate::animation::MorphWeights;
use crate::model::{Morph, Skin};
use crate::transform::{Parent, WorldMatrix};

/// The signature of each of `views` this frame, given the `draws` into
/// them (grouped by view, as `casters::cull` makes them).
pub(crate) fn signatures(
    world: &World,
    views: &[ShadowView],
    draws: &[CasterDraw],
    epoch: u64,
) -> Vec<u64> {
    let worlds = world.storage::<WorldMatrix>();
    let skins = world.storage::<Skin>();
    let morphs = world.storage::<MorphWeights>();
    let mut hashers: Vec<DefaultHasher> = views
        .iter()
        .map(|v| {
            let mut h = DefaultHasher::new();
            epoch.hash(&mut h);
            for f in v.to_map.to_cols_array() {
                f.to_bits().hash(&mut h);
            }
            let s = v.state;
            for f in [s.normal_bias, s.slope_bias, s.constant_bias] {
                f.to_bits().hash(&mut h);
            }
            h
        })
        .collect();
    for d in draws {
        let Some(h) = hashers.get_mut(d.view) else {
            continue;
        };
        let e = d.entity;
        e.hash(h);
        d.program.hash(h);
        let m = &d.mesh;
        m.vao.hash(h);
        m.material.hash(h);
        let c = &m.command;
        discriminant(&c.mode).hash(h);
        discriminant(&c.index_type).hash(h);
        (c.count, c.first, c.offset, c.instance_count).hash(h);
        (c.range_start, c.range_end).hash(h);
        if let Some(w) = worlds {
            w.changed_tick(d.row).hash(h);
        }
        // A skinned mesh moves with its joints; a morphed one with its
        // weights.
        if let (Some(skins), Some(w)) = (skins, worlds)
            && let Some(skin) = skins.get(e)
        {
            let newest = skin
                .joints
                .iter()
                .filter_map(|&j| w.row(j).map(|r| w.changed_tick(r)))
                .max();
            newest.hash(h);
        }
        // A primitive's morph weights are on its node (its parent).
        if let Some(morphs) = morphs
            && world.get::<Morph>(e).is_some()
            && let Some(node) = world.get::<Parent>(e)
            && let Some(row) = morphs.row(node.0)
        {
            morphs.changed_tick(row).hash(h);
        }
    }
    hashers.into_iter().map(|h| h.finish()).collect()
}
